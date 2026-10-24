//! `firstcut verify <folder>`: the core's metadata reader against `exiftool`, file by file.
//!
//! todo.md §8 asks for every parser field to be checked against exiftool. This is the check: scan
//! the folder exactly as the app does, ask exiftool the same questions about the same files, and
//! list every disagreement, grouped by extension so a format-wide gap reads as one line rather than
//! a hundred. It also lists the files the scanner did not pick up at all — an extension missing from
//! the table is otherwise invisible, because the scan simply never sees the file.
//!
//! Only the fields ordering and batching depend on are compared (capture time, camera, exposure,
//! orientation). Dimensions are left out: exiftool and the scanner disagree on purpose about whether
//! a RAW's size is the sensor's or the default crop's.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use firstcut_meta::meta::{self, PhotoMeta, TimeSource};
use serde_json::Value;

pub fn cmd_verify(args: &[String]) -> Result<(), String> {
    let mut folder = None;
    let mut quiet = false;
    let mut previews = false;
    for arg in args {
        match arg.as_str() {
            "--quiet" => quiet = true,
            "--previews" => previews = true,
            other if other.starts_with('-') => return Err(format!("unknown option `{other}`")),
            other => folder = Some(other.to_string()),
        }
    }
    let folder = folder.ok_or("pass a folder of photos")?;
    let root = Path::new(&folder);
    let scan = meta::scan_folder(root).map_err(|e| e.to_string())?;
    let truth = exiftool(root)?;

    let mut by_ext: BTreeMap<String, Tally> = BTreeMap::new();
    for photo in &scan.photos {
        let ext = extension(&photo.rel_path);
        let tally = by_ext.entry(ext).or_default();
        tally.files += 1;
        let problems = match truth.get(&photo.rel_path) {
            Some(tool) => compare(photo, tool),
            None => vec!["exiftool did not report this file".to_string()],
        };
        if photo.capture_time.as_ref().map(|t| t.source) == Some(TimeSource::Exif) {
            tally.exif_time += 1;
        }
        if photo.full_preview.is_some() || photo.preview.is_some() {
            tally.preview += 1;
        }
        if previews {
            let size = |p: &Option<meta::EmbeddedPreview>| {
                p.map_or("-".to_string(), |p| format!("{}x{}@{}+{}", p.width, p.height, p.range.offset, p.range.len))
            };
            println!("PREVIEW {} preview {} full {}", photo.rel_path, size(&photo.preview), size(&photo.full_preview));
        }
        if problems.is_empty() {
            tally.clean += 1;
        } else if !quiet {
            println!("{}", photo.rel_path);
            for problem in &problems {
                println!("    {problem}");
            }
        }
    }

    for skipped in &scan.skipped {
        println!("SKIPPED {}: {}", skipped.rel_path, skipped.reason);
    }
    let unscanned = unscanned_files(root, &scan)?;
    for path in &unscanned {
        println!("NOT SCANNED {path}");
    }

    println!();
    println!("ext     files  clean  exif-time  preview");
    for (ext, t) in &by_ext {
        println!("{ext:<7} {:>5}  {:>5}  {:>9}  {:>7}", t.files, t.clean, t.exif_time, t.preview);
    }
    let files: usize = by_ext.values().map(|t| t.files).sum();
    let clean: usize = by_ext.values().map(|t| t.clean).sum();
    println!("\n{clean} of {files} photos agree with exiftool; {} skipped, {} not scanned", scan.skipped.len(), unscanned.len());
    Ok(())
}

#[derive(Default)]
struct Tally {
    files: usize,
    clean: usize,
    exif_time: usize,
    preview: usize,
}

fn extension(rel_path: &str) -> String {
    rel_path.rsplit_once('.').map_or(String::new(), |(_, ext)| ext.to_ascii_lowercase())
}

/// `exiftool -j` over the folder, numeric (`#`) where the printed form is prose, keyed by the path relative to it.
fn exiftool(root: &Path) -> Result<BTreeMap<String, Value>, String> {
    let output = Command::new("exiftool")
        .args([
            "-j",
            "-r",
            "-q",
            "-q",
            "-api",
            "largefilesupport=1",
            "-DateTimeOriginal",
            "-SubSecTimeOriginal",
            "-Make",
            "-Model",
            // Not `#`: for Pentax the raw ISO tag is an index into a table (12 for ISO 400).
            "-ISO",
            "-ExposureTime#",
            "-FNumber#",
            "-FocalLength#",
            "-Orientation#",
        ])
        .arg(root)
        .output()
        .map_err(|e| format!("running exiftool: {e} (is it installed?)"))?;
    let rows: Vec<Value> = serde_json::from_slice(&output.stdout).map_err(|e| format!("reading exiftool's JSON: {e}"))?;
    let root = root.to_string_lossy();
    let prefix = format!("{}/", root.trim_end_matches('/'));
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let source = row.get("SourceFile")?.as_str()?;
            let rel = source.strip_prefix(&prefix).unwrap_or(source).to_string();
            Some((rel, row))
        })
        .collect())
}

/// Every disagreement between the scanner's record and exiftool's, in words.
fn compare(photo: &PhotoMeta, tool: &Value) -> Vec<String> {
    let mut problems = Vec::new();

    if let Some(stamp) = tool.get("DateTimeOriginal").and_then(Value::as_str) {
        match &photo.capture_time {
            Some(time) if time.source == TimeSource::Exif => {
                let local = time.unix_ms + i64::from(time.offset_minutes.unwrap_or(0)) * 60_000;
                let ours = exif_stamp(local);
                if ours != stamp.trim() {
                    problems.push(format!("capture time {ours}, exiftool {stamp}"));
                }
            }
            _ => problems.push(format!("capture time not read (exiftool has {stamp}); ordered by the file's date")),
        }
    }

    text(&mut problems, "make", photo.camera_make.as_deref(), tool, "Make");
    text(&mut problems, "model", photo.camera_model.as_deref(), tool, "Model");
    number(&mut problems, "ISO", photo.iso.map(|v| v as f64), tool, "ISO");
    number(&mut problems, "exposure", photo.exposure_time_s.map(f64::from), tool, "ExposureTime");
    number(&mut problems, "f-number", photo.f_number.map(f64::from), tool, "FNumber");
    number(&mut problems, "focal length", photo.focal_length_mm.map(f64::from), tool, "FocalLength");
    // exiftool reports 9 for "unknown" (Kodak and Canon TIFF-era bodies); the scanner says 1.
    let orientation_known = tool.get("Orientation").and_then(Value::as_f64).is_some_and(|v| (1.0..=8.0).contains(&v));
    number(&mut problems, "orientation", Some(f64::from(photo.orientation)), if orientation_known { tool } else { &Value::Null }, "Orientation");
    problems
}

fn text(problems: &mut Vec<String>, label: &str, ours: Option<&str>, tool: &Value, key: &str) {
    let Some(theirs) = tool.get(key).map(value_text) else {
        return;
    };
    let theirs = theirs.trim();
    if theirs.is_empty() {
        return;
    }
    match ours {
        Some(ours) if ours.trim().eq_ignore_ascii_case(theirs) => {}
        Some(ours) => problems.push(format!("{label} \"{ours}\", exiftool \"{theirs}\"")),
        None => problems.push(format!("{label} missing, exiftool \"{theirs}\"")),
    }
}

fn number(problems: &mut Vec<String>, label: &str, ours: Option<f64>, tool: &Value, key: &str) {
    let Some(theirs) = tool.get(key).and_then(Value::as_f64) else {
        return;
    };
    match ours {
        // One percent: the two sides round rationals differently, and 1/3 s is 0.333 either way.
        Some(ours) if (ours - theirs).abs() <= theirs.abs() * 0.01 + 1e-6 => {}
        Some(ours) => problems.push(format!("{label} {ours}, exiftool {theirs}")),
        None => problems.push(format!("{label} missing, exiftool {theirs}")),
    }
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `YYYY:MM:DD HH:MM:SS` for a local-time instant, the shape `DateTimeOriginal` is written in.
fn exif_stamp(local_ms: i64) -> String {
    let secs = local_ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}:{month:02}:{day:02} {:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Files under `root` that are neither a scanned photo, a companion, a sidecar nor reported as
/// skipped. An extension the scanner does not know lands here.
fn unscanned_files(root: &Path, scan: &meta::ScanResult) -> Result<Vec<String>, String> {
    let mut known: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for photo in &scan.photos {
        known.insert(&photo.rel_path);
        known.extend(photo.companions.iter().map(String::as_str));
    }
    known.extend(scan.skipped.iter().map(|s| s.rel_path.as_str()));
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
            if !known.contains(rel.as_str()) && extension(&rel) != "xmp" {
                out.push(rel);
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::exif_stamp;

    #[test]
    fn the_stamp_is_the_exif_shape() {
        // 2026-08-27 19:54:49 UTC.
        assert_eq!(exif_stamp(1_787_860_489_000), "2026:08:27 19:54:49");
        assert_eq!(exif_stamp(0), "1970:01:01 00:00:00");
        // A leap day, which is where a hand-rolled calendar usually goes wrong.
        assert_eq!(exif_stamp(951_782_400_000), "2000:02:29 00:00:00");
    }
}
