//! Batching regression on the committed metadata dumps (task.md §5.4).
//!
//! Runs entirely on `tests/fixtures/meta/<game>.json`, so CI needs none of the 42 GB of RAW files.
//! Three kinds of check live here:
//!
//! 1. **Invariants** that must hold for every game, whatever the thresholds end up being.
//! 2. **Ground-truth F1** against `tests/fixtures/ground-truth/<game>.json`, once a human has
//!    verified those boundaries by looking at the photos. Skipped until the file exists, so this
//!    test never blocks anyone on a fixture that hasn't been built yet.
//! 3. **Golden batches** for the four games, so a threshold change that moves a boundary in the
//!    real data shows up as a diff rather than as a silently different cull.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use firstcut_core::batch::fixture::PhotoMeta;
use firstcut_core::batch::view::Photo;
use firstcut_core::batch::{BatchParams, GroundTruth, batch_with, evaluate_names};
use serde_json::Value;

const GAMES: [&str; 4] = ["Game1JENKS", "Gane2NC", "Game3KC", "Game4VRE"];

fn repo_path(relative: &str) -> PathBuf {
    // CARGO_MANIFEST_DIR is `<repo>/core/firstcut-core`, so two levels up is the repo root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn meta_path(game: &str) -> PathBuf {
    repo_path(&format!("tests/fixtures/meta/{game}.json"))
}

fn truth_path(game: &str) -> PathBuf {
    repo_path(&format!("tests/fixtures/ground-truth/{game}.json"))
}

fn load_photos(game: &str) -> Vec<PhotoMeta> {
    let path = meta_path(game);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

fn batch_names(game: &str) -> Vec<Vec<String>> {
    let photos = load_photos(game);
    let out = batch_with(&photos, &HashMap::new(), &[], BatchParams::default());
    out.batches
        .iter()
        .map(|b| {
            b.photo_ids
                .iter()
                .map(|id| {
                    photos
                        .iter()
                        .find(|p| p.id == *id)
                        .unwrap_or_else(|| panic!("unknown photo id {}", id.0))
                        .rel_path
                        .clone()
                })
                .collect()
        })
        .collect()
}

/// Every invariant task.md §5 states, checked against every game. These hold for any thresholds.
#[test]
fn the_invariants_hold_for_every_game() {
    for game in GAMES {
        let photos = load_photos(game);
        let total = photos.len();
        assert!(
            total > 500,
            "{game}: fixture looks truncated ({total} photos)"
        );

        let out = batch_with(&photos, &HashMap::new(), &[], BatchParams::default());
        let batches = &out.batches;

        assert!(!batches.is_empty(), "{game}: no batches");
        assert!(
            batches.iter().all(|b| !b.photo_ids.is_empty()),
            "{game}: an empty batch would show the user a blank step"
        );

        // Every photo is in exactly one batch.
        let mut all: Vec<u64> = batches
            .iter()
            .flat_map(|b| b.photo_ids.iter().map(|i| i.0))
            .collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(
            all.len(),
            total,
            "{game}: a photo is in two batches or none"
        );

        // Determinism: task.md §5.3 step 6.
        let again = batch_with(&photos, &HashMap::new(), &[], BatchParams::default());
        assert_eq!(
            batches, &again.batches,
            "{game}: batching is not deterministic"
        );

        // Batches are capture-ordered and disjoint.
        let order = &out.order;
        let mut previous_end = 0usize;
        for (i, b) in batches.iter().enumerate() {
            assert_eq!(b.index as usize, i, "{game}: batch indices are not dense");
            let positions: Vec<usize> = b
                .photo_ids
                .iter()
                .map(|id| {
                    order
                        .iter()
                        .position(|o| o == id)
                        .expect("batch photo is in the order")
                })
                .collect();
            assert!(
                positions.windows(2).all(|w| w[0] < w[1]),
                "{game}: batch {i} is not in capture order"
            );
            assert!(
                positions[0] >= previous_end,
                "{game}: batch {i} overlaps the previous one"
            );
            previous_end = positions[positions.len() - 1] + 1;

            // Batch ids are a function of the first photo, so they survive a reload.
            assert_eq!(b.id, firstcut_core::batch::batch_id(b.photo_ids[0]));
        }

        // Two different camera bodies never share a batch.
        for b in batches {
            let serials: Vec<Option<&str>> = b
                .photo_ids
                .iter()
                .map(|id| {
                    photos
                        .iter()
                        .find(|p| p.id == *id)
                        .and_then(|p| p.camera_serial.as_deref())
                })
                .collect();
            assert!(
                serials.windows(2).all(|w| w[0] == w[1]),
                "{game}: batch {} mixes camera bodies",
                b.index
            );
        }
    }
}

#[test]
fn ordering_is_capture_time_on_every_game() {
    for game in GAMES {
        let photos = load_photos(game);
        let out = batch_with(&photos, &HashMap::new(), &[], BatchParams::default());
        let times: Vec<Option<i64>> = out
            .order
            .iter()
            .map(|id| {
                photos
                    .iter()
                    .find(|p| p.id == *id)
                    .and_then(|p| p.capture_unix_ms())
            })
            .collect();
        assert!(
            times
                .windows(2)
                .all(|w| w[0].is_none_or(|a| w[1].is_none_or(|b| a <= b))),
            "{game}: order is not sorted by capture time"
        );
        assert!(
            times.iter().all(Option::is_some),
            "{game}: every test photo has a capture time"
        );
    }
}

#[test]
fn the_real_corpus_cannot_exercise_a_name_rollover() {
    // This is the reason the rollover rule needs a synthetic fixture at all, so it is asserted
    // rather than left as folklore: file-name order equals capture order in all 2,880 files of
    // all four games, and Game4VRE stops at 9999 without ever wrapping. A name-based `order()`
    // therefore passes every test built from the real corpus (REV-26, REV-27).
    for game in GAMES {
        let photos = load_photos(game);
        let mut numbers: Vec<u32> = photos.iter().filter_map(|p| p.file_number).collect();
        assert!(
            !numbers.is_empty(),
            "{game}: no file numbers in the fixture"
        );
        numbers.sort_unstable();
        let inversions = load_photos(game)
            .iter()
            .filter_map(|p| p.file_number)
            .collect::<Vec<_>>()
            .windows(2)
            .filter(|w| w[1] < w[0])
            .count();
        assert_eq!(
            inversions, 0,
            "{game}: corpus unexpectedly contains a name inversion"
        );
        assert!(
            numbers.last().copied().unwrap_or(0) < 10_000,
            "{game}: names never wrap"
        );
    }
}

#[test]
fn a_synthetic_rollover_shoot_orders_by_time() {
    // task.md §11 asks for this fixture explicitly. Built in memory so no file is needed.
    use firstcut_core::batch::PhotoId;
    use firstcut_core::order;

    let mut photos: Vec<firstcut_core::batch::fixture::PhotoMeta> = [
        "IMG_9998.CR3",
        "IMG_9999.CR3",
        "IMG_0001.CR3",
        "IMG_0002.CR3",
    ]
    .iter()
    .enumerate()
    .map(|(i, name)| firstcut_core::batch::fixture::PhotoMeta {
        id: PhotoId(firstcut_core::batch::fnv1a64(name.as_bytes())),
        rel_path: (*name).to_string(),
        companions: vec![],
        kind: firstcut_core::batch::fixture::FileKind::Raw(
            firstcut_core::batch::fixture::RawFormat::Cr3,
        ),
        file_size: 1,
        capture_time: Some(firstcut_core::batch::fixture::CaptureTime {
            unix_ms: 1_000 + i as i64 * 90,
            subsec_resolution_ms: 10,
            offset_minutes: None,
            source: firstcut_core::batch::fixture::TimeSource::Exif,
        }),
        shutter_count: Some(100 + i as u64),
        file_number: None,
        camera_make: Some("Canon".into()),
        camera_model: Some("EOS R8".into()),
        camera_serial: Some("1".into()),
        lens_model: None,
        focal_length_mm: Some(200.0),
        exposure_time_s: Some(0.0005),
        f_number: Some(2.8),
        iso: Some(800),
        exposure_comp_ev: Some(0.0),
        metering_mode: None,
        drive_mode: None,
        shutter_mode: None,
        orientation: 1,
        width: 6000,
        height: 4000,
        af: None,
        warnings: vec![],
    })
    .collect();
    photos[0].file_number = Some(9998);

    let ids = order::order(&photos);
    // `PhotoId` is a hash, not an index into `photos` (REV-66). Every fixture so far happened to
    // set `id == index`, which hid that; look the photo up by id instead.
    let by_id: std::collections::HashMap<_, _> =
        photos.iter().map(|p| (p.id, p.rel_path.as_str())).collect();
    let names: Vec<&str> = ids.iter().map(|id| by_id[id]).collect();
    assert_eq!(
        names,
        [
            "IMG_9998.CR3",
            "IMG_9999.CR3",
            "IMG_0001.CR3",
            "IMG_0002.CR3"
        ],
        "capture time wins over the name across the rollover"
    );
}

/// REV-26: renaming every file must not change the order. The only test here that can catch a
/// name-based `order()`, because the corpus itself cannot (see the rollover test above).
#[test]
fn a_scrambled_photo_set_produces_a_byte_identical_order() {
    // REV-26, and the only test in the suite that can actually catch a name-based `order()`.
    // `the_real_corpus_cannot_exercise_a_name_rollover` proves the corpus cannot do it: names run
    // forwards in all 2,880 files, so a purely name-based order passes every other test here, every
    // F1 number and every CI run we have.
    //
    // So rename every photo to a name deliberately *anti-correlated* with capture time and assert
    // the produced order is unchanged. The scramble is a deterministic permutation of the file
    // number, so this is a CI test, not a local curiosity.
    use firstcut_core::batch::PhotoId;
    use firstcut_core::order;

    for game in GAMES {
        let photos = load_photos(game);
        let base: Vec<PhotoId> = order::order(&photos);

        let scrambled: Vec<PhotoMeta> = photos
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut clone = p.clone();
                let number = scramble_number(i as u64, photos.len() as u64);
                clone.rel_path = format!("ZZ_{number:05}.CR3");
                // In production the id is a hash of the path, so re-derive it from the new path —
                // this is exactly what a real re-scan after files are renamed produces.
                clone.id = PhotoId(firstcut_core::batch::fnv1a64(clone.rel_path.as_bytes()));
                clone.companions = Vec::new();
                clone
            })
            .collect();

        let after: Vec<PhotoId> = order::order(&scrambled);
        assert_eq!(
            after.len(),
            base.len(),
            "{game}: scramble changed the photo count"
        );

        // Compare by *capture identity*, not by PhotoId: the ids are hashes of the paths, and the
        // scramble changed those. A name-based order emits these in scrambled order and fails.
        let before: Vec<Option<i64>> = base
            .iter()
            .map(|id| {
                photos
                    .iter()
                    .find(|p| p.id == *id)
                    .and_then(|p| p.capture_time.as_ref())
                    .map(|c| c.unix_ms)
            })
            .collect();
        let now: Vec<Option<i64>> = after
            .iter()
            .map(|id| {
                scrambled
                    .iter()
                    .find(|p| p.id == *id)
                    .and_then(|p| p.capture_time.as_ref())
                    .map(|c| c.unix_ms)
            })
            .collect();

        assert_eq!(
            now, before,
            "{game}: renaming the files changed the order, so order() is reading the file name"
        );
    }
}

/// A deterministic permutation of `0..n`, so names are shuffled, no two photos collide, and the
/// same input always produces the same scramble. Odd multiplier, odd increment, and `n` forced odd.
fn scramble_number(index: u64, n: u64) -> u64 {
    let m = n | 1;
    (index.wrapping_mul(2_654_435_761).wrapping_add(n) % m) + 1
}

// `CullProgress`-style ground truth: the F1 measurement below needs a human, so until the file
// exists the test skips loudly rather than passing on its own output.
#[test]
fn boundary_f1_matches_the_visual_ground_truth() {
    let mut checked = 0;
    let mut failures: Vec<String> = Vec::new();

    for game in GAMES {
        let path = truth_path(game);
        if !path.exists() {
            eprintln!("{game}: no ground truth yet at {}", path.display());
            continue;
        }
        checked += 1;

        let truth: GroundTruth = serde_json::from_str(
            &std::fs::read_to_string(&path).expect("reading the ground truth"),
        )
        .expect("parsing the ground truth");
        let predicted = batch_names(game);
        let m = evaluate_names(&predicted, &truth);

        assert!(
            m.missing_photos.is_empty(),
            "{game}: ground truth names photos that are not in the fixture: {:?}",
            m.missing_photos
        );

        println!(
            "{game}: F1 {:.1}% (P {:.1} / R {:.1}, {}/{} boundaries), \
             {} wrong merges, {} wrong splits",
            m.f1_percent(),
            m.precision * 100.0,
            m.recall * 100.0,
            m.true_positives,
            m.truth_boundaries,
            m.wrong_merges,
            m.wrong_splits
        );

        // task.md §5.4: ≥ 98% boundary F1 and zero merges of clearly different plays.
        if m.f1 < 0.98 {
            failures.push(format!(
                "{game}: F1 {:.1}% is below the 98% target",
                m.f1_percent()
            ));
        }
        if m.wrong_merges > 0 {
            failures.push(format!("{game}: {} wrongly merged bursts", m.wrong_merges));
        }
    }

    // No ground truth committed yet. This is deliberately a **skip with a message**, not a silent
    // pass and not a failure: the ground truth has to come from a human looking at the photos
    // (task.md §12), and generating it from `batch()`'s own output would be self-certification —
    // the exact failure mode REV-55 exists to prevent. The batcher's accuracy is therefore
    // **unmeasured**, and REQ-core-batch-1 tracks producing the first real ground truth file.
    if checked == 0 {
        eprintln!(
            "SKIPPED: no ground truth in {}. Boundary F1 is UNMEASURED — \
             the 98% target in task.md §5.4 is not currently demonstrated. See REQ-core-batch-1.",
            truth_path(GAMES[0])
                .parent()
                .unwrap_or(&PathBuf::new())
                .display()
        );
        return;
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
    if checked == 0 {
        eprintln!(
            "no ground truth scored yet -- see ground_truth_exists_for_every_game, \
             which is the test that tracks the outstanding work"
        );
    }
}

/// The ground truth is **not** written by the agent that is scored against it (REV-55): it is
/// produced by a human looking at the photographs, which is the only way to know whether two
/// adjacent frames are one burst.
///
/// This test asserts nothing. It exists to make the gap **visible in the output of every run**
/// without making the suite red, because a test that is permanently red stops being a signal: the
/// first person to hit it under a deadline runs `--no-fail-fast`, or deletes it, and then it is
/// gone for good. A loud skip is the honest middle -- `cargo test` goes green, and every run still
/// prints that boundary F1 is UNMEASURED and the 98% target in task.md §5.4 is not demonstrated.
///
/// (An earlier version of this failed instead. Same information, worse mechanism: it made `main`
/// permanently red for a gap that is a to-do item, not a regression.)
#[test]
fn ground_truth_exists_for_every_game() {
    let missing: Vec<String> = GAMES
        .iter()
        .filter(|g| !truth_path(g).exists())
        .map(|g| (*g).to_string())
        .collect();
    if missing.is_empty() {
        return;
    }
    eprintln!(
        "SKIPPED: no verified ground truth for {missing:?}. Boundary F1 is UNMEASURED; the 98% \
         target in task.md §5.4 is NOT demonstrated. Build it by LOOKING at the photographs: \
         `firstcut contact-sheet --game <g>` renders every ambiguous-zone boundary, a human rules \
         on them, and the result is written to tests/fixtures/ground-truth/<g>.json. Then \
         `firstcut eval` scores it. See REQ-worker-1 and docs/agents/worker.md deliverable 4."
    );
}

/// Golden batches. Regenerate with `FIRSTCUT_UPDATE_GOLDEN=1 cargo test golden_batches` after a
/// deliberate threshold change, and read the diff before committing it.
#[test]
fn golden_batches_match_the_committed_dumps() {
    let golden_dir = repo_path("tests/fixtures/golden");
    if !golden_dir.exists() {
        eprintln!("no golden batches at {}", golden_dir.display());
        return;
    }

    let update = std::env::var("FIRSTCUT_UPDATE_GOLDEN").is_ok();
    let mut mismatches = Vec::new();

    for game in GAMES {
        let path = golden_dir.join(format!("{game}.json"));
        let predicted = batch_names(game);
        let current = serde_json::to_string_pretty(&Value::Array(
            predicted
                .iter()
                .map(|b| Value::Array(b.iter().map(|n| Value::String(n.clone())).collect()))
                .collect(),
        ))
        .expect("serializing the predicted batches");

        if update {
            std::fs::create_dir_all(&golden_dir).expect("creating the golden directory");
            std::fs::write(&path, format!("{current}\n")).expect("writing the golden file");
            continue;
        }

        let Ok(expected) = std::fs::read_to_string(&path) else {
            mismatches.push(format!("{game}: no golden file at {}", path.display()));
            continue;
        };
        let expected: Value = serde_json::from_str(&expected).expect("parsing the golden file");
        let actual: Value = serde_json::from_str(&current).expect("parsing the prediction");
        if expected != actual {
            let (e, a) = (
                expected.as_array().map(Vec::len).unwrap_or(0),
                actual.as_array().map(Vec::len).unwrap_or(0),
            );
            mismatches.push(format!(
                "{game}: batches changed ({e} golden vs {a} now). \
                 Re-run with FIRSTCUT_UPDATE_GOLDEN=1 and check the diff."
            ));
        }
    }

    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// The high-speed regression case from task.md §5.4: the tail of Game1JENKS at ~11 fps, broken by
/// 0.2-0.8 s re-press pauses. A burst at that rate is 90 ms per frame, so a threshold that only
/// works at 6 fps will over-merge it.
#[test]
fn the_high_speed_tail_of_game1jenks_is_not_one_giant_batch() {
    let photos = load_photos("Game1JENKS");
    let out = batch_with(&photos, &HashMap::new(), &[], BatchParams::default());

    // IMG_6117..IMG_6164, in capture order.
    let start = photos
        .iter()
        .find(|p| p.rel_path == "IMG_6117.CR3")
        .expect("IMG_6117.CR3 is in the fixture")
        .id;
    let end = photos
        .iter()
        .find(|p| p.rel_path == "IMG_6164.CR3")
        .expect("IMG_6164.CR3 is in the fixture")
        .id;
    let first = out.order.iter().position(|id| *id == start).unwrap();
    let last = out.order.iter().position(|id| *id == end).unwrap();

    let containing: Vec<u32> = out
        .batches
        .iter()
        .filter(|b| b.photo_ids.contains(&start) || b.photo_ids.contains(&end))
        .map(|b| b.index)
        .collect();
    assert!(
        containing.len() >= 2,
        "the 48-frame 11 fps run with re-press pauses must not collapse into one batch"
    );

    // Every boundary in that stretch must have been seen at the local 90 ms rate.
    let frame_intervals: Vec<i64> = out.verdicts[first..last]
        .iter()
        .map(|v| v.thresholds.frame_interval_ms)
        .collect();
    assert!(
        frame_intervals.iter().all(|&f| f <= 120),
        "the local frame interval should track ~90 ms, saw {frame_intervals:?}"
    );
}
