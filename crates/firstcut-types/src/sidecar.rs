//! The path of the sidecar for a photo, Lightroom's way (todo.md §11): the photo's base name with
//! `.xmp`, so `Sat/IMG_0001.CR3` → `Sat/IMG_0001.xmp`. A RAW+JPEG pair shares it, as in Lightroom.
//!
//! These two moved out of v1 `xmp::document` at the M1 port because they are pure path computation
//! and the scanner (`firstcut-meta`) needs them when pairing sidecars; the ladder does not give
//! meta an edge to xmp. Everything else about sidecars (reading, merging, writing, discovering an
//! existing legacy one) stays in `firstcut-xmp`, which re-exports these two.
//!
//! Firstcut used to write `IMG_0001.CR3.xmp`, which Lightroom never reads, so a rating made here
//! did not reach it. That name is darktable's (and one Capture One reads); such a file is read as a
//! fallback when importing (`firstcut-xmp`'s `existing_sidecar`) and moves with its photo, but is
//! never renamed or rewritten: it may hold another app's edit history.

use std::path::PathBuf;

/// The sidecar path for a photo: `<basename>.xmp`.
#[must_use]
pub fn sidecar_path(raw_rel_path: &str) -> PathBuf {
    PathBuf::from(raw_rel_path).with_extension("xmp")
}

/// The sidecar name earlier builds wrote (and darktable writes): the full file name plus `.xmp`.
#[must_use]
pub fn legacy_sidecar_path(raw_rel_path: &str) -> PathBuf {
    PathBuf::from(format!("{raw_rel_path}.xmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_paths_follow_lightroom() {
        assert_eq!(sidecar_path("IMG_0001.CR3"), PathBuf::from("IMG_0001.xmp"));
        assert_eq!(
            sidecar_path("Sat.1/IMG_0001.JPG"),
            PathBuf::from("Sat.1/IMG_0001.xmp"),
            "only the last extension goes, never a dot in a folder name"
        );
        assert_eq!(legacy_sidecar_path("IMG_0001.CR3"), PathBuf::from("IMG_0001.CR3.xmp"));
    }
}
