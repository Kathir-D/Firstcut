//! Burst batching: the heart of the app.
//!
//! Task.md §5. Three layers, each testable on its own:
//!
//! - [`signals`] computes the per-pair evidence of §5.2 and the adaptive frame interval of §5.3.
//! - this module turns those into batches and applies the two-phase freeze rule.
//! - [`visual`] holds the perceptual hash that settles the ambiguous zone.
//!
//! Everything is deterministic: same input, same batches, always (todo.md §5.3 step 6). The only
//! unordered input is the `sigs` map, which is never iterated — only looked up.

pub mod eval;
pub mod fixture;
pub mod signals;
pub mod view;
pub mod visual;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::batch::signals::{Decision, FrameIntervals, PairSignals, Thresholds};
use crate::batch::view::{Photo, effective_time_ms};
use crate::order;
pub use eval::{BoundaryMetrics, GroundTruth, evaluate_boundaries, evaluate_names};
pub use fixture::{FileKind, PhotoMeta, RawFormat, TimeSource};
pub use visual::{VisualSig, distance, visual_sig};

/// Stable across runs: hash of the photo's path relative to the session folder
/// (docs/contracts/photo-meta.md). `core-meta` uses this same helper so ids never drift between the
/// scanner and the batcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PhotoId(pub u64);

/// Stable across re-runs: hash of the batch's first [`PhotoId`] (docs/contracts/batching.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BatchId(pub u64);

/// One burst, or one moment of action. The unit the user culls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub id: BatchId,
    /// 0-based position in the shoot.
    pub index: u32,
    /// In capture order.
    pub photo_ids: Vec<PhotoId>,
    /// True until visual signatures have settled this batch's boundaries.
    pub provisional: bool,
}

/// FNV-1a, 64-bit. Chosen because it is specified exactly, so every language and every re-run
/// produces the same id — a session's batch ids have to survive a reload.
#[must_use]
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Folds a 64-bit hash into the 63 bits SQLite can hold.
///
/// `store::db` keeps a `u64` id in an `i64` column by masking off the top bit, and masking is only
/// a round trip if the id never *uses* that bit. FNV-1a sets it for 1,404 of the 2,880 fixture
/// photos (48.8%), so an unmasked id would be written to the database under one number and read
/// back under another — and the rating attached to it would be unreachable. Folding here means
/// every id this crate mints is positive by construction, which is the invariant the store needs.
///
/// This is **not** injective and does not claim to be: `0x1` and `0x8000_0000_0000_0000` both fold
/// to `0`, because the top bit is OR-ed into bit 62 rather than replacing it. Collisions need two
/// hashes to differ in bits 0 and 63 together, which is a 1-in-2^63 chance for any given pair —
/// about 5e-13 expected collisions across a 2,880-frame shoot. [`fnv1a64`] itself is the same
/// situation one bit better, which is why a `u64` id has to be narrowed to reach a `u64` column
/// at all.
///
/// The alternative, storing ids as TEXT in a schema v2, would keep ids exactly 64 bits and cost
/// the index. That is a store decision, not a batcher one, and it is not taken here.
#[must_use]
pub fn fold_to_i64(hash: u64) -> u64 {
    let folded = (hash >> 1) | ((hash & 1) << 62);
    debug_assert_eq!(folded >> 63, 0, "a folded id must stay positive");
    folded
}

/// [`PhotoId`] for a path relative to the session folder.
#[must_use]
pub fn photo_id(rel_path: &str) -> PhotoId {
    PhotoId(fold_to_i64(fnv1a64(rel_path.as_bytes())))
}

/// [`BatchId`] derived from a batch's first photo.
#[must_use]
pub fn batch_id(first: PhotoId) -> BatchId {
    BatchId(first.0)
}

/// How the ambiguous zone is scored. Every weight is a tuning knob; see todo.md §5.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    /// How far into the ambiguous zone the Δt sits.
    pub dt: f32,
    /// Zoom change, in stops.
    pub focal: f32,
    /// Aperture/shutter change, in EV.
    pub exposure: f32,
    /// Shutter-count jump: frames deleted in camera. Supporting evidence only (§5.2).
    pub deleted: f32,
    /// Perceptual-hash distance. Only used when both frames have a signature.
    pub visual: f32,
}

/// Every threshold the batcher applies, in one place so `firstcut gaps` can sweep them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchParams {
    /// Frame interval assumed when a window has no short gaps at all (11 fps, the fastest in the
    /// test set).
    pub default_frame_interval_ms: i64,
    /// Δt ≤ this is always a join (todo.md §5.3 step 2).
    pub hard_join_floor_ms: i64,
    /// Join threshold is `max(hard_join_floor_ms, hard_join_factor * f)`.
    pub hard_join_factor: f32,
    /// Δt > this is always a split (todo.md §5.3 step 3).
    pub hard_split_floor_ms: i64,
    /// Split threshold is `max(hard_split_floor_ms, hard_split_factor * f)`, so a slow local rate
    /// can never make the hard split tighter than the hard join.
    pub hard_split_factor: f32,
    /// Split the ambiguous zone when the score exceeds this.
    pub split_threshold: f32,
    pub weights_meta: Weights,
    pub weights_visual: Weights,
    /// Consecutive one-photo batches closer together than this are merged (todo.md §5.3 step 5).
    /// 0 keeps them apart.
    pub single_group_window_ms: i64,
}

impl Default for BatchParams {
    fn default() -> Self {
        Self {
            default_frame_interval_ms: 90,
            hard_join_floor_ms: 250,
            hard_join_factor: 2.5,
            hard_split_floor_ms: 2_000,
            hard_split_factor: 4.0,
            split_threshold: 0.5,
            weights_meta: Weights {
                dt: 0.55,
                focal: 0.15,
                exposure: 0.15,
                deleted: 0.15,
                visual: 0.0,
            },
            weights_visual: Weights {
                dt: 0.32,
                focal: 0.09,
                exposure: 0.09,
                deleted: 0.05,
                visual: 0.45,
            },
            // Owner's decision, 2026-09-29 (todo.md §2): single frames a few seconds apart are one
            // batch, not a run of one-photo batches. 5 s is the "few seconds": Game1JENKS goes
            // from 99 one-photo batches to 75, and no batch anywhere grows (only singles merge).
            // The exact window is still to be confirmed against visually checked ground truth.
            single_group_window_ms: 5_000,
        }
    }
}

/// A zoom of one full stop counts as a complete "different moment".
pub const FOCAL_FULL_STOPS: f32 = 1.0;
/// Two stops of aperture/shutter/ISO counts as a complete "different moment".
pub const EXPOSURE_FULL_EV: f32 = 2.0;
/// Shutter-count jumps saturate here; §5.2 calls them supporting evidence, not a boundary.
pub const DELETED_FULL: i64 = 16;

/// Δt saturates to "completely a different moment" at this many local frame intervals.
///
/// The point of the ambiguous zone is that time alone cannot decide, so this is expressed in frame
/// intervals rather than milliseconds: a 450 ms gap at 11 fps (five intervals, the shutter was
/// released) is a boundary, and the same 450 ms at 6 fps (2.6 intervals, a dropped frame) is not.
/// Setting it from `f` is what makes one threshold work from 6 fps to 40 fps.
pub const DT_FULL_INTERVALS: f32 = 6.0;

/// What was decided at one boundary, with the evidence. The CLI prints these to tune thresholds.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryVerdict {
    /// Position in the ordered sequence of the photo that would start a new batch.
    pub index: usize,
    pub decision: Decision,
    /// 0.0..=1.0 for ambiguous boundaries; 0.0 otherwise.
    pub score: f32,
    pub signals: PairSignals,
    pub thresholds: Thresholds,
    /// True when a visual signature was available for both frames.
    pub had_sigs: bool,
    /// True when the boundary can still change once signatures arrive, which is exactly when it came
    /// out of the ambiguous zone. A hard join, a hard split and a boundary a visited batch has
    /// locked are all settled.
    pub provisional: bool,
    /// True when an ambiguous boundary's score was past the split threshold, so metadata alone
    /// started a new batch here. Only meaningful when `decision` is `Ambiguous`.
    pub split: bool,
}

/// Batches plus the trace that produced them.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchOutcome {
    pub batches: Vec<Batch>,
    /// One entry per position 1..n in the ordered sequence.
    pub verdicts: Vec<BoundaryVerdict>,
    pub order: Vec<PhotoId>,
    pub params: BatchParams,
}

/// Provisional batches from metadata alone (docs/contracts/batching.md).
///
/// `sigs` may be empty, in which case every batch comes back `provisional`. Batches in `frozen`
/// that the user has already visited come back unchanged; only unvisited ones may move.
#[must_use]
pub fn batch<P: Photo>(
    photos: &[P],
    sigs: &HashMap<PhotoId, VisualSig>,
    frozen: &[Batch],
) -> Vec<Batch> {
    batch_with(photos, sigs, frozen, BatchParams::default()).batches
}

/// [`batch`] with explicit parameters, returning the full trace.
#[must_use]
pub fn batch_with<P: Photo>(
    photos: &[P],
    sigs: &HashMap<PhotoId, VisualSig>,
    frozen: &[Batch],
    params: BatchParams,
) -> BatchOutcome {
    let indices = order::ordered_indices(photos);
    let seq: Vec<&P> = indices.iter().map(|&i| &photos[i]).collect();
    let n = seq.len();

    let gaps: Vec<i64> = (0..n)
        .map(|i| {
            if i == 0 {
                0
            } else {
                match (effective_time_ms(seq[i - 1]), effective_time_ms(seq[i])) {
                    (Some(a), Some(b)) => b.saturating_sub(a).max(0),
                    _ => 0,
                }
            }
        })
        .collect();
    let intervals = FrameIntervals::new(gaps, params.default_frame_interval_ms);
    // Every boundary's local frame interval, in one pass: `FrameIntervals::at` allocates and sorts a
    // sample per call, and this runs once per boundary of every shoot.
    let local = intervals.local_intervals(n);
    let locks = Locks::from_frozen(frozen, &seq);

    let mut verdicts = Vec::with_capacity(n);
    let mut splits: Vec<bool> = vec![false; n];
    for i in 1..n {
        let thresholds = thresholds_at_interval(local[i], &params);
        // Locks first. A boundary inside or beside a batch the user has already been in is settled
        // whatever the evidence says, so nothing is computed for it: a re-batch of a fully visited
        // shoot does no per-pair work at all. A locked verdict carries no signals for the same
        // reason — the freeze settled it, so there is no evidence to report.
        let (decision, score, split, signals) = if locks.forced_join[i] {
            // Inside a batch the user has already seen: nothing may split it.
            (Decision::Join, 0.0, false, PairSignals::default())
        } else if locks.forced_split[i] {
            // The edge of a visited batch: it cannot grow into its neighbour either.
            (Decision::Split, 0.0, false, PairSignals::default())
        } else {
            let signals = PairSignals::compute(seq[i - 1], seq[i]);
            match signals.decide(thresholds) {
                Decision::Join => (Decision::Join, 0.0, false, signals),
                Decision::Split => (Decision::Split, 0.0, false, signals),
                Decision::Ambiguous => {
                    let dist = match (sigs.get(&seq[i - 1].id()), sigs.get(&seq[i].id())) {
                        (Some(a), Some(b)) => Some(visual::distance(a, b)),
                        _ => None,
                    };
                    let score = score_pair(&signals, &thresholds, dist, &params);
                    (
                        Decision::Ambiguous,
                        score,
                        score > params.split_threshold,
                        signals,
                    )
                }
            }
        };
        splits[i] = decision == Decision::Split || split;
        verdicts.push(BoundaryVerdict {
            index: i,
            decision,
            score,
            signals,
            thresholds,
            // Whether both signatures exist is a fact about `sigs`, not about the decision, so a
            // boundary a lock settled still says so. It used to be reported as "no signature
            // available" for every locked pair, which is how a frozen shoot that had been refined
            // read as never refined.
            had_sigs: sigs.contains_key(&seq[i - 1].id()) && sigs.contains_key(&seq[i].id()),
            provisional: decision == Decision::Ambiguous,
            split,
        });
    }

    let mut batches = assemble(&seq, &splits, &verdicts);
    apply_single_grouping(&mut batches, &seq, &params);

    BatchOutcome {
        batches,
        verdicts,
        order: indices.iter().map(|&i| photos[i].id()).collect(),
        params,
    }
}

/// Adaptive thresholds for boundary `i` (todo.md §5.3 steps 2–3).
#[must_use]
pub fn thresholds_at(intervals: &FrameIntervals, params: &BatchParams, i: usize) -> Thresholds {
    thresholds_at_interval(intervals.at(i), params)
}

/// [`thresholds_at`] for a frame interval already in hand, which is how the batching pass reads
/// them: [`FrameIntervals::local_intervals`] has already computed every boundary's rate.
#[must_use]
pub fn thresholds_at_interval(f: i64, params: &BatchParams) -> Thresholds {
    let join_ms = (params.hard_join_floor_ms as f32).max(params.hard_join_factor * f as f32) as i64;
    let split_ms =
        (params.hard_split_floor_ms as f32).max(params.hard_split_factor * f as f32) as i64;
    Thresholds {
        frame_interval_ms: f,
        // A hard split must never land inside the hard join, or one gap would have to be both.
        join_ms: join_ms.min(split_ms),
        split_ms: split_ms.max(join_ms),
    }
}

/// Weighted sum of the normalized signals, todo.md §5.3 step 4.
///
/// Weights of *unavailable* evidence are left out of both the numerator and the denominator: a frame
/// with no focal length recorded must not be pushed toward "same burst" just because one weight
/// silently scored zero.
#[must_use]
pub fn score_pair(
    signals: &PairSignals,
    thresholds: &Thresholds,
    visual: Option<visual::VisualDistance>,
    params: &BatchParams,
) -> f32 {
    let weights = if visual.is_some() {
        params.weights_visual
    } else {
        params.weights_meta
    };

    // Δt normalized in *frame intervals*, not milliseconds: the same wall-clock gap means something
    // completely different at 6 fps and at 11 fps.
    let dt_n = if signals.has_scorable_time() {
        let f = thresholds.frame_interval_ms.max(1) as f32;
        let join_intervals = thresholds.join_ms as f32 / f;
        // The span is the ambiguous zone itself, in frame intervals — except where the constant
        // scale is still the wider of the two. `DT_FULL_INTERVALS - join_intervals` goes negative
        // once the hard join is more than six intervals wide, which is any rate from about 24 fps
        // up: there the old span fell to its 0.5 floor and every gap past the hard join scored a
        // certain split, in exactly the range the rule exists to cover. Below that rate the
        // constant is the narrower scale, and the thresholds it was calibrated against are the ones
        // in the corpus, so it stands.
        let constant_span = DT_FULL_INTERVALS - join_intervals;
        let zone_span = (thresholds.split_ms - thresholds.join_ms) as f32 / f;
        let span = if constant_span > 0.5 {
            constant_span
        } else {
            zone_span.max(0.5)
        };
        ((signals.dt_ms as f32 / f - join_intervals) / span).clamp(0.0, 1.0)
    } else {
        // No timing evidence at all: the middle of the zone, so the other signals decide.
        0.5
    };
    let focal_n = (signals.focal_stops / FOCAL_FULL_STOPS).clamp(0.0, 1.0);
    let exposure_n = (signals.exposure_ev / EXPOSURE_FULL_EV).clamp(0.0, 1.0);
    let deleted_n = match signals.shutter_count_gap {
        Some(gap) if gap > 1 => ((gap - 1) as f32 / DELETED_FULL as f32).clamp(0.0, 1.0),
        _ => 0.0,
    };

    let mut numerator = 0.0f32;
    let mut denominator = 0.0f32;
    if signals.has_scorable_time() {
        numerator += weights.dt * dt_n;
        denominator += weights.dt;
    }
    if signals.focal_stops > 0.0 {
        numerator += weights.focal * focal_n;
        denominator += weights.focal;
    }
    if signals.exposure_ev > 0.0 {
        numerator += weights.exposure * exposure_n;
        denominator += weights.exposure;
    }
    if deleted_n > 0.0 {
        numerator += weights.deleted * deleted_n;
        denominator += weights.deleted;
    }
    if let Some(d) = visual {
        numerator += weights.visual * d.combined.clamp(0.0, 1.0);
        denominator += weights.visual;
    }
    if denominator == 0.0 {
        // No evidence whatsoever: leave them together rather than invent a boundary.
        return 0.0;
    }
    numerator / denominator
}

/// Boundaries that the visited batches have already settled.
#[derive(Debug, Default)]
struct Locks {
    forced_split: Vec<bool>,
    forced_join: Vec<bool>,
}

impl Locks {
    fn from_frozen<P: Photo>(frozen: &[Batch], seq: &[&P]) -> Self {
        let n = seq.len();
        let mut forced_split = vec![false; n];
        let mut forced_join = vec![false; n];
        if frozen.is_empty() {
            return Self {
                forced_split,
                forced_join,
            };
        }
        // Built on the first lookup rather than up front: a shoot is usually frozen a batch or two
        // at a time, and mapping every photo to its position costs a whole pass over the sequence.
        let mut positions: Option<HashMap<PhotoId, usize>> = None;

        for b in frozen {
            let positions = positions
                .get_or_insert_with(|| seq.iter().enumerate().map(|(i, p)| (p.id(), i)).collect());
            let found: Vec<usize> = b
                .photo_ids
                .iter()
                .filter_map(|id| positions.get(id))
                .copied()
                .collect();
            // A frozen batch whose photos are no longer all here is not a lock. The user culled a
            // ten-frame burst, deleted five of its files and rescanned: forcing a join between the
            // first and last survivor swallows whatever bursts happened to sit between them, which
            // is the one outcome a freeze exists to prevent. Skipping the lock only means the
            // batch may be re-cut, which is what an unvisited batch is allowed to do anyway.
            if found.len() != b.photo_ids.len() {
                continue;
            }
            let (Some(&start), Some(&end)) = (found.iter().min(), found.iter().max()) else {
                continue;
            };
            forced_split[start] = true;
            if end + 1 < n {
                forced_split[end + 1] = true;
            }
            for slot in forced_join.iter_mut().take(end + 1).skip(start + 1) {
                *slot = true;
            }
        }
        Self {
            forced_split,
            forced_join,
        }
    }
}

/// Turn the per-boundary split flags into batches.
fn assemble<P: Photo>(seq: &[&P], splits: &[bool], verdicts: &[BoundaryVerdict]) -> Vec<Batch> {
    if seq.is_empty() {
        return Vec::new();
    }
    // A boundary that was scored is still provisional when its score sits on either side of the
    // threshold with nothing to separate it: the batch stays open to change.
    let mut members: Vec<PhotoId> = vec![seq[0].id()];
    let mut provisional = false;
    let mut batches = Vec::new();

    let flush = |members: &mut Vec<PhotoId>, provisional: &mut bool, batches: &mut Vec<Batch>| {
        batches.push(Batch {
            id: batch_id(members[0]),
            index: batches.len() as u32,
            photo_ids: std::mem::take(members),
            provisional: std::mem::replace(provisional, false),
        });
    };

    for v in verdicts {
        // A batch is provisional when any boundary touching it came out of the ambiguous zone, in
        // either direction: a scored join may be split, and a scored split may be joined back. Only
        // signatures settle both. An ambiguous boundary touches *two* batches — the one it ends and
        // the one it starts — so the flag goes on both sides of the flush: set after it and the
        // batch being closed comes back settled, and a later `submit_visual_sigs` can join a photo
        // into a batch the app has already told itself is settled.
        let ambiguous = v.decision == Decision::Ambiguous;
        if ambiguous {
            provisional = true;
        }
        if splits[v.index] {
            flush(&mut members, &mut provisional, &mut batches);
        }
        if ambiguous {
            provisional = true;
        }
        members.push(seq[v.index].id());
    }
    flush(&mut members, &mut provisional, &mut batches);
    batches
}

/// Task.md §5.3 step 5: consecutive one-photo batches close together in time are one moment that
/// happened to be broken up, and culling them apart costs the user a keypress per frame.
///
/// On by default with a 5 s window (owner's decision, todo.md §2); `single_group_window_ms = 0`
/// turns it off.
fn apply_single_grouping<P: Photo>(batches: &mut Vec<Batch>, seq: &[&P], params: &BatchParams) {
    if params.single_group_window_ms <= 0 {
        return;
    }
    let mut merged: Vec<Batch> = Vec::with_capacity(batches.len());
    // The group being built. `single` is how it *started*, not what it is now: a run of three
    // one-photo batches is still a group that began as a single, and refusing to extend it after
    // the first merge stopped a shoot's frames from ever becoming one batch.
    let mut pending: Option<(Batch, bool)> = None;
    let mut positions: Option<HashMap<PhotoId, usize>> = None;

    for batch in batches.drain(..) {
        let started_as_single = batch.photo_ids.len() == 1;
        match pending.take() {
            Some((mut prev, prev_single))
                if started_as_single
                    && prev_single
                    && singles_close(&prev, &batch, seq, &mut positions, params) =>
            {
                // Either half being provisional makes the merged batch provisional: the post-pass
                // has just overridden a boundary the ambiguous zone left open, and reporting
                // `false` would let the app treat the result as settled.
                prev.provisional |= batch.provisional;
                prev.photo_ids.extend(batch.photo_ids);
                pending = Some((prev, true));
            }
            Some((prev, _)) => {
                merged.push(prev);
                pending = Some((batch, started_as_single));
            }
            None => pending = Some((batch, started_as_single)),
        }
    }
    if let Some((last, _)) = pending {
        merged.push(last);
    }
    for (i, b) in merged.iter_mut().enumerate() {
        b.index = i as u32;
    }
    *batches = merged;
}

/// True when a one-photo group may absorb `b`.
///
/// The window is measured from the group's **first** photo, not from the one before `b`. That is
/// what stops a run of singles chaining without limit: a group stays open only while its own start
/// is still within the window, so four frames 1.5 s apart become one batch under a 5 s window and
/// the fifth frame at 6 s starts the next one.
fn singles_close<P: Photo>(
    a: &Batch,
    b: &Batch,
    seq: &[&P],
    positions: &mut Option<HashMap<PhotoId, usize>>,
    params: &BatchParams,
) -> bool {
    // Built on the first pair that could merge at all. A burst shoot has no one-photo batches to
    // group, so this map — one entry per photo in the shoot — is never built there.
    let positions =
        positions.get_or_insert_with(|| seq.iter().enumerate().map(|(i, p)| (p.id(), i)).collect());
    let (Some(&ia), Some(&ib)) = (
        positions.get(&a.photo_ids[0]),
        positions.get(&b.photo_ids[0]),
    ) else {
        return false;
    };
    let (a, b) = (seq[ia], seq[ib]);
    // The rules that keep two frames apart in normal joining still apply here: two camera bodies
    // never share a batch (§5.1), a portrait and a landscape frame are not the same moment (§5.2),
    // and a file-system timestamp is a guess that must never merge anything (REV-63), so only a
    // real capture time counts.
    if a.camera_serial() != b.camera_serial() || a.orientation() != b.orientation() {
        return false;
    }
    match (a.capture_unix_ms(), b.capture_unix_ms()) {
        (Some(x), Some(y)) => y.abs_diff(x) <= params.single_group_window_ms as u64,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::signals::Decision;
    use crate::batch::view::mock::{MockPhoto, batch_ids, batch_ids_with};

    type M = MockPhoto;

    fn no_sigs() -> HashMap<PhotoId, VisualSig> {
        HashMap::new()
    }

    #[test]
    fn a_photo_id_always_survives_the_session_database() {
        // The store keeps ids in an `i64` column by masking the top bit, which is only lossless
        // for an id that never uses that bit. FNV-1a sets it for roughly half of all inputs, so
        // this is the property that stands between a photo and its rating.
        for index in 0..5_000u64 {
            let id = photo_id(&format!("IMG_{index:04}.CR3"));
            assert_eq!(
                id.0 >> 63,
                0,
                "photo id {:#x} for frame {index} would not survive the database",
                id.0
            );
            assert_eq!(
                crate::store::db::i64_to_id(crate::store::db::id_to_i64(id.0)),
                id.0,
                "photo id {:#x} came back as a different photo",
                id.0
            );
        }
    }

    #[test]
    fn folding_a_hash_is_not_injective_and_does_not_pretend_to_be() {
        // Pinned so nobody "fixes" the docs by claiming losslessness: the top bit is OR-ed into
        // bit 62, so this pair collides. Documented on `fold_to_i64`.
        assert_eq!(fold_to_i64(0x1), fold_to_i64(0x8000_0000_0000_0000));
        // What does hold is the invariant the store needs, for every value including the extremes.
        for hash in [0, 1, u64::MAX, 0x8000_0000_0000_0000, 0x7fff_ffff_ffff_ffff] {
            assert_eq!(
                fold_to_i64(hash) >> 63,
                0,
                "{hash:#x} did not fold into 63 bits"
            );
        }
    }

    #[test]
    fn the_fixture_corpus_ids_would_not_have_survived_the_database() {
        // The measured reason this fold exists, and the reason a store that stored ids as raw
        // `u64` would quietly lose ratings for half of every real shoot. 1,404 of 2,880.
        let top_bit = |id: u64| id >> 63 == 1;
        let paths: Vec<String> = (0..2_880).map(|i| format!("IMG_{i:04}.CR3")).collect();
        let unmasked: Vec<u64> = paths.iter().map(|p| fnv1a64(p.as_bytes())).collect();
        let would_break = unmasked.iter().filter(|id| top_bit(**id)).count();
        assert!(
            would_break > 1_000,
            "expected a large fraction of ids to use the top bit, got {would_break}"
        );
        assert!(
            unmasked.iter().all(|id| fold_to_i64(*id) >> 63 == 0),
            "folding is what makes them safe"
        );
    }

    /// `n` frames `interval_ms` apart starting at `start_ms`, ids `0..n`.
    fn burst(n: u64, start_ms: i64, interval_ms: i64) -> Vec<M> {
        (0..n)
            .map(|i| M::frame(i, start_ms + interval_ms * i as i64))
            .collect()
    }

    /// The same frames renumbered from `offset`, for tests that stitch two runs together. Ids and
    /// shutter counts move together: both are what identifies a frame's position in the shoot.
    fn with_ids_offset(frames: Vec<M>, offset: u64) -> Vec<M> {
        frames
            .into_iter()
            .enumerate()
            .map(|(i, mut p)| {
                p.id = offset + i as u64;
                p.shutter_count = Some(1_000 + p.id);
                p
            })
            .collect()
    }

    /// Twelve short bursts separated by three-second pauses: a small stand-in for a shoot, with
    /// boundaries that metadata alone can find.
    fn runs_of_bursts() -> Vec<M> {
        let mut photos = Vec::new();
        for run in 0..12u64 {
            let start = 3_000 * run as i64;
            photos.extend(with_ids_offset(burst(5, start, 90), run * 5));
        }
        photos
    }

    #[test]
    fn one_burst_is_one_batch() {
        assert_eq!(
            batch_ids(&burst(40, 0, 90)),
            vec![(0..40).collect::<Vec<u64>>()]
        );
    }

    #[test]
    fn a_pause_longer_than_the_hard_split_makes_two_batches() {
        let mut photos = burst(5, 0, 90);
        photos.extend(with_ids_offset(burst(5, 5_000, 90), 5));
        assert_eq!(
            batch_ids(&photos),
            vec![vec![0, 1, 2, 3, 4], vec![5, 6, 7, 8, 9]]
        );
    }

    #[test]
    fn one_frame_makes_one_batch() {
        assert_eq!(batch_ids(&burst(1, 0, 90)), vec![vec![0]]);
    }

    #[test]
    fn an_empty_folder_gives_no_batches() {
        assert!(batch::<M>(&[], &no_sigs(), &[]).is_empty());
    }

    #[test]
    fn the_thresholds_follow_the_local_frame_rate_not_a_fixed_number() {
        // The same 500 ms gap, twice. At 11 fps it is 5.6 frame intervals: the shutter was released
        // and it is a boundary. At 6 fps it is 2.9 intervals, still inside the hard join, so it is
        // just a dropped frame. One hard-coded millisecond threshold cannot get both right, which
        // is why the thresholds are derived from the local frame interval.
        assert_eq!(
            batch_ids(&burst(4, 0, 90)),
            vec![vec![0, 1, 2, 3]],
            "4 frames 90 ms apart: one burst"
        );

        let mut fast = burst(4, 0, 90);
        fast.push(M::frame(4, 3 * 90 + 500));
        assert_eq!(
            batch_ids(&fast),
            vec![vec![0, 1, 2, 3], vec![4]],
            "11 fps + 500 ms"
        );

        let mut slow = burst(4, 0, 170);
        slow.push(M::frame(4, 3 * 170 + 500));
        assert_eq!(
            batch_ids(&slow),
            vec![vec![0, 1, 2, 3, 4]],
            "6 fps + 500 ms"
        );
    }

    #[test]
    fn the_hard_split_is_two_seconds_of_idle_camera() {
        let mut photos = burst(3, 0, 90);
        photos.push(M::frame(3, 2 * 90 + 2_001));
        photos.push(M::frame(4, 2 * 90 + 2_001 + 90));
        assert_eq!(batch_ids(&photos), vec![vec![0, 1, 2], vec![3, 4]]);
    }

    #[test]
    fn a_photo_with_no_metadata_never_invents_a_boundary() {
        // No focal length, no exposure, no shutter count, and a Δt the frame interval cannot
        // explain. With nothing to score, the pair stays together rather than being split blind.
        let photos = vec![
            M {
                focal_mm: None,
                exposure_s: None,
                aperture: None,
                iso: None,
                shutter_count: None,
                ..M::frame(0, 0)
            },
            M {
                focal_mm: None,
                exposure_s: None,
                aperture: None,
                iso: None,
                shutter_count: None,
                ..M::frame(1, 400)
            },
        ];
        assert_eq!(batch_ids(&photos), vec![vec![0, 1]]);
    }

    #[test]
    fn an_orientation_change_splits_even_at_the_frame_interval() {
        let mut photos = burst(3, 0, 90);
        photos[2] = photos[2].clone().orientation(6);
        assert_eq!(batch_ids(&photos), vec![vec![0, 1], vec![2]]);
    }

    #[test]
    fn two_camera_bodies_never_share_a_batch() {
        let photos = vec![
            M::frame(0, 0).serial("BODY1"),
            M::frame(1, 0).serial("BODY2"),
            M::frame(2, 90).serial("BODY1"),
        ];
        assert_eq!(batch_ids(&photos), vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn a_deleted_frame_run_weakens_a_join_but_never_forces_a_split() {
        // §5.2: a shutter-count jump means frames were deleted in camera. That is supporting
        // evidence at most, and a boundary by itself is exactly what this must not turn into.
        let mut photos = burst(4, 0, 90);
        photos.push(M::frame(4, 3 * 90 + 150).shutter(1_008));
        assert_eq!(
            batch_ids(&photos),
            vec![vec![0, 1, 2, 3, 4]],
            "4 deleted frames at 150 ms is still inside the burst"
        );
    }

    #[test]
    fn a_hard_split_is_never_provisional() {
        // Nothing a signature can say will move these, so the app must be free to treat them as
        // settled rather than holding the whole shoot open for review.
        let mut photos = burst(10, 0, 90);
        photos.extend(with_ids_offset(burst(10, 5_000, 90), 10));
        let out = batch(&photos, &no_sigs(), &[]);
        assert_eq!(out.len(), 2);
        assert!(
            out.iter().all(|b| !b.provisional),
            "hard joins and hard splits are settled"
        );
    }

    #[test]
    fn a_batch_cut_in_the_ambiguous_zone_is_provisional() {
        // 400 ms at 11 fps is 4.4 frame intervals: past the hard join, inside the ambiguous zone.
        // Only signatures can settle it, so the batch stays provisional.
        // 400 ms at 11 fps is 4.4 frame intervals: past the hard join of 2.5, short of the split.
        let mut photos = burst(4, 0, 90);
        photos.push(M::frame(4, 3 * 90 + 400));
        let out = batch(&photos, &no_sigs(), &[]);
        assert_eq!(out.len(), 2, "the score splits it without signatures");
        assert!(
            out.iter().any(|b| b.provisional),
            "but the split is scored, not certain: signatures may join them back"
        );
    }

    #[test]
    fn a_visited_batch_is_never_split_and_never_absorbs_its_neighbour() {
        let photos = burst(3, 0, 90);
        assert_eq!(
            batch_ids(&photos),
            vec![vec![0, 1, 2]],
            "one batch to begin with"
        );

        // The user culled photos 0 and 1 as one batch. Photo 2 must now stand alone even though
        // metadata alone puts all three in one burst: todo.md §5.4, a batch you are in is never
        // re-split under you.
        let frozen = vec![Batch {
            id: batch_id(PhotoId(0)),
            index: 0,
            photo_ids: vec![PhotoId(0), PhotoId(1)],
            provisional: false,
        }];
        let out = batch(&photos, &no_sigs(), &frozen);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].photo_ids, vec![PhotoId(0), PhotoId(1)]);
        assert_eq!(out[1].photo_ids, vec![PhotoId(2)]);
    }

    #[test]
    fn freezing_every_batch_changes_nothing() {
        let photos = burst(10, 0, 90);
        let all = batch(&photos, &no_sigs(), &[]);
        let frozen: Vec<Batch> = all
            .iter()
            .map(|b| Batch {
                provisional: false,
                ..b.clone()
            })
            .collect();
        assert_eq!(batch(&photos, &no_sigs(), &frozen), all);
    }

    #[test]
    fn a_batch_id_follows_its_first_photo() {
        assert_eq!(batch_id(PhotoId(7)), batch_id(PhotoId(7)));
        assert_ne!(batch_id(PhotoId(7)), batch_id(PhotoId(8)));
    }

    #[test]
    fn a_photo_id_is_a_pure_function_of_its_path() {
        assert_eq!(photo_id("IMG_0001.CR3"), photo_id("IMG_0001.CR3"));
        assert_ne!(photo_id("IMG_0001.CR3"), photo_id("IMG_0002.CR3"));
        assert_ne!(photo_id("IMG_0001.CR3"), photo_id("sub/IMG_0001.CR3"));
    }

    #[test]
    fn every_photo_id_survives_the_session_database() {
        // The store keeps a `u64` id in an `i64` column, so about half of all FNV-1a hashes land on
        // a number SQLite cannot hold as written. An id that does not round-trip comes back from
        // the database as a *different* photo, and the rating written against it is unreachable: the
        // user presses a key and the cull forgets it. This is checked over the whole 63-bit range
        // rather than a sample, because a sampled test would pass on the ids that happen to be
        // small.
        for id in [
            0u64,
            1,
            42,
            u64::MAX >> 1,
            0x7fff_ffff_ffff_ffff,
            0x4000_0000_0000_0000,
            0x0000_0001_0000_0000,
        ] {
            let stored = crate::store::db::id_to_i64(id);
            assert!(stored >= 0, "{id:#x} must stay positive for SQLite");
            assert_eq!(
                crate::store::db::i64_to_id(stored),
                id,
                "{id:#x} must round-trip"
            );
        }
        for seed in 0..5000u64 {
            let id = photo_id(&format!("IMG_{seed:06}.CR3")).0;
            assert!(
                id <= 0x7fff_ffff_ffff_ffff,
                "{id:#x} is out of SQLite's range"
            );
            assert_eq!(
                crate::store::db::i64_to_id(crate::store::db::id_to_i64(id)),
                id,
                "IMG_{seed:06}.CR3: {id:#x} does not round-trip"
            );
        }
    }

    #[test]
    fn folding_keeps_different_paths_apart() {
        // The fold must not merge two names, or two photos would share a row and a rating.
        for seed in 0..2000u64 {
            let a = photo_id(&format!("IMG_{seed:06}.CR3"));
            let b = photo_id(&format!("IMG_{:06}.JPG", seed + 1));
            assert_ne!(a, b, "seed {seed}");
        }
    }

    #[test]
    fn single_frames_a_few_seconds_apart_are_one_batch_by_default() {
        // Owner's decision (2026-09-29): single frames a few seconds apart are grouped, because a
        // one-photo batch costs a keypress and a screen for one frame.
        let photos = vec![M::frame(0, 0), M::frame(1, 3_000)];
        assert_eq!(batch_ids(&photos), vec![vec![0, 1]]);
    }

    #[test]
    fn single_frames_far_apart_still_stand_alone() {
        // 15 s apart is not "a few seconds": it is a different moment, and stays its own batch.
        let photos = vec![M::frame(0, 0), M::frame(1, 15_000), M::frame(2, 30_000)];
        assert_eq!(batch_ids(&photos), vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn the_grouping_can_be_turned_off() {
        let photos = vec![M::frame(0, 0), M::frame(1, 3_000)];
        let off = BatchParams {
            single_group_window_ms: 0,
            ..BatchParams::default()
        };
        assert_eq!(batch_ids_with(&photos, &off), vec![vec![0], vec![1]]);
    }

    #[test]
    fn single_frames_merge_when_the_window_is_opened() {
        // The same fixture with the grouping window open: four frames 5 s apart is four moments,
        // not one, so the window still refuses to merge them.
        let photos = vec![
            M::frame(0, 0),
            M::frame(1, 400),
            M::frame(2, 800),
            M::frame(3, 1_200),
        ];
        let params = BatchParams {
            single_group_window_ms: 2_000,
            ..BatchParams::default()
        };
        assert_eq!(batch_ids_with(&photos, &params).len(), 1);
        assert_eq!(
            batch_ids(&photos).len(),
            1,
            "400 ms at 11 fps is a boundary anyway"
        );
    }

    #[test]
    fn an_ambiguous_split_marks_both_neighbours_provisional() {
        // 400 ms at 11 fps: past the hard join of 250 ms, inside the ambiguous zone, and the score
        // splits it without a signature. The boundary touches two batches — the one it ends and the
        // one it starts — and both can still change. Marking only the second let a later
        // `submit_visual_sigs` join a photo into a batch the app had already called settled.
        let mut photos = burst(4, 0, 90);
        photos.push(M::frame(4, 3 * 90 + 400));
        let out = batch(&photos, &no_sigs(), &[]);
        assert_eq!(out.len(), 2, "the score splits it without signatures");
        assert!(
            out[0].provisional,
            "the batch the ambiguous boundary ends is not settled either"
        );
        assert!(out[1].provisional);
    }

    #[test]
    fn an_ambiguous_boundary_is_provisional_even_when_it_scored_a_join() {
        // 300 ms at 11 fps is inside the ambiguous zone but scores below the threshold, so the two
        // frames stay together. That is precisely the boundary a human is being asked to check, and
        // `provisional` used to mean "the score said split" — so it read `false` here, and the
        // tuning output printed it as `split: no` for a boundary nothing had settled.
        let mut photos = burst(4, 0, 90);
        photos.push(M::frame(4, 3 * 90 + 300));
        let out = batch_with(&photos, &no_sigs(), &[], BatchParams::default());
        assert_eq!(out.batches.len(), 1, "300 ms scores below the threshold");
        let v = out.verdicts.last().expect("a verdict per boundary");
        assert_eq!(v.decision, Decision::Ambiguous);
        assert!(v.provisional, "an ambiguous boundary can still change");
        assert!(!v.split, "but the score did not split it");
        assert!(out.batches[0].provisional, "and the batch says so");
    }

    #[test]
    fn a_frozen_batch_whose_photos_are_missing_is_not_a_lock() {
        // Three bursts three seconds apart, and the user culled the middle one together with the
        // first frame of the third — then deleted two of that batch's files. The batch's photos are
        // no longer all in the sequence, so it is not a lock: forcing a join from the first
        // surviving frame to the last swallowed the whole third burst into a batch the user had
        // already been in. Skipping the lock only means the batch may be re-cut, which is what an
        // unvisited batch is allowed to do.
        let photos: Vec<M> = [
            M::frame(0, 0),
            M::frame(1, 90),
            M::frame(2, 3_000),
            M::frame(3, 3_090),
            M::frame(4, 3_180),
            M::frame(5, 6_000),
            M::frame(6, 6_090),
        ]
        .into_iter()
        .filter(|p| p.id != 4 && p.id != 6)
        .collect();
        let frozen = vec![Batch {
            id: batch_id(PhotoId(2)),
            index: 0,
            photo_ids: vec![PhotoId(2), PhotoId(3), PhotoId(4), PhotoId(5), PhotoId(6)],
            provisional: false,
        }];
        assert_eq!(
            batch_ids(&photos),
            vec![vec![0, 1], vec![2, 3], vec![5]],
            "without the freeze this is what the shoot looks like"
        );
        let out = batch(&photos, &no_sigs(), &frozen);
        let ids: Vec<Vec<u64>> = out
            .iter()
            .map(|b| b.photo_ids.iter().map(|id| id.0).collect())
            .collect();
        assert_eq!(
            ids,
            vec![vec![0, 1], vec![2, 3], vec![5]],
            "and the third burst is not swallowed by a batch the user has been in"
        );
    }

    #[test]
    fn a_locked_boundary_still_reports_the_signatures_it_has() {
        // `had_sigs` says whether a signature existed for both frames, which is a fact about `sigs`
        // and not about the decision. It was hard-coded to `false` for every locked boundary, so a
        // re-batched shoot that had been through the visual phase read as never refined.
        let photos = burst(3, 0, 90);
        let sig = VisualSig {
            dhash: 0,
            hist: [0; 48],
        };
        let sigs: HashMap<PhotoId, VisualSig> = photos.iter().map(|p| (p.id(), sig)).collect();
        let frozen = vec![Batch {
            id: batch_id(PhotoId(0)),
            index: 0,
            photo_ids: vec![PhotoId(0), PhotoId(1)],
            provisional: false,
        }];
        let out = batch_with(&photos, &sigs, &frozen, BatchParams::default());
        assert_eq!(out.batches.len(), 2, "the visited batch still holds");
        assert!(
            out.verdicts.iter().all(|v| v.had_sigs),
            "both locked boundaries had signatures for both frames"
        );
    }

    #[test]
    fn a_run_of_single_frames_becomes_one_batch() {
        // 1.5 s at 11 fps: past the hard join of 250 ms, short of the hard split of 2 s, and scored
        // a certain split on metadata alone. Three one-photo batches span 3 s, inside the 5 s
        // grouping window, so they are one moment — but the post-pass only ever merged *pairs*,
        // because once the group had two photos the "is this a single" guard stopped it growing.
        let photos = vec![M::frame(0, 0), M::frame(1, 1_500), M::frame(2, 3_000)];
        assert_eq!(batch_ids(&photos), vec![vec![0, 1, 2]]);
    }

    #[test]
    fn a_run_of_single_frames_still_stops_at_the_window() {
        // The bound that keeps the grouping from chaining without limit: the window is measured
        // from the group's *first* photo, so the fifth frame at 6 s starts a new group however
        // close it is to the fourth.
        let photos: Vec<M> = (0..5u64).map(|i| M::frame(i, i as i64 * 1_500)).collect();
        assert_eq!(batch_ids(&photos), vec![vec![0, 1, 2, 3], vec![4]]);
    }

    #[test]
    fn grouping_two_singles_keeps_a_provisional_one_provisional() {
        // The first boundary is a hard split, so the frame before it is settled; the second is
        // ambiguous, so the frame after it is not. The post-pass merges the two, and keeping only
        // the settled one's flag reported the result as settled — including when the merge has just
        // overridden a boundary the ambiguous zone left open. (0/2100/3100 rather than
        // 0/2100/4200, where *every* gap is a hard split and nothing is provisional to lose.)
        let photos = vec![M::frame(0, 0), M::frame(1, 2_100), M::frame(2, 3_100)];
        let out = batch(&photos, &no_sigs(), &[]);
        assert_eq!(
            out.len(),
            1,
            "2.1 s and 1 s apart, both inside the 5 s window"
        );
        assert!(
            out[0].provisional,
            "one half of the group was provisional, so the group is"
        );
    }

    #[test]
    fn a_fast_frame_rate_does_not_score_every_gap_as_a_certain_split() {
        // 40 fps. The hard join is ten frame intervals wide there, so the span the score normalises
        // by — a constant six intervals, minus the join — was already negative and fell to its 0.5
        // floor: every gap past the hard join scored 1.0 and split, in exactly the range the
        // ambiguous zone is meant to cover. A 900 ms gap is 36 intervals: a quarter of a second past
        // the hard join and a second short of the hard split, so it is the one boundary a signature
        // exists to decide.
        let params = BatchParams::default();
        let photos = vec![
            M::frame(0, 0),
            M::frame(1, 25),
            M::frame(2, 50),
            M::frame(3, 75),
            M::frame(4, 975),
        ];
        let intervals =
            FrameIntervals::new(vec![0, 25, 25, 25, 900], params.default_frame_interval_ms);
        let thresholds = thresholds_at(&intervals, &params, 4);
        assert_eq!(thresholds.frame_interval_ms, 25, "the local rate is 40 fps");

        let signals = PairSignals::compute(&photos[3], &photos[4]);
        assert_eq!(signals.dt_ms, 900);
        let score = score_pair(&signals, &thresholds, None, &params);
        assert!(
            score < 1.0,
            "a gap inside the ambiguous zone is not a certain split, and it scored {score}"
        );
        assert!(score > 0.0, "nor is it no evidence at all: {score}");

        assert_eq!(
            batch_ids_with(&photos, &params),
            vec![vec![0, 1, 2, 3, 4]],
            "so metadata alone leaves the burst together"
        );
    }

    #[test]
    fn every_photo_lands_in_exactly_one_batch() {
        let photos: Vec<M> = (0..250u64).map(|i| M::frame(i, i as i64 * 120)).collect();
        let out = batch(&photos, &no_sigs(), &[]);
        let mut all: Vec<PhotoId> = out
            .iter()
            .flat_map(|b| b.photo_ids.iter().copied())
            .collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), photos.len());
    }

    #[test]
    fn batch_indices_are_dense_and_ids_match_the_first_photo() {
        let photos: Vec<M> = (0..120u64).map(|i| M::frame(i, i as i64 * 300)).collect();
        for (i, b) in batch(&photos, &no_sigs(), &[]).iter().enumerate() {
            assert_eq!(b.index as usize, i);
            assert_eq!(b.id, batch_id(b.photo_ids[0]));
            assert!(!b.photo_ids.is_empty());
        }
    }

    #[test]
    fn every_verdict_describes_the_boundary_it_produced() {
        let photos: Vec<M> = (0..150u64).map(|i| M::frame(i, i as i64 * 160)).collect();
        let out = batch_with(&photos, &no_sigs(), &[], BatchParams::default());
        assert_eq!(out.verdicts.len(), photos.len() - 1);
        assert_eq!(out.order.len(), photos.len());
        for v in &out.verdicts {
            assert_eq!(v.index, out.order[v.index].0 as usize);
            if v.decision == Decision::Join {
                assert_eq!(v.score, 0.0, "a hard join is never scored");
            }
        }
    }

    #[test]
    fn the_same_input_always_gives_the_same_batches() {
        // Every field varies, so any accidental dependence on iteration order or on a HashMap walk
        // would show up here. Same input, same batches, always: todo.md §5.3 step 6.
        let photos: Vec<M> = (0..300u64)
            .map(|i| {
                let mut p = M::frame(i, i as i64 * 90 + (i % 11) as i64 * 17);
                p = if i % 13 == 0 { p.focal(70.0) } else { p };
                p = if i % 7 == 0 { p.iso(3_200) } else { p };
                if i % 29 == 0 { p.orientation(6) } else { p }
            })
            .collect();
        let first = batch(&photos, &no_sigs(), &[]);
        assert!(
            first.len() > 3,
            "the fixture must actually contain boundaries"
        );
        for _ in 0..25 {
            assert_eq!(batch(&photos, &no_sigs(), &[]), first);
        }
    }

    #[test]
    fn the_order_photos_arrive_in_does_not_change_the_batches() {
        let photos = runs_of_bursts();
        let expected = batch_ids(&photos);
        assert!(expected.len() > 2, "the fixture must contain boundaries");
        for shift in [1usize, 7, 40, photos.len() - 1] {
            let mut rotated = photos.clone();
            rotated.rotate_left(shift);
            assert_eq!(batch_ids(&rotated), expected, "shift {shift}");
        }
    }

    #[test]
    fn frozen_batches_survive_a_reorder() {
        let photos = runs_of_bursts();
        let all = batch(&photos, &no_sigs(), &[]);
        let frozen = vec![all[0].clone(), all[all.len() / 2].clone()];
        let expected = batch(&photos, &no_sigs(), &frozen);
        let mut rotated = photos.clone();
        rotated.rotate_left(33);
        assert_eq!(batch(&rotated, &no_sigs(), &frozen), expected);
    }
}
