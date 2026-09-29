//! The CR3 parser checked against exiftool's own output, on the real photos.
//!
//! `tests/fixtures/exiftool/<game>.json` is what `exiftool -j` reports for all 2,880 test photos.
//! This test parses the same files with [`firstcut_core::meta::cr3`] and asserts the two agree, so
//! a field the batcher depends on — capture time, shutter count, the AF grid — is known to mean
//! what exiftool says it means and not just something plausible.
//!
//! The photos live outside the repo (`FIRSTCUT_TEST_PHOTOS`, default `~/Documents/testing`), so the
//! whole file skips when they are absent rather than failing CI that cannot see them. It is not a
//! mock: a parser that was never run against real Canon files would pass every other test here.
//!
//! The sample is a fixed stride rather than a random draw, so a failure names the same photos on
//! every run and a green run covers the same ground.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use firstcut_core::meta::cr3::Cr3;
use serde::Deserialize;

const GAMES: [&str; 4] = ["Game1JENKS", "Gane2NC", "Game3KC", "Game4VRE"];

/// How many photos per game to check. The task asks for at least 20; the stride below walks the
/// whole shoot evenly, so raising this widens coverage without changing which files are covered
/// at the low end.
const SAMPLE: usize = 60;
// senior-dev audit: full-corpus mode. `cargo test --test cr3_exiftool` uses the sample above;
// set this to 10000 to check every one of the 2,880 files.
fn full_corpus() -> bool {
    std::env::var("FIRSTCUT_CR3_FULL").is_ok()
}

/// One record of `exiftool -j`, restricted to the tags this parser claims to reproduce.
#[derive(Debug, Deserialize)]
struct ExifToolRecord {
    #[serde(rename = "FileName")]
    file_name: String,
    #[serde(rename = "SubSecDateTimeOriginal")]
    subsec_datetime_original: Option<String>,
    #[serde(rename = "Make")]
    make: Option<String>,
    #[serde(rename = "Model")]
    model: Option<String>,
    #[serde(rename = "SerialNumber")]
    serial_number: Option<serde_json::Value>,
    #[serde(rename = "LensModel")]
    lens_model: Option<String>,
    #[serde(rename = "FocalLength")]
    focal_length: Option<f32>,
    #[serde(rename = "ExposureTime")]
    exposure_time: Option<f32>,
    #[serde(rename = "FNumber")]
    f_number: Option<f32>,
    #[serde(rename = "ISO")]
    iso: Option<u32>,
    #[serde(rename = "ExposureCompensation")]
    exposure_compensation: Option<f32>,
    #[serde(rename = "MeteringMode")]
    metering_mode: Option<String>,
    #[serde(rename = "ContinuousDrive")]
    continuous_drive: Option<String>,
    #[serde(rename = "DriveMode")]
    drive_mode: Option<String>,
    #[serde(rename = "ShutterMode")]
    shutter_mode: Option<String>,
    #[serde(rename = "Orientation")]
    orientation: Option<u32>,
    #[serde(rename = "ImageWidth")]
    image_width: Option<u32>,
    #[serde(rename = "ImageHeight")]
    image_height: Option<u32>,
    #[serde(rename = "ShutterCount")]
    shutter_count: Option<u64>,
    #[serde(rename = "AFAreaMode")]
    af_area_mode: Option<String>,
    #[serde(rename = "AFPointsInFocus")]
    af_points_in_focus: Option<serde_json::Value>,
}

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// The folder holding the real photos, or `None` when this machine does not have them.
fn photos_root() -> Option<PathBuf> {
    let configured = std::env::var_os("FIRSTCUT_TEST_PHOTOS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("~/Documents/testing"));
    let expanded: PathBuf = configured
        .to_str()
        .map(|text| {
            if let Some(rest) = text.strip_prefix("~/") {
                match std::env::var_os("HOME") {
                    Some(home) => PathBuf::from(home).join(rest),
                    None => PathBuf::from(text),
                }
            } else {
                PathBuf::from(text)
            }
        })
        .unwrap_or(configured);
    expanded.is_dir().then_some(expanded)
}

fn load_exiftool(game: &str) -> Vec<ExifToolRecord> {
    let path = repo_path(&format!("tests/fixtures/exiftool/{game}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("reading {}: {err}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("parsing {}: {err}", path.display()))
}

/// Evenly spaced sample across the shoot. A stride reaches the first and last frames of a game,
/// which a short prefix would miss — and those are exactly the frames a rollover or a long pause
/// hides in.
fn sample<T>(items: &[T], count: usize) -> Vec<&T> {
    if items.len() <= count {
        return items.iter().collect();
    }
    let stride = items.len() / count;
    (0..count)
        .map(|i| &items[i * stride])
        .chain(std::iter::once(&items[items.len() - 1]))
        .collect()
}

/// Tally of how many files agreed, per field, so one failure says which field broke.
#[derive(Default)]
struct Tally {
    matched: usize,
    total: usize,
    /// The first few disagreements, so a red run is diagnosable without a debugger.
    failures: Vec<String>,
}

impl Tally {
    fn check(
        &mut self,
        field: &str,
        file: &str,
        got: impl std::fmt::Debug,
        want: impl std::fmt::Debug,
    ) {
        let got = format!("{got:?}");
        let want = format!("{want:?}");
        self.total += 1;
        if got == want {
            self.matched += 1;
        } else if self.failures.len() < 3 {
            self.failures
                .push(format!("{file} [{field}]: got {got}, exiftool says {want}"));
        }
    }

    fn finish(self, game: &str) {
        assert_eq!(
            self.matched,
            self.total,
            "{game}: {} of {} field comparisons disagreed with exiftool:\n  {}",
            self.total - self.matched,
            self.total,
            self.failures.join("\n  "),
        );
    }
}

/// exiftool prints `SerialNumber` as a bare number; the file holds it as an ASCII string.
fn serial_as_text(value: Option<serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(text) => Some(text),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `exiftool` prints a single AF point as a number and several as a comma-separated list, so both
/// spellings have to be reduced to one list of point indices before comparing.
fn af_points_as_list(value: Option<serde_json::Value>) -> Option<Vec<u16>> {
    // exiftool prints one point as a bare number and several as a comma-separated list, so both
    // spellings mean the same thing: a list of point indices. A bare `0` is point 0, not "none".
    let text = match value? {
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::String(text) => text,
        _ => return None,
    };
    Some(
        text.split(',')
            .filter_map(|part| part.trim().parse::<u16>().ok())
            .collect(),
    )
}

/// The capture instant, assembled the way `PhotoMeta` holds it: local wall clock plus sub-seconds,
/// and the offset stripped because it is a separate field.
fn capture_instant(record: &ExifToolRecord) -> Option<String> {
    let stamp = record.subsec_datetime_original.as_deref()?;
    // `2026:08:27 19:54:49.84-06:00` → `2026:08:27 19:54:49.84`
    let (body, offset) = match stamp.rfind(['+', '-']) {
        Some(at) if at > 0 => (&stamp[..at], Some(&stamp[at..])),
        _ => (stamp, None),
    };
    assert!(offset.is_some(), "every test photo carries a UTC offset");
    Some(body.to_string())
}

#[test]
fn the_cr3_parser_agrees_with_exiftool() {
    let Some(root) = photos_root() else {
        eprintln!(
            "skipping: no test photos. Set FIRSTCUT_TEST_PHOTOS or create ~/Documents/testing."
        );
        return;
    };

    for game in GAMES {
        let records = load_exiftool(game);
        let chosen = if full_corpus() {
            records.iter().collect()
        } else {
            sample(&records, SAMPLE)
        };
        assert!(
            chosen.len() >= 20,
            "{game}: the sample must cover at least 20 photos to be worth anything"
        );

        let mut tally = Tally::default();
        for record in &chosen {
            let path = root.join(game).join(&record.file_name);
            let parsed = Cr3::parse(&path).unwrap_or_else(|err| {
                panic!("{}: {err}", path.display());
            });

            tally.check("make", &record.file_name, &parsed.make, &record.make);
            tally.check("model", &record.file_name, &parsed.model, &record.model);
            tally.check(
                "body_serial_number",
                &record.file_name,
                &parsed.body_serial_number,
                serial_as_text(record.serial_number.clone()),
            );
            tally.check(
                "lens_model",
                &record.file_name,
                &parsed.lens_model,
                &record.lens_model,
            );
            tally.check(
                "focal_length_mm",
                &record.file_name,
                parsed.focal_length_mm,
                record.focal_length,
            );
            tally.check(
                "exposure_time_s",
                &record.file_name,
                parsed.exposure_time_s,
                record.exposure_time,
            );
            tally.check(
                "f_number",
                &record.file_name,
                parsed.f_number,
                record.f_number,
            );
            tally.check("iso", &record.file_name, parsed.iso, record.iso);
            tally.check(
                "exposure_comp_ev",
                &record.file_name,
                parsed.exposure_comp_ev,
                record.exposure_compensation,
            );
            tally.check(
                "metering_mode",
                &record.file_name,
                &parsed.metering_mode,
                &record.metering_mode,
            );
            // The contract prefers the continuous rate over the coarser DriveMode, so the
            // expectation is whichever of the two exiftool would have kept.
            let expected_drive = record
                .continuous_drive
                .clone()
                .or_else(|| record.drive_mode.clone());
            tally.check(
                "drive_mode",
                &record.file_name,
                &parsed.drive_mode,
                &expected_drive,
            );
            tally.check(
                "shutter_mode",
                &record.file_name,
                &parsed.shutter_mode,
                &record.shutter_mode,
            );
            tally.check(
                "orientation",
                &record.file_name,
                parsed.orientation.map(u32::from),
                record.orientation,
            );
            tally.check(
                "width",
                &record.file_name,
                (parsed.width > 0).then_some(parsed.width),
                record.image_width,
            );
            tally.check(
                "height",
                &record.file_name,
                (parsed.height > 0).then_some(parsed.height),
                record.image_height,
            );
            tally.check(
                "shutter_count",
                &record.file_name,
                parsed.shutter_count,
                record.shutter_count,
            );

            // Capture time arrives in two EXIF tags, so it is compared as the assembled instant
            // rather than field by field: the sub-second part is what orders frames 10 ms apart.
            let expected_capture = capture_instant(record);
            let actual_capture = parsed.date_time_original.as_ref().map(|stamp| {
                match parsed.subsec_time_original.as_deref() {
                    Some(subsec) => format!("{stamp}.{}", subsec.trim()),
                    None => stamp.clone(),
                }
            });
            tally.check(
                "capture_time",
                &record.file_name,
                &actual_capture,
                &expected_capture,
            );

            // The AF grid, which task.md §9.2 draws an overlay from.
            let af = parsed
                .af
                .as_ref()
                .unwrap_or_else(|| panic!("{}: no AF info", record.file_name));
            tally.check(
                "af_area_mode",
                &record.file_name,
                &af.area_mode,
                &record.af_area_mode,
            );
            // exiftool's fixture does not carry the AF image size, so it is compared against the
            // frame size it does carry: the AF grid is expressed over the sensor rectangle.
            tally.check(
                "af_image_width",
                &record.file_name,
                (af.image_width > 0).then_some(af.image_width),
                record.image_width,
            );
            tally.check(
                "af_image_height",
                &record.file_name,
                (af.image_height > 0).then_some(af.image_height),
                record.image_height,
            );
            let actual_points: Vec<u16> = af.points_in_focus.clone();
            let expected_points =
                af_points_as_list(record.af_points_in_focus.clone()).unwrap_or_default();
            tally.check(
                "af_points_in_focus",
                &record.file_name,
                &actual_points,
                &expected_points,
            );

            // The preview has to be a real, complete JPEG at a plausible size, or the pipeline
            // has nothing to decode.
            let preview = parsed
                .preview
                .as_ref()
                .unwrap_or_else(|| panic!("{}: no PRVW preview", record.file_name));
            assert!(
                preview.range.len > 10_000 && preview.range.offset > 0,
                "{}: preview range {:?} is not a JPEG",
                record.file_name,
                preview.range,
            );
            assert_eq!(
                preview.width, 1620,
                "{}: preview width changed",
                record.file_name,
            );
        }

        // Every field, every sampled photo. Reported per game so a regression names the shoot.
        tally.finish(game);
        eprintln!(
            "{game}: {} photos agreed with exiftool on every field",
            chosen.len()
        );
    }
}

#[test]
fn the_shutter_count_increases_across_a_whole_game() {
    // The batcher treats a shutter-count jump as frames deleted in camera (task.md §5.2), so a
    // count that is present but not monotonic would invent boundaries out of nothing. Checked
    // across every photo of one game rather than a sample: this is the one field whose *shape*
    // matters, not just its value on a given file.
    let Some(root) = photos_root() else {
        return;
    };
    let records = load_exiftool("Game1JENKS");
    let mut previous: Option<(String, u64)> = None;
    let mut checked = 0usize;
    let mut regressions: Vec<String> = Vec::new();

    // exiftool writes the fixture in capture order, which is the order the shutter count runs in.
    for record in &records {
        let path = root.join("Game1JENKS").join(&record.file_name);
        let Ok(parsed) = Cr3::parse(&path) else {
            continue;
        };
        let Some(count) = parsed.shutter_count else {
            continue;
        };
        if let Some((previous_name, previous_count)) = &previous
            && count < *previous_count
        {
            regressions.push(format!(
                "{previous_name}={previous_count} then {}={count}",
                record.file_name
            ));
        }
        previous = Some((record.file_name.clone(), count));
        checked += 1;
    }

    assert!(checked > 100, "only {checked} photos had a shutter count");
    assert!(
        regressions.is_empty(),
        "the shutter count must not go backwards in capture order: {}",
        regressions.join(", ")
    );
}

#[test]
fn a_whole_shoot_parses_and_orders_without_a_single_skip() {
    // task.md §8: never skip a file macOS can read. A CR3 that failed to parse here would be a
    // file the user can open in Preview and Firstcut cannot see.
    let Some(root) = photos_root() else {
        return;
    };
    for game in GAMES {
        let records = load_exiftool(game);
        let mut failures: HashMap<String, usize> = HashMap::new();
        let mut parsed_ok = 0usize;
        for record in &records {
            let path = root.join(game).join(&record.file_name);
            match Cr3::parse(&path) {
                Ok(_) => parsed_ok += 1,
                Err(err) => *failures.entry(err.to_string()).or_default() += 1,
            }
        }
        assert_eq!(
            parsed_ok,
            records.len(),
            "{game}: {} of {} files did not parse: {failures:?}",
            records.len() - parsed_ok,
            records.len(),
        );
    }
}
