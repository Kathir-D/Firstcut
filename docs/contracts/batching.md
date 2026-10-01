# Contract: ordering, batching & visual signatures

- **Owner:** core-batch
- **Consumers:** core-store, pipeline, app-logic
- **Version:** v0.3 (draft; frozen as v1.0 at the end of wave 1)

## Types

```rust
pub struct BatchId(pub u64);   // the batch's first PhotoId (stable across re-runs), batch/mod.rs:39

pub struct Batch {
    pub id: BatchId,
    pub index: u32,               // 0-based position in the shoot
    pub photo_ids: Vec<PhotoId>,  // in capture order
    pub provisional: bool,        // true until visual signatures refined its boundaries
}

/// Computed by **pipeline** from the 256 px thumbnail, consumed by batching. It is the *reference*
/// implementation in Rust (batch/visual.rs:12) — the app does not reimplement it, it calls
/// `compute_visual_sig` across the FFI boundary (REV-64, ffi.rs:713).
pub struct VisualSig {
    pub dhash: u64,        // see algorithm below
    pub hist: [u8; 48],    // 16 bins each for R, G, B, normalized to 0..=255
}

/// How far apart two signatures are, each component normalized to 0.0..=1.0 (batch/visual.rs:88).
pub struct VisualDistance { pub dhash_bits: u32, pub dhash_norm: f32, pub hist_norm: f32,
                            pub combined: f32 }   // 0.6 * dhash_norm + 0.4 * hist_norm
```

## Functions

```rust
/// Capture order. Never uses file names except as the final tie-breaker
/// (`order::order`, `order/mod.rs:26`). Generic over the `Photo` trait (`batch/view.rs:14`), which is
/// the narrow set of fields ordering and batching read; `core_meta::PhotoMeta` implements it in a
/// single `impl` block (`meta/mod.rs:297`), as do the JSON fixtures and the tests' `MockPhoto`.
///
/// One consequence worth stating: the file's mtime is **not** on `PhotoMeta` (it lives in the session
/// database, which is what survives a rescan), so `PhotoMeta::file_mtime_ms` is `None` and the
/// batcher falls back to EXIF alone. A file with no EXIF date is therefore ordered last and flagged
/// by `order_with_report` (`meta/mod.rs:293-296, 316-317`).
pub fn order<P: Photo>(photos: &[P]) -> Vec<PhotoId>;

/// `order()` plus the log entries for photos whose time came from the file system or is missing.
pub fn order_with_report<P: Photo>(photos: &[P]) -> (Vec<PhotoId>, OrderReport);

/// Deterministic. `frozen` batches (visited by the user) are returned unchanged.
/// (`batch::batch`, `batch/mod.rs:219`)
pub fn batch<P: Photo>(photos: &[P],
                       sigs: &HashMap<PhotoId, VisualSig>,
                       frozen: &[Batch]) -> Vec<Batch>;

/// `batch()` with explicit `BatchParams`, returning the batches, the per-boundary `BoundaryVerdict`
/// trace and the order. This is what `firstcut gaps` sweeps thresholds with (batch/mod.rs:229).
pub fn batch_with<P: Photo>(photos: &[P], sigs: &HashMap<PhotoId, VisualSig>,
                            frozen: &[Batch], params: BatchParams) -> BatchOutcome;
```

## Thresholds

Every threshold lives in `BatchParams::default` (`batch/mod.rs:141-171`), so `firstcut gaps` can
sweep them:

| Field | Default | Meaning |
| --- | --- | --- |
| `default_frame_interval_ms` | 90 | assumed frame interval when a window has no short gaps at all (11 fps) |
| `hard_join_floor_ms` / `hard_join_factor` | 250 ms / 2.5 | Δt ≤ `max(250, 2.5·f)` is always a join |
| `hard_split_floor_ms` / `hard_split_factor` | 2 000 ms / 4.0 | Δt > `max(2 000, 4·f)` is always a split |
| `split_threshold` | 0.5 | split an ambiguous boundary whose score exceeds this |
| `single_group_window_ms` | 5 000 | consecutive one-photo batches closer together than this are one batch; 0 turns it off |

`f` is the **local** frame interval: the median Δt among neighbouring gaps shorter than 500 ms
(`LOCAL_WINDOW_MAX_MS`, `batch/signals.rs:9`), taken over a window of radius 5, then 25, then the
whole sequence's median, then `default_frame_interval_ms` (`FrameIntervals::at`,
`batch/signals.rs:240-255`). Thresholds are derived from `f` rather than hard-coded so one pair of
numbers works from 6 fps to 40 fps, and the join is clamped so a hard split can never land inside
the hard join (`thresholds_at`, `batch/mod.rs:306-317`).

## The two phases

1. **Metadata only, immediately.** `batch(photos, &HashMap::new(), &[])` on the scan's photos returns
   provisional batches. Every boundary that came out of the *ambiguous* zone marks its batch
   `provisional` — in either direction, since a scored join may split and a scored split may join
   back (`assemble`, `batch/mod.rs:437-462`). Hard joins and hard splits produce settled,
   non-provisional batches even with no signatures at all.
2. **Visual refinement, in the background.** pipeline submits signatures through
   `Session.submit_visual_sigs` in chunks as thumbnails finish (`session.rs:464`), which re-batches
   and calls `batches_changed` **only if the boundaries actually moved**. Visited batches are frozen
   (todo.md §5.4) and come back unchanged: `Locks::from_frozen` (`batch/mod.rs:391-427`) forces a
   join inside every frozen batch and a split at both of its edges, so a batch the user has been in
   is neither re-cut nor absorbed by its neighbour.

The single-frame post-pass runs on both phases, after `assemble` (`apply_single_grouping`,
`batch/mod.rs:472`): consecutive one-photo batches whose gap is within `single_group_window_ms` merge,
but never across camera bodies, orientations, or times that are not real capture times — a
file-system timestamp is a guess and must never merge anything (REV-63, `singles_close`,
`batch/mod.rs:507-530`).

## VisualSig algorithm (pipeline implements it; must match exactly)

1. Start from the 256 px thumbnail (longest edge 256), orientation applied, sRGB.
2. **dHash**: convert to grayscale (Rec. 601 luma), resize to 9×8 with area averaging (rounded, not
   truncated), and set bit `row*8+col` = `px[row][col] > px[row][col+1]`, MSB first
   (`batch/visual.rs:157-169`).
3. **hist**: 16 bins per channel over all thumbnail pixels, each channel normalized so its largest
   bin is 255; a flat channel stays all-zero rather than dividing by zero
   (`batch/visual.rs:172-196`).
4. The Rust reference `visual_sig(rgba: &[u8], w, h) -> VisualSig` (`batch/visual.rs:128`) is
   **exported over UniFFI as `compute_visual_sig`** (`ffi.rs:713`), and pipeline calls it rather than
   reimplementing the algorithm — the two sides drifting is exactly how the ambiguous zone ends up
   decided by a hash nobody tested (REV-64). It takes a thumbnail of any size: step 1's downscale to
   256 px is pipeline's thumbnail stage, not this function's.

## Guarantees

- Metadata-only `batch()` (empty `sigs`) returns provisional batches well inside its budget:
  **measured under 0.5 ms for 1,500 photos** for `order()` + `batch()` together — roughly 4,000×
  inside the 2 s target (`firstcut bench --folder`, `docs/qa/perf-baselines.md`). The metadata scan
  dominates it; batching is not the bottleneck.
- With `sigs`, only ambiguous-zone boundaries can change; hard joins and hard splits never do,
  because they are decided before any scoring (`PairSignals::decide`, `batch/signals.rs:140-158`).
  Neither can a frozen (visited) batch move (`Locks::from_frozen`, `batch/mod.rs:391`).
- Same input → same output, always — including when the photos arrive in a different order
  (`batch/mod.rs:965` and `:987`). The only unordered input is the `sigs` map, which is never
  iterated, only looked up.
- Two different `camera_serial`s are never in the same batch, and neither is a changed EXIF
  orientation, nor a pair whose capture time runs backwards (REV-63: the ordering said one way round
  and the timestamps say the other, which is a data problem, not a burst).
- A **fallback or missing** capture time never produces a hard decision: it goes to the ambiguous
  zone so the other signals decide and the batch stays `provisional`
  (`batch/signals.rs:150-152`). A pair with no evidence at all scores 0.0 and stays together, rather
  than a boundary being invented blind (`score_pair`, `batch/mod.rs:377-381`).
- Weights of unavailable evidence are left out of both numerator and denominator, so a frame with no
  focal length recorded is not pushed toward "same burst" by a weight that silently scored zero
  (`score_pair`, `batch/mod.rs:325-381`).

## Proposed changes

(none)

## Changelog

- v0.1: initial draft.
- v0.2 (2026-09-30): single-frame grouping is on by default with a 5 s window (owner's decision):
  consecutive one-photo batches within the window are one batch, but never across camera bodies,
  orientations or file-system-guessed times. Visual signatures are computed in the background by the
  app, from the 256 px thumbnail, by calling the exported Rust `compute_visual_sig`.
- v0.3 (2026-10-01): matched the contract to `core/firstcut-core/src/batch/`. Added a **Thresholds**
  section with the actual `BatchParams` defaults and the local-frame-interval rule, and a **The two
  phases** section spelling out the metadata-only → visual-refinement design, what makes a batch
  `provisional` (any boundary that came out of the ambiguous zone, in either direction), and the
  freeze rule for visited batches. `order` / `batch` are generic over the `Photo` trait, not
  `&[PhotoMeta]`; added `order_with_report`, `batch_with` and `BatchOutcome`, `VisualDistance`, and
  the `0.6 / 0.4` dHash-vs-histogram weighting. Corrected the guarantees: the batching budget is the
  **measured** < 0.5 ms, not an estimate; a fallback or missing capture time never yields a hard
  decision; a backwards time always splits; unavailable evidence is excluded from the score's
  denominator rather than counting as zero.

