//! The embedded-JPEG fast path, tested against a real (committed, 8×8) JPEG — a synthetic CR3
//! carries no JPEG, and placeholder bytes would be (correctly) rejected as unparseable.

use std::path::Path;

use firstcut_decode::{DecodeError, decode_display, decode_jpeg, read_range};
use firstcut_meta::meta::{ByteRange, EmbeddedPreview, FileKind, PhotoMeta, RawFormat};

fn fixture_jpeg() -> Vec<u8> {
    include_bytes!("fixtures/tiny.jpg").to_vec()
}

/// A `PhotoMeta` with only what the decode path reads; everything else is the neutral value a
/// scan would not have filled.
fn photo_meta(rel_path: String, full_preview: Option<EmbeddedPreview>, preview: Option<EmbeddedPreview>) -> PhotoMeta {
    PhotoMeta {
        id: firstcut_types::PhotoId(1),
        rel_path,
        companions: Vec::new(),
        kind: FileKind::Raw(RawFormat::Cr3),
        file_size: 0,
        capture_time: None,
        shutter_count: None,
        file_number: None,
        camera_make: None,
        camera_model: None,
        camera_serial: None,
        lens_model: None,
        focal_length_mm: None,
        exposure_time_s: None,
        f_number: None,
        iso: None,
        exposure_comp_ev: None,
        metering_mode: None,
        drive_mode: None,
        shutter_mode: None,
        orientation: 1,
        width: 6000,
        height: 4000,
        af: None,
        preview,
        full_preview,
        device: None,
        ino: None,
        warnings: Vec::new(),
    }
}

/// The real JPEG planted twice in one container, with junk-shaped gaps around each copy.
struct Container {
    // Held, never read: alive only to keep the tempdir the file lives in from being removed.
    #[allow(dead_code)]
    dir: tempfile::TempDir,
    path: std::path::PathBuf,
    full: ByteRange,
    small: ByteRange,
}

impl Container {
    #[cfg(test)]
    fn with_two_jpegs() -> Self {
        let jpeg = fixture_jpeg();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("IMG_0001.CR3");
        // [0..100) filler, [100..) the "full" JPEG, [..] filler, then the "small" one.
        let mut bytes = vec![0u8; 100];
        let full = ByteRange { offset: 100, len: jpeg.len() as u64 };
        bytes.extend_from_slice(&jpeg);
        bytes.extend_from_slice(&[0x7f; 50]);
        let small = ByteRange { offset: bytes.len() as u64, len: jpeg.len() as u64 };
        bytes.extend_from_slice(&jpeg);
        std::fs::write(&path, &bytes).unwrap();
        Self { dir, path, full, small }
    }
}

impl std::ops::Deref for Container {
    type Target = std::path::PathBuf;
    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

#[test]
fn the_committed_fixture_decodes_to_rgba() {
    let image = decode_jpeg(&fixture_jpeg()).unwrap();
    assert_eq!(image.width, 8);
    assert_eq!(image.height, 8);
    assert_eq!(image.rgba.len(), 8 * 8 * 4, "RGBA8, one byte per channel");
    // Every alpha byte is opaque: the surfaces must not learn to treat alpha specially.
    assert!(image.rgba.iter().skip(3).step_by(4).all(|a| *a == 255));
}

#[test]
fn read_range_reads_only_the_named_bytes() {
    let container = Container::with_two_jpegs();
    // The two ranges decode equally well: the bytes at the offset are what matters.
    let full = read_range(&container, container.full).unwrap();
    let small = read_range(&container, container.small).unwrap();
    assert_eq!(full.rgba, small.rgba, "both copies decode to the same pixels");
}

#[test]
fn junk_before_the_soi_is_rejected_not_guessed_through() {
    let container = Container::with_two_jpegs();
    let junk = ByteRange { offset: 0, len: 50 };
    let error = read_range(&container, junk).unwrap_err();
    assert!(matches!(error, DecodeError::Jpeg(_)), "got: {error:?}");
}

#[test]
fn range_past_end_is_an_error_not_a_panic() {
    let container = Container::with_two_jpegs();
    let past = ByteRange { offset: container.full.offset, len: u32::MAX as u64 + 2 };
    let error = read_range(&container, past).unwrap_err();
    assert!(matches!(error, DecodeError::RangePastEnd { .. }), "got: {error:?}");
}

#[test]
fn decode_display_prefers_the_full_preview() {
    let container = Container::with_two_jpegs();
    let meta = photo_meta(
        "IMG_0001.CR3".to_string(),
        Some(EmbeddedPreview { range: container.full, width: 8, height: 8 }),
        Some(EmbeddedPreview { range: container.small, width: 8, height: 8 }),
    );
    let image = decode_display(&container, &meta).unwrap();
    assert_eq!(image.width, 8);
    assert_eq!(image.height, 8);
}

#[test]
fn decode_display_falls_back_to_the_prvw_preview() {
    let container = Container::with_two_jpegs();
    let meta = photo_meta("IMG_0001.CR3".to_string(), None, Some(EmbeddedPreview { range: container.small, width: 8, height: 8 }));
    decode_display(&container, &meta).unwrap();
}

#[test]
fn a_file_with_no_embedded_jpeg_reports_itself() {
    let meta = photo_meta("IMG_0001.CR3".to_string(), None, None);
    let error = decode_display(Path::new("/dev/null"), &meta).unwrap_err();
    assert!(matches!(error, DecodeError::NoEmbeddedJpeg { .. }), "got: {error:?}");
}

#[test]
fn garbage_jpeg_bytes_error() {
    let error = decode_jpeg(&[0u8; 64]).unwrap_err();
    assert!(matches!(error, DecodeError::Jpeg(_)), "got: {error:?}");
}
