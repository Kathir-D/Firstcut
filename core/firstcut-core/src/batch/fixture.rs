//! Compatibility re-export. The types live in [`crate::meta`] now.
//!
//! This used to declare `PhotoMeta`, `CaptureTime`, `FileKind` and friends itself, as core-batch's
//! stand-in while the real scanner did not exist. It does not any more: the scanner, the CLI, the
//! tests and the FFI all need the *same* type, and two definitions means a permanent adapter
//! between them -- which is exactly the shape of bug REV-56 describes, where a photo reads as
//! Keep in one layer and Unrated in another.
//!
//! The re-exports keep every existing `use crate::batch::fixture::PhotoMeta` working, so this is a
//! move rather than a rewrite. `meta` is the one definition; nothing here adds to it.

pub use crate::meta::{
    AfInfo, AfPoint, CaptureTime, FileKind, Folder, PhotoFingerprint, PhotoMeta, PreviewSpan,
    RawFormat, ScanDump, SkippedFile, TimeSource,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::view::Photo;

    /// The id has to be a function of the path so the same folder scanned twice produces the same
    /// ids -- resume, the ground-truth F1 numbers and the CI regression test all depend on it.
    #[test]
    fn the_id_is_derived_from_the_path() {
        let mut a = PhotoMeta::default();
        a.rel_path = "IMG_0001.CR3".into();
        let mut b = PhotoMeta::default();
        b.rel_path = "IMG_0002.CR3".into();
        assert_ne!(a.id(), b.id());
        assert_eq!(a.id(), crate::meta::stable_id("IMG_0001.CR3"));
    }

    /// A `capture_time` whose source is `FileModified` is a *fallback*: `file_mtime_ms` has to
    /// report it so the batcher can refuse to hard-join on it (REV-63).
    #[test]
    fn a_fallback_time_is_reported_as_an_mtime() {
        let mut p = PhotoMeta::default();
        p.capture_time = Some(CaptureTime {
            unix_ms: 1_000,
            subsec_resolution_ms: 1000,
            offset_minutes: None,
            source: TimeSource::FileModified,
        });
        assert_eq!(p.file_mtime_ms(), Some(1_000));
        assert!(!crate::batch::view::time_is_fallback(&p) == false);
    }

    /// ...and an EXIF time must not be mistaken for one.
    #[test]
    fn an_exif_time_is_not_a_fallback() {
        let mut p = PhotoMeta::default();
        p.capture_time = Some(CaptureTime {
            unix_ms: 1_000,
            subsec_resolution_ms: 10,
            offset_minutes: Some(-360),
            source: TimeSource::Exif,
        });
        assert_eq!(p.file_mtime_ms(), None);
        assert!(!crate::batch::view::time_is_fallback(&p));
    }

    /// The dump is sorted, so the same folder scanned twice is byte-identical (REV-17).
    #[test]
    fn the_dump_is_sorted_by_path() {
        let mut a = PhotoMeta::default();
        a.rel_path = "IMG_0002.CR3".into();
        let mut b = PhotoMeta::default();
        b.rel_path = "IMG_0001.CR3".into();
        let dump = ScanDump::new(vec![a, b], vec![]);
        assert_eq!(dump.schema, "photo-meta/1");
        assert_eq!(dump.photos[0].rel_path, "IMG_0001.CR3");
    }

    /// A photo with no serial, no time or no shutter count has no fingerprint, and must say so
    /// rather than hash something incomplete into an identity that collides with another photo's.
    #[test]
    fn a_fingerprint_needs_every_component() {
        let mut p = PhotoMeta::default();
        assert!(p.fingerprint().is_none(), "nothing at all");
        p.camera_serial = Some("1".into());
        assert!(p.fingerprint().is_none(), "serial but no time");
        p.capture_time = Some(CaptureTime {
            unix_ms: 5,
            subsec_resolution_ms: 10,
            offset_minutes: None,
            source: TimeSource::Exif,
        });
        assert!(p.fingerprint().is_none(), "no shutter count");
        p.shutter_count = Some(7);
        p.file_size = 100;
        let fp = p.fingerprint().expect("complete");
        assert_eq!(fp.shutter_count, 7);
        assert_eq!(fp.camera_serial, "1");
    }
}
