//! Plays mode: merge neighbouring bursts into plays.
//!
//! [`batch_mode`] runs the ordinary burst batcher first (stage 1) and, on top of it, runs
//! [`merge_into_plays`] to join adjacent bursts into longer plays. A play is a burst or a chain of
//! bursts that belong to the same moment — the user still culls by batch, so a play is just a batch
//! whose photos came from more than one burst.

use std::collections::{HashMap, HashSet};

use super::{Batch, BatchId, BatchOutcome, BatchParams, Photo, PhotoId, batch_with};
use crate::batch::signals::PairSignals;
use crate::batch::view::effective_time_ms;
use crate::batch::visual::distance;

/// How the batches from stage 1 are recombined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BatchingMode {
    /// Adjacent bursts are merged into plays (todo.md §7.3).
    #[default]
    Plays,
    /// The burst batcher's batches stand alone.
    Bursts,
}

/// How [`merge_into_plays`] weighs the evidence for joining two neighbouring bursts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayParams {
    /// Never merge bursts whose first photos are further apart than this (60 s, a different moment).
    pub max_gap_ms: i64,
    /// A gap of `max_gap_ms / gap_scale` already counts as a full "different moment" for the gap term.
    pub gap_scale: f32,
    /// Weight of the time-gap term.
    pub weight_gap: f32,
    /// Weight of the focal-change term.
    pub weight_focal: f32,
    /// Weight of the orientation-change term.
    pub weight_orientation: f32,
    /// Weight of the visual-distance term, counted only when both bursts have a signature.
    pub weight_visual: f32,
    /// Merge when the weighted score is below this.
    pub merge_threshold: f32,
}

impl Default for PlayParams {
    fn default() -> Self {
        Self {
            max_gap_ms: 60_000,
            gap_scale: 3.0,
            weight_gap: 0.5,
            weight_focal: 0.2,
            weight_orientation: 0.1,
            weight_visual: 0.2,
            merge_threshold: 0.5,
        }
    }
}

/// Stage-1 batcher plus the optional play-merge pass.
///
/// `Bursts` returns [`batch_with`] unchanged. `Plays` returns the same verdicts and order but with
/// `batches` replaced by [`merge_into_plays`].
#[must_use]
pub fn batch_mode<P: Photo>(
    photos: &[P],
    sigs: &HashMap<PhotoId, crate::batch::VisualSig>,
    frozen: &[Batch],
    params: BatchParams,
    play: PlayParams,
    mode: BatchingMode,
) -> BatchOutcome {
    let outcome = batch_with(photos, sigs, frozen, params);
    match mode {
        BatchingMode::Bursts => outcome,
        BatchingMode::Plays => BatchOutcome {
            batches: merge_into_plays(&outcome.batches, photos, sigs, frozen, play),
            ..outcome
        },
    }
}

/// Merge neighbouring bursts into plays.
///
/// The bursts are consecutive in time; for each adjacent pair a weighted score decides whether they
/// belong to the same moment. Consecutive "merge" decisions chain into one play. Frozen batches are
/// never merged.
#[must_use]
pub fn merge_into_plays<P: Photo>(
    bursts: &[Batch],
    photos: &[P],
    sigs: &HashMap<PhotoId, crate::batch::VisualSig>,
    frozen: &[Batch],
    play: PlayParams,
) -> Vec<Batch> {
    if bursts.len() <= 1 {
        return bursts.to_vec();
    }

    // id -> position in the shoot, so a batch's first photo can be pulled out of `photos`.
    let positions: HashMap<PhotoId, usize> = photos
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id(), i))
        .collect();

    // The ids of batches the user has already been in, which no play may absorb.
    let frozen_ids: HashSet<BatchId> = frozen.iter().map(|b| b.id).collect();

    // Burst `i` and burst `i + 1`, in order, so the gap between them lines up with the pair's index.
    let firsts: Vec<&Batch> = bursts.iter().collect();
    let lasts: Vec<&Batch> = bursts[1..].iter().collect();

    // gap_ms for each adjacent pair, and which pairs have a usable gap.
    let mut gaps: Vec<Option<i64>> = Vec::with_capacity(bursts.len() - 1);
    for (a, b) in firsts.iter().zip(lasts.iter()) {
        gaps.push(gap_between(a, b, photos, &positions));
    }

    // The decision for each adjacent pair.
    let mut merge: Vec<bool> = Vec::with_capacity(bursts.len() - 1);
    for (i, a) in firsts.iter().take(lasts.len()).enumerate() {
        let b = lasts[i];
        if frozen_ids.contains(&a.id) || frozen_ids.contains(&b.id) {
            merge.push(false);
            continue;
        }
        match gaps[i] {
            None => merge.push(false),
            Some(gap_ms) if gap_ms > play.max_gap_ms => merge.push(false),
            Some(gap_ms) => {
                let a_last =
                    &photos[positions[a.photo_ids.last().expect("a burst has a last photo")]];
                let b_first =
                    &photos[positions[b.photo_ids.first().expect("a burst has a first photo")]];
                let signals = PairSignals::compute(a_last, b_first);
                let score = score_pair(signals, a_last.id(), b_first.id(), gap_ms, sigs, play);
                merge.push(score < play.merge_threshold);
            }
        }
    }

    // Chain the decisions into batches.
    let mut batches = Vec::with_capacity(bursts.len());
    let mut group: Vec<PhotoId> = Vec::with_capacity(1);
    let mut provisional = false;
    let mut index = 0u32;
    for (i, b) in bursts.iter().enumerate() {
        if !group.is_empty() && i > 0 && merge[i - 1] {
            group.extend_from_slice(&b.photo_ids);
            provisional |= b.provisional;
        } else {
            finish(&mut group, &mut provisional, &mut batches, index);
            group = b.photo_ids.clone();
            provisional = b.provisional;
            index += 1;
        }
    }
    finish(&mut group, &mut provisional, &mut batches, index);
    for (i, b) in batches.iter_mut().enumerate() {
        b.index = i as u32;
    }
    batches
}

/// `effective_time_ms(first of B) - effective_time_ms(last of A)`, or `None` when a time is missing.
fn gap_between<P: Photo>(
    a: &Batch,
    b: &Batch,
    photos: &[P],
    positions: &HashMap<PhotoId, usize>,
) -> Option<i64> {
    let a_last = &photos[positions[a.photo_ids.last().expect("a burst has a last photo")]];
    let b_first = &photos[positions[b.photo_ids.first().expect("a burst has a first photo")]];
    match (effective_time_ms(a_last), effective_time_ms(b_first)) {
        (Some(ta), Some(tb)) => Some(tb - ta),
        _ => None,
    }
}

/// Build one batch from a group of consecutive bursts.
fn finish(group: &mut Vec<PhotoId>, provisional: &mut bool, batches: &mut Vec<Batch>, index: u32) {
    if group.is_empty() {
        return;
    }
    batches.push(Batch {
        id: BatchId(group[0].0),
        index,
        photo_ids: std::mem::take(group),
        provisional: std::mem::replace(provisional, false),
    });
}

/// The weighted score for joining burst A to the burst after it.
fn score_pair(
    signals: PairSignals,
    a_id: PhotoId,
    b_id: PhotoId,
    gap_ms: i64,
    sigs: &HashMap<PhotoId, crate::batch::VisualSig>,
    play: PlayParams,
) -> f32 {
    let gap_term = (gap_ms as f32 / (play.max_gap_ms as f32 / play.gap_scale)).clamp(0.0, 1.0);
    let focal_term = (signals.focal_stops / crate::batch::FOCAL_FULL_STOPS).min(1.0);
    let orientation_term = if signals.orientation_changed {
        1.0
    } else {
        0.0
    };
    let visual_term = match (sigs.get(&a_id), sigs.get(&b_id)) {
        (Some(sa), Some(sb)) => Some(distance(sa, sb).combined),
        _ => None,
    };

    // A change of a full focal-length stop is a new composition, whatever the other evidence says.
    if focal_term >= 1.0 {
        return 1.0;
    }

    let mut numerator = gap_term * play.weight_gap
        + focal_term * play.weight_focal
        + orientation_term * play.weight_orientation;
    let mut denominator = play.weight_gap + play.weight_focal + play.weight_orientation;
    if let Some(v) = visual_term {
        numerator += v * play.weight_visual;
        denominator += play.weight_visual;
    }
    numerator / denominator
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::view::mock::MockPhoto;

    fn burst(ids: &[u64], index: u32) -> Batch {
        Batch {
            id: BatchId(ids[0]),
            index,
            photo_ids: ids.iter().map(|&n| PhotoId(n)).collect(),
            provisional: false,
        }
    }

    /// Two bursts of two frames; the second starts `gap_ms` after the first ends.
    fn two_bursts(gap_ms: i64) -> (Vec<MockPhoto>, Vec<Batch>) {
        let photos = vec![
            MockPhoto::frame(1, 0),
            MockPhoto::frame(2, 200),
            MockPhoto::frame(3, 200 + gap_ms),
            MockPhoto::frame(4, 400 + gap_ms),
        ];
        (photos, vec![burst(&[1, 2], 0), burst(&[3, 4], 1)])
    }

    fn merge(photos: &[MockPhoto], bursts: &[Batch], frozen: &[Batch]) -> Vec<Batch> {
        merge_into_plays(
            bursts,
            photos,
            &HashMap::new(),
            frozen,
            PlayParams::default(),
        )
    }

    #[test]
    fn a_three_second_gap_merges_into_one_play() {
        let (photos, bursts) = two_bursts(3_000);
        let out = merge(&photos, &bursts, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].photo_ids.len(), 4);
        assert_eq!(out[0].id, BatchId(1));
    }

    #[test]
    fn a_forty_five_second_gap_stays_split() {
        let (photos, bursts) = two_bursts(45_000);
        let out = merge(&photos, &bursts, &[]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn a_one_stop_focal_change_keeps_bursts_apart() {
        let (mut photos, bursts) = two_bursts(20_000);
        photos[2] = MockPhoto::frame(3, 20_200).focal(400.0);
        photos[3] = MockPhoto::frame(4, 20_400).focal(400.0);
        let out = merge(&photos, &bursts, &[]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn frozen_batches_are_never_merged() {
        let (photos, bursts) = two_bursts(3_000);
        let out = merge(&photos, &bursts, &[bursts[1].clone()]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn bursts_mode_equals_the_burst_batcher() {
        let (photos, _) = two_bursts(3_000);
        let sigs = HashMap::new();
        let plain = batch_with(&photos, &sigs, &[], BatchParams::default());
        let out = batch_mode(
            &photos,
            &sigs,
            &[],
            BatchParams::default(),
            PlayParams::default(),
            BatchingMode::Bursts,
        );
        assert_eq!(out.batches, plain.batches);
    }
}
