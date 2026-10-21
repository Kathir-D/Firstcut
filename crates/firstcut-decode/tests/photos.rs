//! Milestone-only, opt-in: the embedded-JPEG display path on the owner's real photographs.
//!
//! `scripts/test-with-photos.sh` sets both variables and runs ignored tests. The photos live
//! outside the repo (they are the owner's), so this file never asserts on a committed fixture.
//! What it pins is the M1 defect the showcase found: decoded pixels are in display orientation,
//! because the decode seam applies the EXIF tag exactly once.

use std::path::{Path, PathBuf};

use firstcut_decode::{decode_display_at, orientation_swaps_axes};
use firstcut_meta::meta::{PhotoMeta, scan_folder};

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

/// Every RAW/JPEG group scan knows about, across the four games, evenly sampled. The game folder
/// travels with the photo because `rel_path` is relative to it, not to the corpus root.
fn sample(folder: &Path) -> Vec<(PathBuf, PhotoMeta)> {
    let games = ["Game1JENKS", "Game3KC", "Game4VRE", "Gane2NC"];
    let mut photos = Vec::new();
    for game in games {
        let game_folder = folder.join(game);
        let Ok(scan) = scan_folder(&game_folder) else { continue };
        let stride = (scan.photos.len() / 3).max(1);
        photos.extend(scan.photos.into_iter().step_by(stride).take(3).map(|photo| (game_folder.clone(), photo)));
    }
    photos
}

#[test]
#[ignore = "real-photo sweep; run through scripts/test-with-photos.sh"]
fn every_sampled_photo_displays_in_display_orientation() {
    let Some(root) = photos_root() else {
        eprintln!("skipping: no test photos. Set FIRSTCUT_TEST_PHOTOS and FIRSTCUT_ALLOW_PHOTO_TESTS=1.");
        return;
    };
    let photos = sample(&root);
    assert!(!photos.is_empty(), "the corpus should yield photographs");
    let mut checked = 0usize;
    for (game_folder, photo) in &photos {
        let Some(_preview) = photo.full_preview.or(photo.preview) else { continue };
        let path = game_folder.join(&photo.rel_path);
        let image = decode_display_at(&path, photo, 1600).unwrap_or_else(|err| panic!("{}: {err}", photo.rel_path));
        assert!(image.width > 0 && image.height > 0 && image.rgba.len() == image.width as usize * image.height as usize * 4);
        let expects_portrait = (photo.width > photo.height) == orientation_swaps_axes(photo.orientation);
        if photo.width != photo.height {
            assert_eq!(image.width < image.height, expects_portrait, "{} orientation {}", photo.rel_path, photo.orientation);
        }
        checked += 1;
    }
    assert!(checked > 0, "no sampled photo had an embedded preview");
    eprintln!("checked {checked} photographs for display orientation");
}
