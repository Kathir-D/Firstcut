//! Milestone-only, opt-in: the LibRaw backend against the owner's real CR3s.
//!
//! The assertion that matters is geometric: LibRaw develops in sensor orientation and the backend
//! applies the EXIF tag once, so an orientation-8 frame must come out portrait. Doing both would
//! rotate it twice, which is exactly the v1 T4 defect the plan calls out.

use std::path::{Path, PathBuf};

use firstcut_decode::orientation_swaps_axes;
use firstcut_meta::meta::{FileKind, PhotoMeta, scan_folder};

fn photos_root() -> Option<PathBuf> {
    if std::env::var("FIRSTCUT_ALLOW_PHOTO_TESTS").ok().as_deref() != Some("1") {
        return None;
    }
    let configured = std::env::var_os("FIRSTCUT_TEST_PHOTOS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("~/Documents/testing"));
    let expanded: PathBuf = configured
        .to_str()
        .map(|text| match text.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME").map_or_else(|| PathBuf::from(text), |home| PathBuf::from(home).join(rest)),
            None => PathBuf::from(text),
        })
        .unwrap_or(configured);
    expanded.is_dir().then_some(expanded)
}

/// The first game folder with RAW files, plus one sample from it.
fn one_raw(folder: &Path) -> Option<(PathBuf, PhotoMeta)> {
    for game in ["Game1JENKS", "Game3KC", "Game4VRE", "Gane2NC"] {
        let game_folder = folder.join(game);
        let Ok(scan) = scan_folder(&game_folder) else { continue };
        let stride = (scan.photos.len() / 4).max(1);
        if let Some(photo) = scan.photos.into_iter().step_by(stride).find(|photo| matches!(photo.kind, FileKind::Raw(_))) {
            return Some((game_folder, photo));
        }
    }
    None
}

#[test]
#[ignore = "real-photo develop; run through scripts/test-with-photos.sh"]
fn a_real_raw_develops_in_sensor_orientation_and_is_rotated_once() {
    let Some(root) = photos_root() else {
        eprintln!("skipping: no test photos. Set FIRSTCUT_TEST_PHOTOS and FIRSTCUT_ALLOW_PHOTO_TESTS=1.");
        return;
    };
    let Some(backend) = firstcut_decode_libraw::backend() else {
        eprintln!("skipping: the LibRaw backend is not available on this platform");
        return;
    };
    let Some((folder, photo)) = one_raw(&root) else {
        eprintln!("skipping: the corpus has no RAW photographs");
        return;
    };
    let image = backend.decode_raw(&folder.join(&photo.rel_path), &photo).unwrap_or_else(|err| panic!("{}: {err}", photo.rel_path));
    assert_eq!(image.rgba.len(), image.width as usize * image.height as usize * 4);
    // LibRaw may crop a pixel or two, so compare the axis role rather than the exact count.
    let swapped = orientation_swaps_axes(photo.orientation);
    let (expected_long, expected_short) = (photo.width.max(photo.height), photo.width.min(photo.height));
    let (long, short) = (image.width.max(image.height), image.width.min(image.height));
    assert!((long as i64 - expected_long as i64).abs() <= 8, "{}: {long} vs {expected_long}", photo.rel_path);
    assert!((short as i64 - expected_short as i64).abs() <= 8, "{}: {short} vs {expected_short}", photo.rel_path);
    assert_eq!(
        image.width > image.height,
        (photo.width > photo.height) != swapped,
        "{} orientation {}: {}x{}",
        photo.rel_path,
        photo.orientation,
        image.width,
        image.height
    );
    eprintln!("developed {} ({}x{}, orientation {})", photo.rel_path, image.width, image.height, photo.orientation);
}

/// The fused tail (gamma table + pack + rotate) and the skipped crop copy must be the same picture
/// as rawler's stock pipeline, not merely a fast one. This is the quality gate for D11's backend:
/// every pixel is within one 8-bit level of `RawDevelop::default()` — which is the pipeline this
/// backend was measured against and the one the §5 numbers were taken with.
#[test]
#[ignore = "real-photo develop; run through scripts/test-with-photos.sh"]
fn the_fused_develop_matches_rawlers_stock_pipeline_within_one_level() {
    let Some(root) = photos_root() else {
        eprintln!("skipping: no test photos. Set FIRSTCUT_TEST_PHOTOS and FIRSTCUT_ALLOW_PHOTO_TESTS=1.");
        return;
    };
    let Some(backend) = firstcut_decode_libraw::backend() else {
        eprintln!("skipping: the RAW backend is not available on this platform");
        return;
    };
    let Some((folder, photo)) = one_raw(&root) else {
        eprintln!("skipping: the corpus has no RAW photographs");
        return;
    };
    let path = folder.join(&photo.rel_path);

    // The reference: rawler's own full pipeline, packed and rotated with the shared seam.
    let raw = rawler::decode_file(&path).unwrap_or_else(|err| panic!("{}: {err}", photo.rel_path));
    let intermediate =
        rawler::imgop::develop::RawDevelop::default().develop_intermediate(&raw).unwrap_or_else(|err| panic!("{}: {err}", photo.rel_path));
    let rawler::imgop::develop::Intermediate::ThreeColor(pixels) = intermediate else {
        panic!("{}: expected a three-colour develop", photo.rel_path);
    };
    let mut bytes = Vec::with_capacity(pixels.data.len() * 4);
    for pixel in &pixels.data {
        for value in pixel {
            let clamped = value.clamp(0.0, 1.0);
            bytes.push((clamped * 255.0 + 0.5) as u8);
        }
        bytes.push(255);
    }
    let reference = firstcut_decode::apply_orientation(
        firstcut_decode::DecodedImage {
            width: pixels.width as u32,
            height: pixels.height as u32,
            rgba: firstcut_decode::RgbaBuffer::from_bytes(&bytes),
        },
        photo.orientation,
    )
    .unwrap_or_else(|err| panic!("{}: {err}", photo.rel_path));

    let image = backend.decode_raw(&path, &photo).unwrap_or_else(|err| panic!("{}: {err}", photo.rel_path));
    assert_eq!((image.width, image.height), (reference.width, reference.height), "{}: the frames differ in size", photo.rel_path);
    assert_eq!(image.rgba.len(), reference.rgba.len(), "{}: the frames differ in length", photo.rel_path);
    let mut worst = 0i32;
    let mut differing = 0usize;
    for (ours, theirs) in image.rgba.as_slice().as_chunks::<4>().0.iter().zip(reference.rgba.as_slice().as_chunks::<4>().0.iter()) {
        let mut pixel_differs = false;
        for channel in 0..3 {
            let (Some(&a), Some(&b)) = (ours.get(channel), theirs.get(channel)) else { continue };
            let difference = (i32::from(a) - i32::from(b)).abs();
            worst = worst.max(difference);
            pixel_differs |= difference != 0;
        }
        differing += usize::from(pixel_differs);
    }
    eprintln!("{}: max channel difference {worst}, {differing} pixels differ", photo.rel_path);
    assert!(worst <= 1, "{}: the fused develop drifted {worst} levels from rawler's pipeline", photo.rel_path);
    assert!(differing * 100 < image.width as usize * image.height as usize, "{}: more than 1% of pixels differ", photo.rel_path);
}
