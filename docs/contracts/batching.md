# Contract: ordering, batching & visual signatures

- **Owner:** core-batch
- **Consumers:** core-store, pipeline, app-logic
- **Version:** v0.1 (draft; frozen as v1.0 at the end of wave 1)

## Types

```rust
pub struct BatchId(pub u64);   // hash of the batch's first PhotoId (stable across re-runs)

pub struct Batch {
    pub id: BatchId,
    pub index: u32,               // 0-based position in the shoot
    pub photo_ids: Vec<PhotoId>,  // in capture order
    pub provisional: bool,        // true until visual signatures refined its boundaries
}

/// Computed by **pipeline** from the 256 px thumbnail, consumed by batching.
pub struct VisualSig {
    pub dhash: u64,        // see algorithm below
    pub hist: [u8; 48],    // 16 bins each for R, G, B, normalized to 0..=255
}
```

## Functions

```rust
/// Capture order. Never uses file names except as the final tie-breaker.
pub fn order(photos: &[PhotoMeta]) -> Vec<PhotoId>;

/// Deterministic. `frozen` batches (visited by the user) are returned unchanged.
pub fn batch(photos: &[PhotoMeta],
             sigs: &HashMap<PhotoId, VisualSig>,
             frozen: &[Batch]) -> Vec<Batch>;
```

## VisualSig algorithm (pipeline implements it; must match exactly)

1. Start from the 256 px thumbnail (longest edge 256), orientation applied, sRGB.
2. **dHash**: convert to grayscale (Rec. 601 luma), resize to 9×8 with area averaging, and set bit
   `row*8+col` = `px[row][col] > px[row][col+1]`.
3. **hist**: 16 bins per channel over all thumbnail pixels, each channel normalized so its largest
   bin is 255.
4. A Rust reference implementation `visual_sig(rgba: &[u8], w, h) -> VisualSig` is exported so
   pipeline can run golden tests against it (or simply call it).

## Guarantees

- Metadata-only `batch()` (empty `sigs`) returns within 2 s for 1,500 photos → provisional batches.
- With `sigs`, only ambiguous-zone boundaries can change; hard joins and hard splits never do.
- Same input → same output, always.
- Two different `camera_serial`s are never in the same batch.

## Proposed changes

(none)

## Changelog

- v0.1: initial draft.
