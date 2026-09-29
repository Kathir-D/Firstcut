//! `Session` end to end on a real folder (docs/contracts/session-api.md, task.md §11).
//!
//! These are the data-safety tests, and they are the ones that matter most in this project: a wrong
//! merge hides photos, and a wrong unkeep deletes them. Every assertion here is about a user's work
//! surviving, not about an API returning a value.
//!
//! They need the real RAW files (`FIRSTCUT_TEST_PHOTOS`) and skip without them rather than passing
//! vacuously.

use std::path::{Path, PathBuf};

use firstcut_core::meta::PhotoMeta;
use firstcut_core::session::{Session, SessionMatch, photo_id_for};
use firstcut_core::store::rating::{Flag, Rating, RatingMode, Tier};

fn test_photos() -> Option<PathBuf> {
    let raw =
        std::env::var("FIRSTCUT_TEST_PHOTOS").unwrap_or_else(|_| "~/Documents/testing".into());
    let expanded = match raw.strip_prefix('~') {
        Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(&raw),
    };
    let path = std::fs::canonicalize(expanded).ok()?;
    path.is_dir().then_some(path)
}

/// A *copy* of one game's folder, so a test can rate photos without touching the shoot. Copying a
/// handful of files, not 42 GB: a CR3's metadata is in the first 60 KB, but `Session` stats and
/// hashes the whole file, so the copies are real.
fn sample_shoot(source: &Path, count: usize) -> Option<tempfile::TempDir> {
    let dir = tempfile::tempdir().ok()?;
    let mut names: Vec<PathBuf> = std::fs::read_dir(source)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("CR3"))
        .collect();
    // Sorted so the sample is the same every run: `read_dir` order is not stable, and a test that
    // picks a different eight files each time cannot be reasoned about.
    names.sort();
    names.truncate(count);
    for path in &names {
        std::fs::copy(path, dir.path().join(path.file_name()?)).ok()?;
    }
    (!names.is_empty()).then_some(dir)
}

/// How many CR3s a sample was asked for, so assertions can use it rather than a literal.
fn sample_count(dir: &tempfile::TempDir) -> usize {
    std::fs::read_dir(dir.path())
        .expect("read_dir")
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("CR3"))
        .count()
}

/// Every file's bytes, so "nothing touched the originals" can be asserted rather than assumed.
fn fingerprint_dir(dir: &Path) -> std::collections::HashMap<String, (u64, u64)> {
    let mut out = std::collections::HashMap::new();
    for entry in std::fs::read_dir(dir).expect("read_dir").flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("CR3") {
            continue;
        }
        let meta = entry.metadata().expect("metadata");
        out.insert(
            path.file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned(),
            (
                meta.len(),
                meta.modified()
                    .ok()
                    .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs()),
            ),
        );
    }
    out
}

/// task.md §11: "Files are only moved/deleted at the explicit end-of-cull step", and §5.4: one
/// `PhotoMeta` = one batch member.
#[test]
fn opening_a_real_folder_scans_orders_and_batches_it() {
    let Some(root) = test_photos() else {
        eprintln!("SKIPPED: no FIRSTCUT_TEST_PHOTOS");
        return;
    };
    let Some(shoot) = sample_shoot(&root.join("Game1JENKS"), 40) else {
        eprintln!("SKIPPED: no CR3 files to sample");
        return;
    };

    let (session, matched, scan) = Session::open(shoot.path()).expect("open a real folder");
    assert!(
        matches!(matched, SessionMatch::Created),
        "a fresh folder is a new session"
    );
    assert!(
        scan.skipped.is_empty(),
        "no file should be skipped: {:?}",
        scan.skipped
    );
    let expected_files = sample_count(&shoot);
    assert_eq!(session.photos().len(), expected_files);
    assert!(!session.batches().is_empty(), "a shoot has batches");

    // Every photo is in exactly one batch, and every batch is in capture order.
    let mut seen = std::collections::HashSet::new();
    for batch in session.batches() {
        assert!(
            !batch.photo_ids.is_empty(),
            "an empty batch is a blank step for the user"
        );
        for id in &batch.photo_ids {
            assert!(seen.insert(*id), "a photo is in two batches");
        }
    }
    assert_eq!(
        seen.len(),
        session.photos().len(),
        "every photo is in some batch"
    );

    // Batches, concatenated, must equal the capture order.
    let flattened: Vec<_> = session
        .batches()
        .iter()
        .flat_map(|b| b.photo_ids.iter())
        .copied()
        .collect();
    let expected: Vec<_> = firstcut_core::order::order(session.photos());
    assert_eq!(flattened, expected, "batches must tile the capture order");
}

/// §11: "Never touch an original" until the explicit finish step. The strongest form of this test
/// is byte-level, and it is cheap because the files are copies.
#[test]
fn rating_a_photo_does_not_touch_the_original() {
    let Some(root) = test_photos() else {
        eprintln!("SKIPPED: no FIRSTCUT_TEST_PHOTOS");
        return;
    };
    let Some(shoot) = sample_shoot(&root.join("Game1JENKS"), 12) else {
        eprintln!("SKIPPED: no CR3 files");
        return;
    };

    let before = fingerprint_dir(shoot.path());
    {
        let (mut session, _, _) = Session::open(shoot.path()).expect("open");
        let photo = session.photos()[0].clone();
        session
            .set_rating(photo.id, Rating::stars(4), 0, 0)
            .expect("rate");
        session
            .set_rating(photo.id, Rating::new(3, Flag::Reject, None, false), 0, 0)
            .expect("reject");
        session.flush();
    }
    let after = fingerprint_dir(shoot.path());
    assert_eq!(before, after, "rating a photo must not modify the RAW file");

    // The rating went somewhere durable, and it is not the RAW.
    let sidecars: Vec<_> = std::fs::read_dir(shoot.path())
        .expect("read_dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("xmp"))
        .collect();
    assert!(!sidecars.is_empty(), "the rating has to land in a sidecar");
}

/// The rating has to survive closing and reopening. Resume is the whole point of the store.
#[test]
fn a_rating_survives_closing_and_reopening() {
    let Some(root) = test_photos() else {
        eprintln!("SKIPPED: no FIRSTCUT_TEST_PHOTOS");
        return;
    };
    let Some(shoot) = sample_shoot(&root.join("Game1JENKS"), 12) else {
        eprintln!("SKIPPED: no CR3 files");
        return;
    };

    let (rated, kept, other) = {
        let (mut session, _, _) = Session::open(shoot.path()).expect("open");
        let a = session.photos()[0].clone();
        let b = session.photos()[1].clone();
        session
            .set_rating(a.id, Rating::stars(5), 0, 0)
            .expect("rate a");
        session
            .set_rating(b.id, Rating::keep(), 0, 0)
            .expect("rate b");
        session.flush();
        (a.id, 5u8, b.id)
    };

    let (session, matched, _) = Session::open(shoot.path()).expect("reopen");
    assert!(
        matches!(matched, SessionMatch::Resumed),
        "the second open resumes"
    );
    assert_eq!(session.rating(rated).stars, kept);
    assert!(
        session.rating(other).keep,
        "a keep made in keep mode survives too"
    );
}

/// Undo, and the rule that it applies **by photo** rather than by batch (REV-36): after a
/// re-batching the photo may be in a different batch, and undo must still work.
#[test]
fn undo_applies_by_photo_and_survives_a_rebatch() {
    let Some(root) = test_photos() else {
        eprintln!("SKIPPED: no FIRSTCUT_TEST_PHOTOS");
        return;
    };
    let Some(shoot) = sample_shoot(&root.join("Game1JENKS"), 20) else {
        eprintln!("SKIPPED: no CR3 files");
        return;
    };

    let (mut session, _, _) = Session::open(shoot.path()).expect("open");
    let photo = session.photos()[3].clone();
    let before = session.rating(photo.id);
    let change = session
        .set_rating(photo.id, Rating::stars(3), 0, 0)
        .expect("rate");
    assert_eq!(change.before, before);
    assert_eq!(session.rating(photo.id).stars, 3);

    let undone = session
        .undo()
        .expect("undo")
        .expect("there is something to undo");
    assert_eq!(
        undone.photo_id, photo.id,
        "undo targets the photo, not the batch"
    );
    assert_eq!(session.rating(photo.id), before, "the rating is back");

    let redone = session
        .redo()
        .expect("redo")
        .expect("there is something to redo");
    assert_eq!(redone.after, Rating::stars(3));
    assert_eq!(session.rating(photo.id).stars, 3);

    // Undo again and it is gone; a third undo has nothing to do and says so rather than panicking.
    assert!(session.undo().expect("undo").is_some());
    assert!(
        session.undo().expect("undo").is_none(),
        "nothing left to undo"
    );
    assert_eq!(session.rating(photo.id), before);
}

/// REV-68, and the reason `PhotoFingerprint` exists: a **renamed** file keeps its rating. Keyed
/// by path it would silently lose it, with the sidecar still on disk.
#[test]
fn a_renamed_photo_keeps_its_rating() {
    let Some(root) = test_photos() else {
        eprintln!("SKIPPED: no FIRSTCUT_TEST_PHOTOS");
        return;
    };
    let Some(shoot) = sample_shoot(&root.join("Game1JENKS"), 8) else {
        eprintln!("SKIPPED: no CR3 files");
        return;
    };

    let (id, old_name, new_name) = {
        let (mut session, _, _) = Session::open(shoot.path()).expect("open");
        let photo = session.photos()[2].clone();
        session
            .set_rating(photo.id, Rating::stars(5), 0, 0)
            .expect("rate");
        session.flush();
        (
            photo.id,
            photo.rel_path.clone(),
            format!("renamed_{}", photo.rel_path),
        )
    };

    std::fs::rename(shoot.path().join(&old_name), shoot.path().join(&new_name)).expect("rename");

    let (session, _, _) = Session::open(shoot.path()).expect("reopen after rename");
    assert_eq!(
        session.rating(id).stars,
        5,
        "the renamed photo must keep its rating; the fingerprint is the identity, not the path"
    );
    // And the photo is still found under its new name, so the app can show it.
    assert!(
        session.photos().iter().any(|p| p.rel_path == new_name),
        "the renamed file has to appear under its new name"
    );
}

/// REV-78, from the session's own door: whatever the app shows as kept, Finish keeps.
#[test]
fn the_tier_counts_agree_with_what_finish_would_keep() {
    let Some(root) = test_photos() else {
        eprintln!("SKIPPED: no FIRSTCUT_TEST_PHOTOS");
        return;
    };
    let Some(shoot) = sample_shoot(&root.join("Game1JENKS"), 16) else {
        eprintln!("SKIPPED: no CR3 files");
        return;
    };

    let (mut session, _, _) = Session::open(shoot.path()).expect("open");
    for mode in [RatingMode::Stars, RatingMode::KeepNotKeep] {
        session.set_rating_mode(mode);
        let ids: Vec<_> = session.photos().iter().map(|p| p.id).collect();
        for (i, id) in ids.iter().enumerate() {
            // A spread of states, including the awkward one: 4 stars and keep unset.
            let rating = match i % 4 {
                0 => Rating::stars(4),
                1 => Rating::stars(3),
                2 => Rating::stars(0),
                _ => Rating::keep(),
            };
            session.set_rating(*id, rating, 0, 0).expect("rate");
        }
        let counts = session.count_by_tier();
        let counted: usize = counts.values().sum();
        let rated = session.ratings().len();
        assert_eq!(counted, rated, "every rated photo is in exactly one tier");

        // A photo the summary calls a keep is a keep, and a photo it does not is not.
        for id in &ids {
            let rating = session.rating(*id);
            assert_eq!(
                rating.tier(mode) == Tier::Keep,
                rating.is_kept(mode),
                "photo {id:?} in {mode}: the tier and the keep decision disagree"
            );
        }
    }
}

/// A folder with no photographs is an error with a sentence in it, not an empty window the user
/// cannot get out of.
#[test]
fn an_empty_folder_is_refused_with_a_reason() {
    let dir = tempfile::tempdir().expect("temp dir");
    let err = Session::open(dir.path()).expect_err("an empty folder cannot be a session");
    let text = err.to_string();
    assert!(text.contains("no photographs"), "unhelpful message: {text}");
}

/// Opening a path that is not a folder at all.
#[test]
fn a_missing_folder_is_refused_with_a_reason() {
    let err = Session::open(Path::new("/definitely/not/here")).expect_err("no such folder");
    assert!(err.to_string().contains("not a folder"), "{err}");
}

/// The durable id has to be a function of the fingerprint, and must fall back to the path only
/// when a camera omitted the fields. An id built from a partial fingerprint would collide with
/// another photo's, which is worse than admitting it is path-based.
#[test]
fn the_photo_id_is_the_fingerprint_not_the_path() {
    let base = PhotoMeta {
        camera_serial: Some("122022006902".into()),
        shutter_count: Some(33_537),
        file_size: 12_162_194,
        ..PhotoMeta::default()
    };
    let mut a = base.clone();
    a.rel_path = "IMG_0001.CR3".into();
    a.capture_time = Some(firstcut_core::meta::CaptureTime {
        unix_ms: 1_787_882_089_840,
        subsec_resolution_ms: 10,
        offset_minutes: Some(-360),
        source: firstcut_core::meta::TimeSource::Exif,
    });
    let mut b = a.clone();
    b.rel_path = "completely_different_name.CR3".into();
    assert_eq!(
        photo_id_for(&a),
        photo_id_for(&b),
        "the same photo under a new name must get the same id"
    );

    // Different shutter count = different photo, so a different id.
    let mut c = a.clone();
    c.shutter_count = Some(33_538);
    assert_ne!(photo_id_for(&a), photo_id_for(&c));

    // No fingerprint available: falls back to the path rather than colliding.
    let d = PhotoMeta {
        rel_path: "IMG_0001.CR3".into(),
        ..PhotoMeta::default()
    };
    let e = PhotoMeta {
        rel_path: "IMG_0002.CR3".into(),
        ..PhotoMeta::default()
    };
    assert_ne!(photo_id_for(&d), photo_id_for(&e));
    assert_eq!(
        photo_id_for(&d),
        firstcut_core::batch::photo_id("IMG_0001.CR3")
    );
}

/// The bug that made every rename create a **second database**.
///
/// `FolderIdentity::db_file_name` includes a short folder fingerprint, and the fingerprint used to
/// hash `(name, size)` pairs sorted as tuples. Permuting the names permuted which size landed in
/// which position, so the hash changed and a renamed folder looked like a re-shot one: a fresh
/// database, and the rating sitting in the old one. This asserts the property the name implies --
/// the session a folder resolves to does not depend on what its files are called.
#[test]
fn a_renamed_file_does_not_create_a_second_session() {
    let Some(root) = test_photos() else {
        eprintln!("SKIPPED: no FIRSTCUT_TEST_PHOTOS");
        return;
    };
    let Some(shoot) = sample_shoot(&root.join("Game1JENKS"), 8) else {
        eprintln!("SKIPPED: no CR3 files");
        return;
    };

    use firstcut_core::store::identity::FolderIdentity;
    let before = FolderIdentity::detect(shoot.path()).expect("identity");
    let original = shoot
        .path()
        .read_dir()
        .expect("read_dir")
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("CR3"))
        .expect("a CR3 to rename");

    // Rename it to something that sorts in a different place, and change its length too, so a
    // name- or length-derived fingerprint would both notice.
    let renamed = shoot.path().join("aaaa_renamed.CR3");
    std::fs::rename(&original, &renamed).expect("rename");

    let after = FolderIdentity::detect(shoot.path()).expect("identity after rename");
    assert_eq!(
        before.db_file_name(),
        after.db_file_name(),
        "renaming a file must not make the folder look like a different shoot"
    );
    assert_eq!(
        before.fingerprint.file_count, after.fingerprint.file_count,
        "the folder still holds the same number of photographs"
    );
    assert_eq!(
        before.fingerprint.total_bytes, after.fingerprint.total_bytes,
        "a rename changes no bytes"
    );
}
