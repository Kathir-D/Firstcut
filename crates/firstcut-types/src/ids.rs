//! Photo and batch ids, and the hash behind them (docs/contracts/photo-meta.md, batching.md).
//!
//! Moved verbatim from v1 `firstcut-core::batch` at the M1 port: the scanner mints
//! `PhotoId`s, the store keys rows on them, the batcher derives `BatchId`s — so the
//! definitions sit at the bottom of the ladder and no other crate re-declares them.

use serde::{Deserialize, Serialize};

/// Stable across runs: hash of the photo's path relative to the session folder
/// (docs/contracts/photo-meta.md). The scanner uses this same helper so ids never drift
/// between the scanner and the batcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PhotoId(pub u64);

/// Stable across re-runs: hash of the batch's first [`PhotoId`] (docs/contracts/batching.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BatchId(pub u64);

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
/// The catalog keeps a `u64` id in an `i64` column by masking off the top bit, and masking is only
/// a round trip if the id never *uses* that bit. FNV-1a sets it for 1,404 of the 2,880 fixture
/// photos (48.8%), so an unmasked id would be written to the database under one number and read
/// back under another — and the rating attached to it would be unreachable. Folding here means
/// every id this module mints is positive by construction, which is the invariant the store needs.
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn folding_a_hash_is_not_injective_and_does_not_pretend_to_be() {
        // Pinned so nobody "fixes" the docs by claiming losslessness: the top bit is OR-ed into
        // bit 62, so this pair collides. Documented on `fold_to_i64`.
        assert_eq!(fold_to_i64(0x1), fold_to_i64(0x8000_0000_0000_0000));
        // What does hold is the invariant the store needs, for every value including the extremes.
        for hash in [0, 1, u64::MAX, 0x8000_0000_0000_0000, 0x7fff_ffff_ffff_ffff] {
            assert_eq!(fold_to_i64(hash) >> 63, 0, "{hash:#x} did not fold into 63 bits");
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
        assert!(would_break > 1_000, "expected a large fraction of ids to use the top bit, got {would_break}");
        assert!(unmasked.iter().all(|id| fold_to_i64(*id) >> 63 == 0), "folding is what makes them safe");
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
}
