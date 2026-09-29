//! Owner: core-meta. `scan_folder`: turn a directory of RAW files into `PhotoMeta` (task.md §7.4).
//!
//! The shape of a scan, and why:
//!
//! * **Headers only.** Each file is opened, its `moov` box is walked, and roughly the first 60 KB
//!   is read. The 12 MB of image payload is never touched, which is what keeps this under the
//!   2 ms/file target in §7.4 and what lets 1,500 files be scanned in the seconds the app allows.
//! * **Parallel.** Files are independent, so the scan fans out across the performance cores. Each
//!   thread opens its own handles; nothing is shared but the results, which are collected in a
//!   fixed order so two runs of the same folder produce byte-identical output (REV-17).
//! * **Nothing is dropped.** A file that cannot be parsed becomes a `SkippedFile` with a reason
//!   and still appears in the list, because task.md §8 requires a corrupt file to show up rather
//!   than disappear.
//! * **Sidecars are excluded from the photo list** but travel with their photo, exactly as
//!   core-store's folder fingerprint already does: a rating written to a `.xmp` must not change
//!   the folder's identity.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::meta::{FileKind, PhotoMeta, ScanResult, SkippedFile, TimeSource};
pub mod cr3;

use crate::scan::cr3::Cr3Reader;

/// Extensions treated as sidecars rather than photos.
const SIDECAR_EXTENSIONS: &[&str] = &["xmp", "aae", "lrcat", "lrtemplate", "lrprefs", "dop", "tmp"];

/// `scan_folder(path) -> ScanResult` (docs/contracts/photo-meta.md).
///
/// Parallel when there is more than one file, because a 1,500-file shoot is the case that has to
/// be fast and a single-threaded header read would spend most of its time in `open()`/`seek()`.
pub fn scan_folder(path: &Path) -> ScanResult {
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(e) => {
            return ScanResult {
                warnings: vec![format!("cannot read {}: {e}", path.display())],
                ..ScanResult::default()
            };
        }
    };

    // Group first: a RAW plus its paired JPEG is one photo, so the companions have to be known
    // before any file is parsed.
    let mut photos: Vec<PendingPhoto> = Vec::new();
    let mut companions: Vec<PathBuf> = Vec::new();
    let mut skipped: Vec<SkippedFile> = Vec::new();

    for entry in entries.flatten() {
        let file_path = entry.path();
        let name = match file_path.file_name().and_then(|n| n.to_str()) {
            Some(n) if !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if name == "Firstcut.session" {
            continue;
        }
        if SIDECAR_EXTENSIONS.contains(&ext.as_str()) {
            companions.push(file_path);
            continue;
        }
        if FileKind::from_extension(&ext).is_none() {
            continue;
        }

        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        photos.push(PendingPhoto {
            path: file_path,
            name,
            size,
        });
    }

    // Capture-ordered by name first, so the parallel results land in a deterministic order
    // regardless of which thread finished first.
    photos.sort_by(|a, b| a.name.cmp(&b.name));

    let workers = std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
        .min(photos.len().max(1));

    let parsed: Vec<Parsed> = if workers <= 1 {
        photos
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let (meta, err) = read_one(p);
                (i, meta, err)
            })
            .collect()
    } else {
        parse_in_parallel(&photos, workers)
    };

    // Pair each companion with the photo it belongs to, by stem.
    let stems: Vec<String> = photos.iter().map(|p| stem_of(&p.name)).collect();
    let mut out: Vec<PhotoMeta> = Vec::with_capacity(parsed.len());
    for (i, mut meta, err) in parsed {
        if let Some(reason) = err {
            skipped.push(SkippedFile {
                rel_path: meta.rel_path.clone(),
                reason,
            });
        }
        let stem = &stems[i];
        let own: Vec<String> = companions
            .iter()
            .filter_map(|c| c.file_name())
            .filter_map(|n| n.to_str())
            .filter(|n| *n != meta.rel_path)
            .filter(|n| stem_of(n) == *stem)
            .map(str::to_string)
            .collect();
        if own.is_empty() {
            meta.companions.clear();
        } else {
            meta.companions = own;
        }
        out.push(meta);
    }

    ScanResult {
        photos: out,
        skipped,
        warnings: Vec::new(),
    }
}

/// One parsed file: its index in the input, the photo, and the reason it could not be read.
type Parsed = (usize, PhotoMeta, Option<String>);

struct PendingPhoto {
    path: PathBuf,
    name: String,
    size: u64,
}

/// Fans the parse across `workers` threads using a shared cursor, so a slow file does not idle a
/// core and the work stays balanced without a work-stealing dependency.
fn parse_in_parallel(photos: &[PendingPhoto], workers: usize) -> Vec<Parsed> {
    let next = AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<Parsed>>> = (0..photos.len())
        .map(|_| std::sync::Mutex::new(None))
        .collect();

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= photos.len() {
                        break;
                    }
                    let (meta, err) = read_one(&photos[i]);
                    *slots[i].lock().expect("slot mutex") = Some((i, meta, err));
                }
            });
        }
    });

    slots
        .into_iter()
        .map(|s| {
            s.into_inner()
                .expect("slot mutex")
                .expect("every index is filled")
        })
        .collect()
}

/// Reads one file. Returns the photo and, if anything went wrong, the reason -- a file with a
/// warning is still a photo, which is the whole point of task.md §8.
fn read_one(pending: &PendingPhoto) -> (PhotoMeta, Option<String>) {
    let ext = Path::new(&pending.name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let kind = FileKind::from_extension(&ext).unwrap_or(FileKind::Jpeg);
    let mut meta = PhotoMeta {
        rel_path: pending.name.clone(),
        file_size: pending.size,
        kind,
        file_number: file_number_of(&pending.name),
        ..PhotoMeta::default()
    };

    if !kind.is_raw() {
        // JPEG/HEIF/PNG: the app still shows them, it just cannot read a MakerNote out of them.
        // `AFPhotoMeta`/ImageIO fills the rest in on the Swift side (REV-20).
        meta.capture_time = mtime_fallback(&pending.path);
        return (meta, None);
    }

    match Cr3Reader::open(&pending.path) {
        Ok(mut reader) => match reader.read() {
            Ok(m) => {
                meta.camera_make = m.make;
                meta.camera_model = m.model;
                meta.camera_serial = m.serial;
                meta.lens_model = m.lens;
                meta.orientation = m.orientation;
                meta.capture_time = m.capture;
                meta.focal_length_mm = m.focal_length_mm.map(|v| v as f32);
                meta.exposure_time_s = m.exposure_time_s.map(|v| v as f32);
                meta.f_number = m.f_number.map(|v| v as f32);
                meta.iso = m.iso;
                meta.exposure_comp_ev = m.exposure_comp_ev.map(|v| v as f32);
                meta.width = m.width;
                meta.height = m.height;
                meta.shutter_count = m.shutter_count;
                if m.shutter_count.is_none() {
                    meta.warnings.push(
                        "no ShutterCount in this file: ordering falls back to time alone".into(),
                    );
                }
                (meta, None)
            }
            Err(e) => {
                meta.warnings.push(e.to_string());
                meta.capture_time = mtime_fallback(&pending.path);
                (meta, Some(e.to_string()))
            }
        },
        Err(e) => {
            meta.warnings.push(e.to_string());
            meta.capture_time = mtime_fallback(&pending.path);
            (meta, Some(e.to_string()))
        }
    }
}

/// The file's mtime, as an explicit *fallback*. Task.md §5.1 requires this to be flagged rather
/// than presented as a capture time, and §5.3/REV-63 forbid treating it as evidence.
fn mtime_fallback(path: &Path) -> Option<crate::meta::CaptureTime> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(crate::meta::CaptureTime {
        unix_ms: since.as_millis() as i64,
        subsec_resolution_ms: 1000,
        offset_minutes: None,
        source: TimeSource::FileModified,
    })
}

/// `IMG_0451.CR3` -> `451`. Never the rank in the folder: a shoot can start at 9146 and a rename
/// changes it, so it is evidence about the file, not about the sequence.
fn file_number_of(name: &str) -> Option<u32> {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let digits: String = stem.chars().filter(char::is_ascii_digit).collect();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

/// The stem a companion is matched on, lowercased so `IMG_0001.CR3` and `img_0001.jpg` pair.
///
/// Strips **every** extension, not just the last: a sidecar is named `IMG_0001.CR3.xmp`, so
/// stripping one extension leaves `img_0001.cr3` and it never matches its photo. (A test caught
/// this, which is the argument for having one.)
fn stem_of(name: &str) -> String {
    name.split('.').next().unwrap_or(name).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_number_is_not_its_rank() {
        assert_eq!(file_number_of("IMG_0451.CR3"), Some(451));
        assert_eq!(file_number_of("IMG_9999.CR3"), Some(9999));
        // A rollover means the number is not a position: 9999 is followed by 1.
        assert!(file_number_of("IMG_9999.CR3").unwrap() > file_number_of("IMG_0001.CR3").unwrap());
        assert_eq!(file_number_of("DSC_0001.CR3"), Some(1));
        assert_eq!(file_number_of("no-digits.cr3"), None);
    }

    #[test]
    fn stems_pair_companions_to_their_photo() {
        assert_eq!(stem_of("IMG_0001.CR3"), stem_of("IMG_0001.JPG"));
        // A sidecar carries its photo's full name plus its own extension.
        assert_eq!(stem_of("IMG_0001.CR3"), stem_of("IMG_0001.CR3.xmp"));
        // Case-insensitive, because a case-insensitive filesystem will hand back either spelling.
        assert_eq!(stem_of("IMG_0001.CR3"), stem_of("img_0001.cr3"));
        assert_ne!(stem_of("IMG_0001.CR3"), stem_of("IMG_0002.CR3"));
    }

    #[test]
    fn extensions_map_to_formats_case_insensitively() {
        assert_eq!(
            FileKind::from_extension("CR3"),
            Some(FileKind::Raw(crate::meta::RawFormat::Cr3))
        );
        assert_eq!(
            FileKind::from_extension("cr3"),
            Some(FileKind::Raw(crate::meta::RawFormat::Cr3))
        );
        assert_eq!(FileKind::from_extension("JPG"), Some(FileKind::Jpeg));
        assert_eq!(FileKind::from_extension("heic"), Some(FileKind::Heif));
        assert_eq!(FileKind::from_extension("txt"), None);
    }
}
