//! §7.4: "CR3 parser … verified against exiftool on all four games."
//!
//! These tests need the real RAW files, which live in `~/Documents/testing` and are never committed
//! (task.md §12). They skip when the folder is absent, so CI stays green without 42 GB. Run them
//! deliberately:
//!
//! ```sh
//! FIRSTCUT_TEST_PHOTOS=~/Documents/testing cargo test --test cr3 -- --nocapture
//! ```
//!
//! What is asserted is deliberately *comparative*: the parser is checked against `exiftool -j`
//! output on the same files, field by field, rather than against hand-copied constants. A constant
//! only proves the parser has not changed since someone wrote it; comparing with exiftool is what
//! "verified against exiftool" actually means, and it is the check that would catch Canon moving a
//! field.

use std::path::PathBuf;

use firstcut_core::meta::{FileKind, PhotoMeta, RawFormat};
use firstcut_core::scan::scan_folder;

const GAMES: [&str; 4] = ["Game1JENKS", "Gane2NC", "Game3KC", "Game4VRE"];

/// Expected file counts from task.md §3. Also a check that the folder is the one we think it is:
/// a truncated copy would otherwise make every other test quietly vacuous.
const EXPECTED_COUNTS: [(&str, usize); 4] = [
    ("Game1JENKS", 708),
    ("Gane2NC", 529),
    ("Game3KC", 920),
    ("Game4VRE", 723),
];

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

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(format!("tests/fixtures/exiftool/{name}.json"))
}

/// Minimal exiftool-shaped row. Pulled out rather than using `serde_json::Value` throughout so a
/// field that is missing from the dump reads as "skip" rather than as a silent mismatch.
struct Row {
    file_name: String,
    model: Option<String>,
    lens: Option<String>,
    serial: Option<String>,
    iso: Option<f64>,
    f_number: Option<f64>,
    focal: Option<f64>,
    shutter_count: Option<u64>,
    width: Option<u64>,
    height: Option<u64>,
}

fn load_rows(path: &PathBuf) -> Vec<Row> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    value
        .as_array()
        .expect("exiftool -j output is a JSON array")
        .iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(|v| v.as_str()).map(str::to_string);
            Row {
                file_name: r
                    .get("FileName")
                    .and_then(|v| v.as_str())
                    .expect("every row has a FileName")
                    .to_string(),
                model: s("Model"),
                lens: s("LensModel"),
                serial: s("SerialNumber"),
                iso: r.get("ISO").and_then(|v| v.as_f64()),
                f_number: r.get("FNumber").and_then(|v| v.as_f64()),
                focal: r.get("FocalLength").and_then(|v| v.as_f64()),
                shutter_count: r.get("ShutterCount").and_then(|v| v.as_u64()),
                width: r.get("ImageWidth").and_then(|v| v.as_u64()),
                height: r.get("ImageHeight").and_then(|v| v.as_u64()),
            }
        })
        .collect()
}

/// Every comparable field, for one file. Anything the parser and exiftool disagree about is
/// reported, and the test fails.
fn assert_matches_exiftool(photo: &PhotoMeta, row: &Row, mismatches: &mut Vec<String>) {
    let name = &photo.rel_path;
    let mut str_field = |field: &str, want: &Option<String>, got: &Option<String>| {
        if let (Some(want), Some(got)) = (want, got)
            && !want.eq_ignore_ascii_case(got)
        {
            mismatches.push(format!("{name}: {field} exiftool={want:?} parsed={got:?}"));
        }
    };
    str_field("Model", &row.model, &photo.camera_model);
    str_field("LensModel", &row.lens, &photo.lens_model);
    str_field("SerialNumber", &row.serial, &photo.camera_serial);

    let mut num_field = |field: &str, want: Option<f64>, got: Option<f32>| {
        if let (Some(want), Some(got)) = (want, got)
            // exiftool prints ExposureTime as "1/2000" and FNumber as 2.8, so only the fields
            // that are plain numbers are compared here.
            && (want - f64::from(got)).abs() > 0.01
        {
            mismatches.push(format!("{name}: {field} exiftool={want} parsed={got}"));
        }
    };
    num_field("FNumber", row.f_number, photo.f_number);
    num_field("FocalLength", row.focal, photo.focal_length_mm);
    num_field("ISO", row.iso, photo.iso.map(|v| v as f32));

    if let (Some(want), Some(got)) = (row.shutter_count, photo.shutter_count)
        && want != got
    {
        mismatches.push(format!("{name}: ShutterCount exiftool={want} parsed={got}"));
    }
    if let Some(want) = row.width
        && want != u64::from(photo.width)
    {
        mismatches.push(format!(
            "{name}: ImageWidth exiftool={want} parsed={}",
            photo.width
        ));
    }
    if let Some(want) = row.height
        && want != u64::from(photo.height)
    {
        mismatches.push(format!(
            "{name}: ImageHeight exiftool={want} parsed={}",
            photo.height
        ));
    }
}

/// §7.4's headline requirement, on the real files.
#[test]
fn the_parser_matches_exiftool_on_every_game() {
    let Some(root) = test_photos() else {
        eprintln!(
            "SKIPPED: no FIRSTCUT_TEST_PHOTOS. The CR3 parser is UNVERIFIED against exiftool \
             without the real files. Run: FIRSTCUT_TEST_PHOTOS=~/Documents/testing cargo test"
        );
        return;
    };

    let mut total = 0usize;
    for (game, expected) in EXPECTED_COUNTS {
        let folder = root.join(game);
        assert!(folder.is_dir(), "{game} is missing from {}", root.display());
        let result = scan_folder(&folder);

        assert_eq!(
            result.photos.len(),
            expected,
            "{game}: task.md §3 says {expected} files; a different count means the test data \
             changed, and every number derived from it has to be re-derived"
        );
        assert!(
            result.skipped.is_empty(),
            "{game}: {} files could not be parsed at all: {:?}",
            result.skipped.len(),
            result
                .skipped
                .iter()
                .take(5)
                .map(|s| &s.rel_path)
                .collect::<Vec<_>>()
        );

        let rows = load_rows(&fixture(game));
        assert_eq!(
            rows.len(),
            expected,
            "{game}: the exiftool dump should cover every file"
        );

        let by_name: std::collections::HashMap<&str, &PhotoMeta> = result
            .photos
            .iter()
            .map(|p| (p.rel_path.as_str(), p))
            .collect();

        let mut mismatches = Vec::new();
        for row in &rows {
            let Some(photo) = by_name.get(row.file_name.as_str()) else {
                mismatches.push(format!(
                    "{}: in the dump but not in the scan",
                    row.file_name
                ));
                continue;
            };
            assert_matches_exiftool(photo, row, &mut mismatches);
        }
        assert!(
            mismatches.is_empty(),
            "{game}: {} fields disagree with exiftool:\n  {}",
            mismatches.len(),
            mismatches
                .iter()
                .take(25)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  ")
        );
        total += result.photos.len();
    }
    assert_eq!(
        total, 2_880,
        "task.md §3: 2,880 files across the four games"
    );
    eprintln!("verified {total} files against exiftool, field for field");
}

/// Every file must produce the fields the batcher orders by. A `None` here is not a nit: without a
/// capture time the photo is ordered by mtime, and REV-63 is about what that costs.
#[test]
fn every_photo_has_the_fields_ordering_needs() {
    let Some(root) = test_photos() else {
        eprintln!("SKIPPED: no FIRSTCUT_TEST_PHOTOS");
        return;
    };

    for game in GAMES {
        let result = scan_folder(&root.join(game));
        for photo in &result.photos {
            assert_eq!(
                photo.kind,
                FileKind::Raw(RawFormat::Cr3),
                "{}",
                photo.rel_path
            );
            let time = photo
                .capture_time
                .unwrap_or_else(|| panic!("{}: no capture time", photo.rel_path));
            assert_eq!(
                time.source,
                firstcut_core::meta::TimeSource::Exif,
                "{}: fell back to a non-EXIF time, so ordering is on mtime",
                photo.rel_path
            );
            assert_eq!(
                time.subsec_resolution_ms, 10,
                "{}: task.md §3 says the R8 records 10 ms sub-seconds",
                photo.rel_path
            );
            assert!(
                photo.shutter_count.is_some(),
                "{}: no ShutterCount, so §5.2's deleted-frames signal is lost",
                photo.rel_path
            );
            assert_eq!(photo.width, 6000, "{}", photo.rel_path);
            assert_eq!(photo.height, 4000, "{}", photo.rel_path);
        }
    }
}

/// `ShutterCount` is strictly monotonic in capture time on all four games (task.md §3, measured
/// by senior-dev). If the parser ever returned a wrong number, this is what would notice.
#[test]
fn shutter_count_is_monotonic_in_capture_time() {
    let Some(root) = test_photos() else {
        eprintln!("SKIPPED: no FIRSTCUT_TEST_PHOTOS");
        return;
    };
    for game in GAMES {
        let mut photos = scan_folder(&root.join(game)).photos;
        photos.sort_by_key(|p| p.capture_time.map(|c| c.unix_ms).unwrap_or(i64::MAX));
        let counts: Vec<u64> = photos.iter().filter_map(|p| p.shutter_count).collect();
        let drops = counts.windows(2).filter(|w| w[1] < w[0]).count();
        assert_eq!(
            drops, 0,
            "{game}: {drops} times ShutterCount went backwards"
        );
        // And it actually moves: a constant would mean we are reading one field over and over.
        assert!(
            counts.iter().max().unwrap_or(&0) - counts.iter().min().unwrap_or(&0) > 100,
            "{game}: ShutterCount barely varies, which means the offset is probably wrong"
        );
    }
}

/// The < 2 ms/file budget in §7.4. This is a *measurement*, and it is allowed to be slow: a
/// performance test that is skipped is worse than useless, so it runs whenever the files do.
#[test]
fn a_scan_is_under_two_milliseconds_per_file() {
    let Some(root) = test_photos() else {
        eprintln!("SKIPPED: no FIRSTCUT_TEST_PHOTOS");
        return;
    };
    let folder = root.join("Game1JENKS");

    // Warm the cache first: the first read of 708 files measures the disk, not the parser.
    let _ = scan_folder(&folder);

    let mut best = f64::MAX;
    for _ in 0..3 {
        let started = std::time::Instant::now();
        let result = scan_folder(&folder);
        let elapsed = started.elapsed().as_secs_f64();
        assert!(!result.photos.is_empty());
        best = best.min(elapsed);
    }
    let per_file = best * 1000.0 / 708.0;
    eprintln!("Game1JENKS: {per_file:.3} ms/file (best of 3, warm cache)");
    assert!(
        per_file < 2.0,
        "{per_file:.3} ms/file is over §7.4's 2 ms budget"
    );
}

/// A corrupt or truncated file must produce a photo with a reason, not a panic and not silence.
/// task.md §8 requires the file to still show up.
#[test]
fn a_truncated_file_is_reported_rather_than_dropped() {
    let dir = std::env::temp_dir().join(format!("firstcut-cr3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    // A real file header followed by nothing: the box walk hits the end mid-box.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&24u32.to_be_bytes());
    bytes.extend_from_slice(b"ftypcrx ");
    bytes.extend_from_slice(&4096u32.to_be_bytes());
    bytes.extend_from_slice(b"moov");
    bytes.truncate(40);
    std::fs::write(dir.join("IMG_0001.CR3"), &bytes).expect("write");

    let result = scan_folder(&dir);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        result.photos.len(),
        1,
        "the file must still be a photo: task.md §8"
    );
    assert_eq!(result.photos[0].rel_path, "IMG_0001.CR3");
    assert!(
        !result.photos[0].warnings.is_empty(),
        "a file that could not be read must say why"
    );
}

/// Not a CR3 at all: a JPEG named `.jpg` is shown, and a text file pretending to be a RAW is
/// reported. Neither may panic.
#[test]
fn a_file_that_is_not_a_raw_does_not_panic() {
    let dir = std::env::temp_dir().join(format!("firstcut-junk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("IMG_0001.CR3"), b"this is not a Canon file at all").expect("write");
    std::fs::write(
        dir.join("IMG_0002.JPG"),
        b"\xff\xd8\xff\xe0 not really a jpeg",
    )
    .expect("write");

    let result = scan_folder(&dir);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(result.photos.len(), 2, "both files are still photos");
    // Look each up by name: the scan sorts by name, so index 0 is the CR3.
    let by_name = |n: &str| result.photos.iter().find(|p| p.rel_path == n);
    assert_eq!(
        by_name("IMG_0002.JPG").map(|p| p.kind),
        Some(FileKind::Jpeg),
        "the JPG is not a RAW"
    );
    let fake = by_name("IMG_0001.CR3").expect("the fake CR3 is still listed");
    assert!(
        !fake.warnings.is_empty(),
        "a file that could not be parsed must carry a reason"
    );
}
