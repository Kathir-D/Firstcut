//! The embedded-JPEG fast path, tested against a real (committed, 8×8) JPEG — a synthetic CR3
//! carries no JPEG, and placeholder bytes would be (correctly) rejected as unparseable.

use std::path::Path;

use firstcut_decode::{
    DecodeError, DecodedImage, apply_orientation, decode_display, decode_display_at, decode_jpeg, downsample_to, histogram, orientation_swaps_axes,
    read_range,
};
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
fn decoded_pixels_are_four_byte_aligned_for_a_zero_copy_handoff() {
    let image = decode_jpeg(&fixture_jpeg()).unwrap();
    assert_eq!(image.rgba.as_slice().as_ptr() as usize % 4, 0, "the buffer can be adopted as Color32");
    assert_eq!(image.rgba.len(), 8 * 8 * 4);
    assert_eq!(image.rgba.into_words().len(), 8 * 8, "one aligned word per pixel");
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
fn the_display_target_picks_the_smallest_embedded_jpeg_that_covers_it() {
    let container = Container::with_two_jpegs();
    // The full-preview range is deliberately broken: if a request covered only by the 8 px
    // PRVW ever reads the full copy, this test errors instead of passing silently.
    let broken_full = EmbeddedPreview { range: ByteRange { offset: container.full.offset, len: 1 }, width: 6000, height: 4000 };
    let preview = EmbeddedPreview { range: container.small, width: 8, height: 8 };
    let meta = photo_meta("IMG_0001.CR3".to_string(), Some(broken_full), Some(preview));
    let image = decode_display_at(&container, &meta, 8).expect("the PRVW covers an 8 px request");
    assert_eq!((image.width, image.height), (8, 8));
    assert!(decode_display_at(&container, &meta, 9).is_err(), "a 9 px request must go to the broken full preview");
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

// ---- orientation and histogram (the v1 audit's fourth defect) ------------------------------

/// A `width`×`height` frame with pixel `(x, y)` set to `(r, g, b, 255)` and everything else black.
fn framed(width: u32, height: u32, pixels: &[(u32, u32, [u8; 3])]) -> DecodedImage {
    let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];
    for (x, y, [r, g, b]) in pixels {
        let index = ((*y as usize) * (width as usize) + (*x as usize)) * 4;
        if let Some(slot) = rgba.get_mut(index..index.saturating_add(4)) {
            slot.copy_from_slice(&[*r, *g, *b, 255]);
        }
    }
    DecodedImage { width, height, rgba: rgba.into() }
}

fn pixel(image: &DecodedImage, x: u32, y: u32) -> [u8; 4] {
    let index = ((y as usize) * (image.width as usize) + (x as usize)) * 4;
    let mut out = [0u8; 4];
    if let Some(slot) = image.rgba.get(index..index.saturating_add(4)) {
        out.copy_from_slice(slot);
    }
    out
}

#[test]
fn every_exif_orientation_maps_the_corners_the_way_the_spec_says() {
    // A 3×2 frame whose six pixels are all distinct, so every transform is unambiguous.
    // Source layout (x, y):
    //   (0,0)=1 (1,0)=2 (2,0)=3
    //   (0,1)=4 (1,1)=5 (2,1)=6
    let source = framed(3, 2, &[(0, 0, [1, 0, 0]), (1, 0, [2, 0, 0]), (2, 0, [3, 0, 0]), (0, 1, [4, 0, 0]), (1, 1, [5, 0, 0]), (2, 1, [6, 0, 0])]);
    let expected: &[(u8, u32, u32, u8)] = &[
        // orientation, output width, output height, red channel at (0,0)
        (1, 3, 2, 1),
        (2, 3, 2, 3),
        (3, 3, 2, 6),
        (4, 3, 2, 4),
        (5, 2, 3, 1),
        (6, 2, 3, 4),
        (7, 2, 3, 6),
        (8, 2, 3, 3),
    ];
    for (orientation, width, height, corner) in expected {
        let oriented = apply_orientation(source.clone(), *orientation).unwrap();
        assert_eq!((oriented.width, oriented.height), (*width, *height), "orientation {orientation} geometry");
        assert_eq!(pixel(&oriented, 0, 0)[0], *corner, "orientation {orientation}: the top-left source pixel must land top-left");
        // The transforms are bijections: every source pixel survives exactly once.
        let mut seen: Vec<u8> =
            (0..oriented.width).flat_map(|x| (0..oriented.height).map(move |y| (x, y))).map(|(x, y)| pixel(&oriented, x, y)[0]).collect();
        seen.sort_unstable();
        assert_eq!(seen, vec![1, 2, 3, 4, 5, 6], "orientation {orientation} loses or duplicates a pixel");
        assert_eq!(orientation_swaps_axes(*orientation), *orientation >= 5);
    }
}

#[test]
fn orientation_one_and_invalid_values_leave_the_frame_alone() {
    let source = framed(3, 2, &[(0, 0, [9, 9, 9])]);
    for orientation in [0, 1, 9, 255] {
        assert_eq!(apply_orientation(source.clone(), orientation).unwrap(), source, "orientation {orientation}");
    }
}

#[test]
fn a_corrupt_dimension_at_a_rotated_orientation_errors_instead_of_panicking() {
    // width×height×4 does not match the buffer: the checked path reports it.
    let broken = DecodedImage { width: 3, height: 2, rgba: vec![0; 3].into() };
    let error = apply_orientation(broken, 6).unwrap_err();
    assert!(matches!(error, DecodeError::SizeMismatch { .. }), "got: {error:?}");
}

#[test]
fn decode_display_applies_the_exif_orientation_exactly_once() {
    let container = Container::with_two_jpegs();
    let meta = photo_meta("IMG_0001.CR3".to_string(), Some(EmbeddedPreview { range: container.full, width: 8, height: 8 }), None);
    let upright = decode_display(&container, &meta).unwrap();
    let mut rotated = meta.clone();
    rotated.orientation = 6;
    let shown = decode_display(&container, &rotated).unwrap();
    // The 8×8 fixture is square, so geometry alone cannot see the rotation; the pixels can.
    let expected = apply_orientation(upright, 6).unwrap();
    assert_eq!(shown, expected, "decode_display must apply the tag, and only once");
}

#[test]
fn the_parallel_downsample_hits_the_requested_edge_without_an_integer_stride_undershoot() {
    // 4.8 MB is above the parallel threshold. The long side is exactly 512 (not a 3-pixel
    // stride's 400), the aspect is preserved, and every destination row is filled.
    let image = framed(1200, 1000, &[(23, 16, [10, 20, 30]), (1197, 997, [40, 50, 60])]);
    let reduced = downsample_to(image, 512);
    assert_eq!((reduced.width, reduced.height), (512, 427), "the long side is the requested 512 px");
    assert_eq!(pixel(&reduced, 10, 7), [10, 20, 30, 255], "destination (10, 7) samples source (23, 16)");
    assert_eq!(pixel(&reduced, 511, 426), [40, 50, 60, 255], "the last destination pixel is the clamped last sample");
}

#[test]
fn a_smaller_frame_is_not_upscaled() {
    let image = framed(1200, 1000, &[(0, 0, [1, 2, 3])]);
    let same = downsample_to(image, 2000);
    assert_eq!((same.width, same.height), (1200, 1000));
}

#[test]
fn decode_display_at_caps_the_decoded_size() {
    let container = Container::with_two_jpegs();
    let meta = photo_meta("IMG_0001.CR3".to_string(), Some(EmbeddedPreview { range: container.full, width: 8, height: 8 }), None);
    let thumb = decode_display_at(&container, &meta, 4).unwrap();
    assert!(thumb.longest_edge() <= 4, "got {}×{}", thumb.width, thumb.height);
    assert!(thumb.longest_edge() >= 1);
}

#[test]
fn the_histogram_is_computed_from_the_oriented_frame() {
    // Big enough for the 256 px reduction to stride by 4. Two uniquely coloured pixels are
    // placed so each is sampled by exactly one of the two orientations' grids:
    //  - source (4, 0) is on the un-oriented grid and falls off it once rotated;
    //  - source (0, 1019) is off the un-oriented grid and lands on (4, 0) after orientation 6.
    // A histogram path that re-decoded without the tag would report GREEN; the oriented frame
    // reports MAGENTA.
    let image = framed(512, 1024, &[(4, 0, [0, 255, 0]), (0, 1019, [255, 0, 255])]);
    let oriented = apply_orientation(image, 6).unwrap();
    assert_eq!((oriented.width, oriented.height), (1024, 512));
    let bins = histogram(&oriented);
    // Magenta (255, 0, 255) lands in red bin 63 and blue bin 63, with no green at all.
    assert!(bins.red[63] > 0.0 && bins.blue[63] > 0.0, "the magenta pixel must be sampled");
    assert_eq!(bins.red[63], bins.blue[63], "magenta contributes equally to red and blue");
    assert_eq!(bins.green[63], 0.0, "the green pixel fell off the oriented sampling grid");
    let green_only = framed(512, 1024, &[(4, 0, [0, 255, 0])]);
    let unoriented_bins = histogram(&green_only);
    assert!(unoriented_bins.green[63] > 0.0, "control: the un-oriented grid samples the green pixel");
    assert_eq!(bins.sampled_pixels, unoriented_bins.sampled_pixels, "the reduction samples the same count either way");
}

#[test]
fn a_histogram_of_nothing_is_empty_not_a_panic() {
    let empty = DecodedImage { width: 0, height: 0, rgba: Vec::new().into() };
    let bins = histogram(&empty);
    assert_eq!(bins.sampled_pixels, 0);
    assert!(bins.luma.iter().all(|bin| *bin == 0.0));
}
