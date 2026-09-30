//! Owner: core-meta. See docs/contracts/ for this module's contract.
//!
//! Turning a folder on disk into a `Vec<PhotoMeta>` (docs/contracts/photo-meta.md). Two rules shape
//! everything here:
//!
//! * **Never skip a file macOS can read** (task.md §8). A file the parser chokes on comes back in
//!   `ScanResult::skipped` with the reason attached, never silently dropped, because a photo the
//!   user can see in Preview and Firstcut cannot is a bug they cannot work around.
//! * **Deterministic order** (REV-17). Results are sorted by `rel_path`, so a rescan produces the
//!   same ids in the same sequence and a session database written from one scan matches the next.

pub mod cr3;
pub(crate) mod exif;

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use cr3::{AfInfo, AfPoint, ByteRange, Cr3, EmbeddedPreview};

/// A half-open run of bytes inside a file.
pub type ByteRangeRef = ByteRange;

/// The kinds of file a shoot is made of. Mirrors `FileKind` in docs/contracts/photo-meta.md, and
/// `crate::batch::fixture::FileKind` is the same enumeration in its serde spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    Raw(RawFormat),
    Jpeg,
    Heif,
    Tiff,
    Png,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RawFormat {
    Cr3,
    Cr2,
    Crw,
    Arw,
    Sr2,
    Srf,
    Nef,
    Nrw,
    Raf,
    Rw2,
    Orf,
    Pef,
    Dng,
    Rwl,
    ThreeFr,
    Fff,
    Iiq,
    Srw,
    Dcr,
    Kdc,
    Erf,
    Mef,
    Mos,
    Gpr,
    X3f,
}

/// Where a capture time came from. `FileModified` has to be flagged, because it is a guess and
/// ordering a shoot by it can be wrong (docs/contracts/photo-meta.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeSource {
    Exif,
    FileModified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTime {
    pub unix_ms: i64,
    /// 10 for the Canon R8; 1000 when the file has no sub-seconds.
    pub subsec_resolution_ms: u16,
    pub offset_minutes: Option<i16>,
    pub source: TimeSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoMeta {
    pub id: crate::batch::PhotoId,
    /// Primary file: the RAW when there is one, otherwise the JPEG.
    pub rel_path: String,
    /// Paired JPEG/HEIF with the same base name, plus any existing `.xmp`.
    pub companions: Vec<String>,
    pub kind: FileKind,
    pub file_size: u64,
    pub capture_time: Option<CaptureTime>,
    pub shutter_count: Option<u64>,
    pub file_number: Option<u32>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub camera_serial: Option<String>,
    pub lens_model: Option<String>,
    pub focal_length_mm: Option<f32>,
    pub exposure_time_s: Option<f32>,
    pub f_number: Option<f32>,
    pub iso: Option<u32>,
    pub exposure_comp_ev: Option<f32>,
    pub metering_mode: Option<String>,
    pub drive_mode: Option<String>,
    pub shutter_mode: Option<String>,
    /// EXIF orientation, 1..=8.
    pub orientation: u8,
    /// Sensor dimensions, before the orientation is applied.
    pub width: u32,
    pub height: u32,
    pub af: Option<AfInfo>,
    pub preview: Option<EmbeddedPreview>,
    /// `st_dev` of the file, so a session can tell a rename from a reshoot (REV-68).
    pub device: Option<i64>,
    /// `st_ino`, which survives a rename inside a volume and is the cheapest identity there is.
    pub ino: Option<i64>,
    /// Non-fatal parse problems, shown in the log rather than swallowed.
    pub warnings: Vec<String>,
}

/// A file that could not be turned into a `PhotoMeta`, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub rel_path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    /// In `rel_path` order, which is also the order the scanner discovered them.
    pub photos: Vec<PhotoMeta>,
    /// Files that were found and not understood. Never empty because a parse failed silently.
    pub skipped: Vec<Skipped>,
}

impl ScanResult {
    pub fn is_empty(&self) -> bool {
        self.photos.is_empty() && self.skipped.is_empty()
    }
}

#[derive(Debug)]
pub enum ScanError {
    FolderNotFound(PathBuf),
    NotAFolder(PathBuf),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanError::FolderNotFound(path) => write!(f, "no such folder: {}", path.display()),
            ScanError::NotAFolder(path) => write!(f, "not a folder: {}", path.display()),
            ScanError::Io { path, source } => {
                write!(f, "reading {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ScanError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The file extensions the scanner treats as photos, grouped by what they are.
///
/// Mirrors `IMAGE_EXTENSIONS` in `store::identity`, which hashes the same set to identify a shoot.
/// The two lists must agree or a reshoot would be invisible to the session identity, so both are
/// derived from the table below.
/// Dropped by Finish Cull into a folder it creates inside the shoot; the scanner skips any folder
/// that holds one.
pub const FINISH_MARKER: &str = ".firstcut-finished";

/// The RAW extensions, listed so the test that keeps this table and the scanner in step has
/// something to iterate over. `raw_format_of` is the authority on which are which.
#[cfg(test)]
const RAW_EXTENSIONS: &[&str] = &[
    "3fr", "arw", "cr2", "cr3", "crw", "dcr", "dng", "erf", "fff", "gpr", "iiq", "kdc", "mef",
    "mos", "nef", "nrw", "orf", "pef", "raf", "rw2", "rwl", "sr2", "srf", "srw", "x3f",
];
const JPEG_EXTENSIONS: &[&str] = &["jpg", "jpeg"];
const HEIF_EXTENSIONS: &[&str] = &["heic", "heif"];
const TIFF_EXTENSIONS: &[&str] = &["tif", "tiff"];
const PNG_EXTENSIONS: &[&str] = &["png"];

/// Every extension the scanner will open, for the store's fingerprint to agree with.
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "3fr", "arw", "cr2", "cr3", "crw", "dcr", "dng", "erf", "fff", "gpr", "heic", "heif", "iiq",
    "jpeg", "jpg", "kdc", "mef", "mos", "nef", "nrw", "orf", "pef", "png", "raf", "rw2", "rwl",
    "sr2", "srf", "srw", "tif", "tiff", "x3f",
];

/// The RAW formats this build can name. A RAW we cannot name is still a photo; it is reported with
/// a reason rather than being hidden, which is what task.md §8 asks for.
fn raw_format_of(extension: &str) -> Option<RawFormat> {
    Some(match extension {
        "cr3" => RawFormat::Cr3,
        "cr2" => RawFormat::Cr2,
        "crw" => RawFormat::Crw,
        "arw" => RawFormat::Arw,
        "sr2" => RawFormat::Sr2,
        "srf" => RawFormat::Srf,
        "nef" => RawFormat::Nef,
        "nrw" => RawFormat::Nrw,
        "raf" => RawFormat::Raf,
        "rw2" => RawFormat::Rw2,
        "orf" => RawFormat::Orf,
        "pef" => RawFormat::Pef,
        "dng" => RawFormat::Dng,
        "rwl" => RawFormat::Rwl,
        "3fr" => RawFormat::ThreeFr,
        "fff" => RawFormat::Fff,
        "iiq" => RawFormat::Iiq,
        "srw" => RawFormat::Srw,
        "dcr" => RawFormat::Dcr,
        "kdc" => RawFormat::Kdc,
        "erf" => RawFormat::Erf,
        "mef" => RawFormat::Mef,
        "mos" => RawFormat::Mos,
        "gpr" => RawFormat::Gpr,
        "x3f" => RawFormat::X3f,
        _ => return None,
    })
}

fn kind_of(extension: &str) -> Option<FileKind> {
    let lower = extension.to_ascii_lowercase();
    if let Some(format) = raw_format_of(&lower) {
        return Some(FileKind::Raw(format));
    }
    if JPEG_EXTENSIONS.contains(&lower.as_str()) {
        return Some(FileKind::Jpeg);
    }
    if HEIF_EXTENSIONS.contains(&lower.as_str()) {
        return Some(FileKind::Heif);
    }
    if TIFF_EXTENSIONS.contains(&lower.as_str()) {
        return Some(FileKind::Tiff);
    }
    if PNG_EXTENSIONS.contains(&lower.as_str()) {
        return Some(FileKind::Png);
    }
    None
}

/// The shared base name, which is what pairs a RAW with its JPEG and names its sidecar.
///
/// `IMG_0001.CR3` and `IMG_0001.JPG` share `IMG_0001`; `IMG_0001.CR3` and `IMG_0001.CR3.xmp` also
/// share it, which is why the sidecar is looked up as `<base>.xmp` rather than `<file>.xmp`.
/// The grouping key for one file: its relative path with the *file name's* extension removed, so
/// it is `pub` because the session stores the same key in `photos.group_key` and the two must not
/// drift. See [`scan_folder`].
///
/// `Sat.1/IMG_2.CR3`, `Sat.1/IMG_2.JPG` and `Sat.1/IMG_2.xmp` share a key and `Sun.1/IMG_2.CR3`
/// does not.
///
/// Only the extension is stripped, never a dot inside a folder name — a card shot over a weekend
/// really does have folders called `Sat.1`, and splitting on the first dot would have merged every
/// file in that folder into one group.
pub fn group_key(rel_path: &str) -> String {
    let (dir, name) = rel_path
        .rsplit_once('/')
        .map_or(("", rel_path), |(dir, name)| (dir, name));
    let stem = name.split_once('.').map_or(name, |(stem, _)| stem);
    if dir.is_empty() {
        stem.to_string()
    } else {
        format!("{dir}/{stem}")
    }
}

/// `PhotoMeta` is the one type the batcher reads, so it implements the read-only view directly.
///
/// The file's mtime is not on the struct — it is in the session database, which is what survives a
/// rescan — so the in-memory `PhotoMeta` reports no mtime and the batcher falls back to EXIF alone.
/// A file with no EXIF date is ordered last and flagged by `order_with_report`.
impl crate::batch::view::Photo for PhotoMeta {
    fn id(&self) -> crate::batch::PhotoId {
        self.id
    }
    fn rel_path(&self) -> &str {
        &self.rel_path
    }
    fn camera_serial(&self) -> Option<&str> {
        self.camera_serial.as_deref()
    }
    fn capture_unix_ms(&self) -> Option<i64> {
        self.capture_time.as_ref().map(|time| time.unix_ms)
    }
    fn subsec_resolution_ms(&self) -> u16 {
        self.capture_time
            .as_ref()
            .map_or(1000, |time| time.subsec_resolution_ms)
    }
    fn file_mtime_ms(&self) -> Option<i64> {
        None
    }
    fn shutter_count(&self) -> Option<u64> {
        self.shutter_count
    }
    fn file_number(&self) -> Option<u32> {
        self.file_number
    }
    fn focal_length_mm(&self) -> Option<f32> {
        self.focal_length_mm
    }
    fn exposure_time_s(&self) -> Option<f32> {
        self.exposure_time_s
    }
    fn f_number(&self) -> Option<f32> {
        self.f_number
    }
    fn iso(&self) -> Option<u32> {
        self.iso
    }
    fn orientation(&self) -> u8 {
        self.orientation
    }
}

/// Scans `folder` recursively and returns one `PhotoMeta` per photo.
///
/// Reads headers only, several files at a time. A file that cannot be parsed appears in `skipped` with
/// the parser's own message, so a user can be told *why* rather than just losing a photo.
pub fn scan_folder(folder: &Path) -> Result<ScanResult, ScanError> {
    let root = std::fs::canonicalize(folder).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ScanError::FolderNotFound(folder.to_path_buf())
        } else {
            ScanError::Io {
                path: folder.to_path_buf(),
                source,
            }
        }
    })?;
    if !root.is_dir() {
        return Err(ScanError::NotAFolder(root));
    }

    // Every candidate file first, in path order, so the pairing below cannot depend on the order
    // the file system happened to hand entries back.
    let mut candidates: Vec<Candidate> = Vec::new();
    collect(&root, &root, &mut candidates)?;

    // Pair by shared base name: RAW primary, JPEG/HEIF companions, `.xmp` alongside.
    //
    // The key is the *relative path* minus its extension, not the bare file name. A card shot over
    // several days has `IMG_0001.CR3` in every subfolder, and keying on the name alone collapsed
    // those into one photo — which then silently dropped the losers, because a second RAW in a
    // group is neither a companion nor reported. Keying on the full path keeps the RAW/JPEG/XMP
    // pairing and separates subfolders.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut by_key: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let key = group_key(&candidate.rel_path);
        match by_key.get(&key) {
            Some(&group) => groups[group].push(index),
            None => {
                by_key.insert(key, groups.len());
                groups.push(vec![index]);
            }
        }
    }

    // First decide each group's primary file and companions (cheap, sequential), then read the
    // headers of the primaries in parallel: the header read is the only part that touches the
    // disk per photo, and an SSD serves several small reads at once far faster than one by one.
    let mut jobs: Vec<(usize, Vec<String>)> = Vec::with_capacity(groups.len());
    let mut skipped = Vec::new();
    for members in groups {
        // The RAW is the primary file; otherwise the first member in path order, which is what
        // makes a JPEG-only shoot deterministic.
        let primary = members
            .iter()
            .copied()
            .find(|&i| matches!(candidates[i].kind, FileKind::Raw(_)))
            .unwrap_or(members[0]);

        let mut companions: Vec<String> = members
            .iter()
            .copied()
            .filter(|&i| i != primary)
            .filter(|&i| !matches!(candidates[i].kind, FileKind::Raw(_)))
            .map(|i| candidates[i].rel_path.clone())
            .collect();
        // A sidecar is named after the file, not the group, and is not a photo in its own right.
        let sidecar = format!("{}.xmp", candidates[primary].rel_path);
        if root.join(&sidecar).is_file() {
            companions.push(sidecar);
        }
        companions.sort();

        // Every other member is reported. A second RAW in the same group is not a companion — a
        // `Sat.1/IMG_2.CR3` and a `Sat.1/IMG_2.NEF` are two different photos, and the contract
        // says a file is never dropped silently — so it is listed as skipped with the reason
        // rather than quietly vanishing.
        for &member in &members {
            if member == primary {
                continue;
            }
            if matches!(candidates[member].kind, FileKind::Raw(_)) {
                skipped.push(Skipped {
                    rel_path: candidates[member].rel_path.clone(),
                    reason: format!(
                        "a second raw beside {}; only one raw can be the primary file",
                        candidates[primary].rel_path
                    ),
                });
            }
        }
        jobs.push((primary, companions));
    }

    let results = parallel_map(jobs, |(primary, companions)| {
        let file = &candidates[primary];
        meta_for(
            &file.rel_path,
            &file.path,
            file.kind,
            file.size,
            companions,
            file.device,
            file.ino,
        )
        .map_err(|reason| Skipped {
            rel_path: file.rel_path.clone(),
            reason,
        })
    });
    let mut photos = Vec::with_capacity(results.len());
    for result in results {
        match result {
            Ok(meta) => photos.push(meta),
            Err(skip) => skipped.push(skip),
        }
    }

    photos.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    skipped.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(ScanResult { photos, skipped })
}

/// Maps `work` through `f` on a few threads and returns the results in input order.
///
/// Header reads are small and mostly waiting on the disk, so a handful of threads is enough; more
/// only adds contention. Falls back to the calling thread for tiny inputs.
fn parallel_map<T: Send, R: Send>(work: Vec<T>, f: impl Fn(T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);
    if threads == 1 || work.len() < 32 {
        return work.into_iter().map(f).collect();
    }
    let len = work.len();
    let queue = std::sync::Mutex::new(work.into_iter().enumerate());
    let mut out: Vec<Option<R>> = (0..len).map(|_| None).collect();
    let results = std::sync::Mutex::new(&mut out);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let next = queue.lock().unwrap_or_else(|e| e.into_inner()).next();
                    let Some((index, item)) = next else { break };
                    let value = f(item);
                    results.lock().unwrap_or_else(|e| e.into_inner())[index] = Some(value);
                }
            });
        }
    });
    out.into_iter().flatten().collect()
}

/// Walks `dir` collecting image files, sorted so the scan is reproducible (REV-17).
/// One file the scanner found, with the stat identity a session needs to recognise a rename.
struct Candidate {
    rel_path: String,
    path: PathBuf,
    kind: FileKind,
    size: u64,
    device: i64,
    ino: i64,
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<Candidate>) -> Result<(), ScanError> {
    let entries = std::fs::read_dir(dir).map_err(|source| ScanError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ScanError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Dot-files and AppleDouble sidecars are Finder's business, not a shoot's.
        if name.starts_with('.') || name.starts_with("._") {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            // A folder Finish Cull created inside the shoot (`_Not kept`, `5 Keep`, …) holds photos
            // that have been dealt with. Without this the next scan would find them again, and the
            // photos the user just moved away would reappear in the cull.
            if path.join(FINISH_MARKER).exists() {
                continue;
            }
            collect(root, &path, out)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        // `.xmp` is a sidecar, not a photo: it belongs to a group, never on its own.
        if extension.eq_ignore_ascii_case("xmp") {
            continue;
        }
        let Some(kind) = kind_of(extension) else {
            continue;
        };
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        use std::os::unix::fs::MetadataExt;
        out.push(Candidate {
            rel_path: rel,
            path,
            kind,
            size: metadata.len(),
            device: metadata.dev() as i64,
            ino: metadata.ino() as i64,
        });
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(())
}

/// Builds the `PhotoMeta` for one file.
#[allow(clippy::too_many_arguments)]
fn meta_for(
    rel: &str,
    path: &Path,
    kind: FileKind,
    size: u64,
    companions: Vec<String>,
    device: i64,
    ino: i64,
) -> Result<PhotoMeta, String> {
    let id = crate::batch::photo_id(rel);
    let mtime_ms = file_mtime_ms(path);

    let parsed = match kind {
        FileKind::Raw(RawFormat::Cr3) => Cr3::parse(path).map_err(|err| err.to_string())?,
        // Every other format goes through the generic reader, which never fails on a file it does
        // not understand: the photo appears, ordered by file time, with a warning (task.md §8).
        other => exif::read(path, other, size)?,
    };

    let mut warnings = parsed.warnings.clone();
    if parsed.width == 0 || parsed.height == 0 {
        warnings.push("no image dimensions in the file".to_string());
    }

    let capture_time = capture_time_from(&parsed, mtime_ms);
    if capture_time
        .as_ref()
        .is_some_and(|time| time.source == TimeSource::FileModified)
    {
        // A file-system timestamp is a guess; the contract says it has to be flagged, and the
        // batcher's ordering report keys off exactly this.
        warnings.push("capture time came from the file system, not EXIF".to_string());
    }

    Ok(PhotoMeta {
        id,
        rel_path: rel.to_string(),
        companions,
        kind,
        file_size: size,
        capture_time,
        shutter_count: parsed.shutter_count,
        file_number: file_number_of(rel),
        camera_make: parsed.make,
        camera_model: parsed.model,
        camera_serial: parsed.body_serial_number,
        lens_model: parsed.lens_model,
        focal_length_mm: parsed.focal_length_mm,
        exposure_time_s: parsed.exposure_time_s,
        f_number: parsed.f_number,
        iso: parsed.iso,
        exposure_comp_ev: parsed.exposure_comp_ev,
        metering_mode: parsed.metering_mode,
        drive_mode: parsed.drive_mode,
        shutter_mode: parsed.shutter_mode,
        orientation: parsed.orientation.unwrap_or(1),
        width: parsed.width,
        height: parsed.height,
        af: parsed.af,
        preview: parsed.preview,
        device: Some(device),
        ino: Some(ino),
        warnings,
    })
}

/// `IMG_0451.CR3` → 451. The number in the name, not its rank: it survives a rollover.
///
/// Only ever a tie-breaker of last resort (task.md §5.1).
fn file_number_of(rel: &str) -> Option<u32> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let digits: String = stem.chars().skip_while(|c| !c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn file_mtime_ms(path: &Path) -> Option<i64> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(since.as_millis() as i64)
}

/// Turns the three EXIF timestamps into one instant in UTC.
///
/// The sub-second digits are the camera's resolution as well as the value: the R8 writes two
/// digits, so `.84` is 840 ms at 10 ms resolution, and reporting 1 ms resolution would be a lie
/// about how precisely the frames can be ordered.
fn capture_time_from(parsed: &Cr3, mtime_ms: Option<i64>) -> Option<CaptureTime> {
    // No EXIF date: the file's own timestamp is all there is, flagged so the batcher knows the
    // ordering it produces is a guess.
    let Some(stamp) = parsed.date_time_original.as_deref() else {
        return capture_time_fallback(mtime_ms);
    };
    let (y, mo, d, h, mi, s) = parse_exif_datetime(stamp)?;
    let subsec = parsed.subsec_time_original.as_deref().unwrap_or("").trim();
    let digits = subsec.len().min(9) as u32;
    let fraction_ms: i64 = if digits == 0 {
        0
    } else {
        subsec.parse::<i64>().unwrap_or(0).saturating_mul(1_000) / 10_i64.pow(digits)
    };
    let resolution = match subsec.len() {
        0 | 1 => 1000,
        2 => 10,
        _ => 1,
    };
    let offset = parsed
        .offset_time_original
        .as_deref()
        .and_then(parse_utc_offset);

    let local_ms = days_from_civil(y, mo, d) * 86_400_000
        + h * 3_600_000
        + mi * 60_000
        + s * 1_000
        + fraction_ms;

    Some(CaptureTime {
        // Without an offset the local reading is all there is; ordering inside a folder is
        // unaffected, only the absolute instant moves.
        unix_ms: local_ms - i64::from(offset.unwrap_or(0)) * 60_000,
        subsec_resolution_ms: resolution,
        offset_minutes: offset,
        source: TimeSource::Exif,
    })
}

/// Falls back to the file's own timestamp, flagged, when the file has no EXIF date at all.
///
/// Returns `None` only when there is neither, so ordering can report the photo as untimed rather
/// than inventing an instant.
fn capture_time_fallback(mtime_ms: Option<i64>) -> Option<CaptureTime> {
    Some(CaptureTime {
        unix_ms: mtime_ms?,
        subsec_resolution_ms: 1000,
        offset_minutes: None,
        source: TimeSource::FileModified,
    })
}

/// `2026:08:27 19:54:49` → `(2026, 8, 27, 19, 54, 49)`.
fn parse_exif_datetime(stamp: &str) -> Option<(i64, i64, i64, i64, i64, i64)> {
    let (date, time) = stamp.split_once(' ')?;
    let mut date = date.split(':');
    let (y, mo, d) = (date.next()?, date.next()?, date.next()?);
    if date.next().is_some() {
        return None;
    }
    let mut clock = time.split(':');
    let (h, mi, s) = (clock.next()?, clock.next()?, clock.next()?);
    if clock.next().is_some() {
        return None;
    }
    let (y, mo, d) = (y.parse().ok()?, mo.parse().ok()?, d.parse().ok()?);
    let (h, mi, s) = (h.parse().ok()?, mi.parse().ok()?, s.parse().ok()?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some((y, mo, d, h, mi, s))
}

/// `+HH:MM` / `-HH:MM` → signed minutes.
fn parse_utc_offset(text: &str) -> Option<i16> {
    let bytes = text.trim().as_bytes();
    if bytes.len() < 6 || bytes[3] != b':' {
        return None;
    }
    let sign = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let text = text.trim();
    let hours: i64 = text.get(1..3)?.parse().ok()?;
    let minutes: i64 = text.get(4..6)?.parse().ok()?;
    if hours > 14 || minutes > 59 {
        return None;
    }
    i16::try_from(sign * (hours * 60 + minutes)).ok()
}

/// Days since the Unix epoch for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_map_keeps_input_order() {
        for len in [0, 1, 31, 32, 1_000] {
            let input: Vec<usize> = (0..len).collect();
            let output = parallel_map(input.clone(), |n| n * 3);
            assert_eq!(output, input.iter().map(|n| n * 3).collect::<Vec<_>>());
        }
    }

    #[test]
    fn a_large_folder_scans_to_the_same_result_every_time() {
        // Above the parallel threshold, including unreadable files, so a race in collecting the
        // results would show up as a different or shorter list.
        let dir = std::env::temp_dir().join(format!("firstcut-par-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..80 {
            std::fs::write(dir.join(format!("IMG_{i:04}.JPG")), b"not a jpeg").unwrap();
        }
        let first = scan_folder(&dir).unwrap();
        assert_eq!(first.photos.len() + first.skipped.len(), 80);
        for _ in 0..3 {
            let again = scan_folder(&dir).unwrap();
            let names = |r: &ScanResult| {
                (
                    r.photos
                        .iter()
                        .map(|p| p.rel_path.clone())
                        .collect::<Vec<_>>(),
                    r.skipped
                        .iter()
                        .map(|s| s.rel_path.clone())
                        .collect::<Vec<_>>(),
                )
            };
            assert_eq!(names(&first), names(&again));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extensions_map_to_kinds_case_insensitively() {
        assert_eq!(kind_of("CR3"), Some(FileKind::Raw(RawFormat::Cr3)));
        assert_eq!(kind_of("cr3"), Some(FileKind::Raw(RawFormat::Cr3)));
        assert_eq!(kind_of("jpg"), Some(FileKind::Jpeg));
        assert_eq!(kind_of("JPEG"), Some(FileKind::Jpeg));
        assert_eq!(kind_of("heic"), Some(FileKind::Heif));
        assert_eq!(kind_of("tiff"), Some(FileKind::Tiff));
        assert_eq!(kind_of("png"), Some(FileKind::Png));
        assert_eq!(kind_of("xmp"), None);
        assert_eq!(kind_of("txt"), None);
    }

    #[test]
    fn the_scanner_and_the_session_fingerprint_agree_on_what_is_a_photo() {
        // The store hashes `IMAGE_EXTENSIONS` to decide "is this the same shoot?". A RAW the
        // scanner reads but the fingerprint ignores would make a rescan look like a new shoot.
        for extension in IMAGE_EXTENSIONS {
            assert!(
                kind_of(extension).is_some(),
                "{extension} is in the fingerprint list but the scanner has no kind for it"
            );
        }
        for extension in RAW_EXTENSIONS {
            assert!(
                IMAGE_EXTENSIONS.contains(extension),
                "{extension} is a RAW the scanner reads but the fingerprint ignores"
            );
            assert!(
                raw_format_of(extension).is_some(),
                "{extension} is listed as a RAW but has no RawFormat"
            );
        }
        for extension in RAW_EXTENSIONS {
            assert!(
                IMAGE_EXTENSIONS.contains(extension),
                "{extension} is a RAW the scanner reads but the fingerprint ignores"
            );
            assert!(
                raw_format_of(extension).is_some(),
                "{extension} is listed as a RAW but has no RawFormat"
            );
        }
    }

    #[test]
    fn a_raw_and_its_jpeg_share_a_group() {
        assert_eq!(group_key("IMG_0001.CR3"), "IMG_0001");
        assert_eq!(group_key("IMG_0001.JPG"), "IMG_0001");
        assert_eq!(group_key("IMG_0001.CR3.xmp"), "IMG_0001");
        assert_eq!(group_key("no-extension"), "no-extension");
    }

    #[test]
    fn a_file_name_number_is_not_its_rank() {
        assert_eq!(file_number_of("IMG_0451.CR3"), Some(451));
        assert_eq!(file_number_of("IMG_9999.CR3"), Some(9999));
        assert_eq!(file_number_of("holiday.CR3"), None);
        // A subfolder must not change the answer.
        assert_eq!(file_number_of("card2/IMG_0007.CR3"), Some(7));
    }

    #[test]
    fn exif_datetimes_parse() {
        assert_eq!(
            parse_exif_datetime("2026:08:27 19:54:49"),
            Some((2026, 8, 27, 19, 54, 49))
        );
        assert_eq!(parse_exif_datetime("2026:08:27"), None);
        assert_eq!(parse_exif_datetime("2026:08:27 19:54:49:00"), None);
        assert_eq!(parse_exif_datetime("2026:13:27 19:54:49"), None);
    }

    #[test]
    fn two_sub_second_digits_means_ten_millisecond_resolution() {
        // The R8 writes two digits. Reporting 1 ms resolution would claim a precision the frames
        // do not have, and the batcher would trust it.
        let parsed = Cr3 {
            date_time_original: Some("2026:08:27 19:54:49".to_string()),
            subsec_time_original: Some("84".to_string()),
            offset_time_original: Some("-06:00".to_string()),
            ..Cr3::default()
        };
        let time = capture_time_from(&parsed, None).expect("a time");
        assert_eq!(time.subsec_resolution_ms, 10);
        assert_eq!(time.unix_ms % 1000, 840);
        assert_eq!(time.offset_minutes, Some(-360));
        assert_eq!(time.source, TimeSource::Exif);
    }

    #[test]
    fn one_sub_second_digit_is_a_whole_tenth() {
        let parsed = Cr3 {
            date_time_original: Some("2026:08:27 19:54:49".to_string()),
            subsec_time_original: Some("8".to_string()),
            ..Cr3::default()
        };
        let time = capture_time_from(&parsed, None).expect("a time");
        assert_eq!(time.subsec_resolution_ms, 1000);
        assert_eq!(time.unix_ms % 1000, 800);
    }

    #[test]
    fn a_file_with_no_sub_seconds_keeps_whole_second_resolution() {
        let parsed = Cr3 {
            date_time_original: Some("2026:08:27 19:54:49".to_string()),
            ..Cr3::default()
        };
        let time = capture_time_from(&parsed, None).expect("a time");
        assert_eq!(time.subsec_resolution_ms, 1000);
        assert_eq!(time.unix_ms % 1000, 0);
    }

    #[test]
    fn an_offset_moves_the_instant_towards_utc() {
        let base = Cr3 {
            date_time_original: Some("2026:01:02 03:04:05".to_string()),
            ..Cr3::default()
        };
        let east = capture_time_from(
            &Cr3 {
                offset_time_original: Some("+05:30".to_string()),
                ..base.clone()
            },
            None,
        )
        .expect("a time");
        let west = capture_time_from(
            &Cr3 {
                offset_time_original: Some("-05:30".to_string()),
                ..base
            },
            None,
        )
        .expect("a time");
        assert_eq!(east.offset_minutes, Some(330));
        // The same wall-clock reading east of Greenwich is an *earlier* instant in UTC.
        assert_eq!(west.unix_ms - east.unix_ms, 11 * 3_600_000);
    }

    #[test]
    fn a_nonsense_offset_is_ignored_rather_than_guessed() {
        assert_eq!(parse_utc_offset("nonsense"), None);
        assert_eq!(parse_utc_offset("+25:00"), None);
        assert_eq!(parse_utc_offset("+05:00"), Some(300));
        assert_eq!(parse_utc_offset(" -06:00 "), Some(-360));
    }

    #[test]
    fn a_file_with_no_exif_date_uses_its_mtime_and_says_so() {
        let time = capture_time_fallback(Some(1_700_000_000_000)).expect("a time");
        assert_eq!(time.source, TimeSource::FileModified);
        assert_eq!(time.unix_ms, 1_700_000_000_000);
        assert!(capture_time_fallback(None).is_none());
    }

    #[test]
    fn civil_days_match_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(days_from_civil(2026, 8, 28), 20693);
        // Every date in a month, then a leap day, then the century that is not a leap year.
        assert_eq!(
            days_from_civil(2026, 2, 28) + 1,
            days_from_civil(2026, 3, 1)
        );
        assert_eq!(
            days_from_civil(2024, 2, 29) + 1,
            days_from_civil(2024, 3, 1)
        );
        assert_eq!(
            days_from_civil(1900, 2, 28) + 1,
            days_from_civil(1900, 3, 1)
        );
    }

    #[test]
    fn a_missing_folder_is_an_error_not_an_empty_shoot() {
        let missing = std::env::temp_dir().join("firstcut-no-such-folder-9a1b");
        let err = scan_folder(&missing).expect_err("a missing folder must fail");
        assert!(matches!(err, ScanError::FolderNotFound(_)), "{err}");
    }

    #[test]
    fn a_file_is_not_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, b"hello").unwrap();
        let err = scan_folder(&file).expect_err("a file must fail");
        assert!(matches!(err, ScanError::NotAFolder(_)), "{err}");
    }

    #[test]
    fn an_empty_folder_scans_to_nothing_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let result = scan_folder(dir.path()).expect("an empty folder is fine");
        assert!(result.is_empty());
    }

    #[test]
    fn a_file_the_parser_cannot_read_is_reported_with_a_reason() {
        // task.md §8: never skip a file macOS can read. A truncated CR3 comes back in `skipped`
        // with the parser's own message, so the user can be told what happened.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("IMG_0001.CR3"), b"not a real raw file").unwrap();
        let result = scan_folder(dir.path()).expect("a scan");
        assert!(result.photos.is_empty(), "nothing parsed");
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].rel_path, "IMG_0001.CR3");
        assert!(
            result.skipped[0].reason.contains("CR3"),
            "the reason should name the problem: {}",
            result.skipped[0].reason
        );
    }

    #[test]
    fn a_jpeg_only_folder_scans_like_a_raw_folder() {
        // task.md §8: "Folders of only JPEG/HEIF must cull exactly like RAW folders."
        use exif::fixtures::{heic, jpeg, png, tiff};
        let dir = tempfile::tempdir().unwrap();
        let at = |sec: u32| format!("2026:08:27 10:00:{sec:02}");
        for (name, bytes) in [
            (
                "IMG_0001.JPG",
                jpeg(&tiff(&at(1), "10", 100, "FUJIFILM", None), 6000, 4000),
            ),
            (
                "IMG_0002.jpeg",
                jpeg(&tiff(&at(1), "20", 100, "FUJIFILM", None), 6000, 4000),
            ),
            (
                "IMG_0003.HEIC",
                heic(&tiff(&at(9), "", 100, "Apple", None), 4032, 3024),
            ),
            (
                "IMG_0004.png",
                png(&tiff(&at(12), "", 100, "Apple", None), 800, 600),
            ),
        ] {
            std::fs::write(dir.path().join(name), bytes).unwrap();
        }
        let result = scan_folder(dir.path()).expect("a scan");
        assert!(result.skipped.is_empty(), "{:?}", result.skipped);
        assert_eq!(result.photos.len(), 4);
        for photo in &result.photos {
            let time = photo.capture_time.as_ref().expect("a capture time");
            assert_eq!(time.source, TimeSource::Exif, "{}", photo.rel_path);
            assert!(photo.width > 0 && photo.height > 0, "{}", photo.rel_path);
        }
        // Two frames 100 ms apart, resolved from the sub-second digits.
        let first = result.photos[0].capture_time.as_ref().unwrap();
        let second = result.photos[1].capture_time.as_ref().unwrap();
        assert_eq!(second.unix_ms - first.unix_ms, 100);
        assert_eq!(first.subsec_resolution_ms, 10);
    }

    #[test]
    fn a_raw_and_its_jpeg_are_one_photo_for_a_non_canon_raw() {
        use exif::fixtures::{jpeg, tiff};
        let dir = tempfile::tempdir().unwrap();
        let raw = tiff("2026:08:27 10:00:01", "50", 400, "SONY", Some((7000, 4600)));
        std::fs::write(dir.path().join("DSC00001.ARW"), &raw).unwrap();
        std::fs::write(dir.path().join("DSC00001.JPG"), jpeg(&raw, 7000, 4600)).unwrap();
        let result = scan_folder(dir.path()).expect("a scan");
        assert!(result.skipped.is_empty(), "{:?}", result.skipped);
        assert_eq!(result.photos.len(), 1);
        assert_eq!(result.photos[0].rel_path, "DSC00001.ARW");
        assert_eq!(
            result.photos[0].companions,
            vec!["DSC00001.JPG".to_string()]
        );
        assert_eq!(result.photos[0].camera_make.as_deref(), Some("SONY"));
    }

    #[test]
    fn a_format_the_reader_cannot_parse_still_appears_with_a_warning() {
        // A Sony ARW is a real photo. Losing it, even into `skipped`, would leave a photo the user
        // can see in Finder missing from the cull, which is what task.md §8 forbids. It comes back
        // ordered by file time, flagged, with the reason in `warnings`.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("DSC00001.ARW"), vec![0u8; 32]).unwrap();
        let result = scan_folder(dir.path()).expect("a scan");
        assert!(result.skipped.is_empty(), "{:?}", result.skipped);
        assert_eq!(result.photos.len(), 1);
        let photo = &result.photos[0];
        assert_eq!(
            photo.capture_time.as_ref().map(|t| t.source),
            Some(TimeSource::FileModified)
        );
        assert!(!photo.warnings.is_empty(), "{:?}", photo.warnings);
    }

    #[test]
    fn results_are_sorted_by_path_so_a_rescan_is_reproducible() {
        // REV-17: the file system hands entries back in whatever order it likes, and two scans of
        // the same folder have to produce the same sequence or the session database churns.
        let dir = tempfile::tempdir().unwrap();
        for name in ["IMG_0300.CR3", "IMG_0100.CR3", "IMG_0200.CR3"] {
            std::fs::write(dir.path().join(name), b"junk").unwrap();
        }
        let first = scan_folder(dir.path()).expect("a scan");
        let second = scan_folder(dir.path()).expect("a scan");
        let names: Vec<&str> = first.skipped.iter().map(|s| s.rel_path.as_str()).collect();
        assert_eq!(names, ["IMG_0100.CR3", "IMG_0200.CR3", "IMG_0300.CR3"]);
        assert_eq!(first, second, "the same folder scans the same way twice");
    }

    #[test]
    fn subfolders_are_scanned_and_paths_are_forward_slashed() {
        let dir = tempfile::tempdir().unwrap();
        let card2 = dir.path().join("card2");
        std::fs::create_dir(&card2).unwrap();
        std::fs::write(dir.path().join("IMG_0001.CR3"), b"junk").unwrap();
        std::fs::write(card2.join("IMG_0002.CR3"), b"junk").unwrap();
        let result = scan_folder(dir.path()).expect("a scan");
        let names: Vec<&str> = result.skipped.iter().map(|s| s.rel_path.as_str()).collect();
        assert_eq!(names, ["IMG_0001.CR3", "card2/IMG_0002.CR3"]);
        assert!(
            names.iter().all(|name| !name.contains('\\')),
            "paths are '/' separated so a PhotoId is the same on every platform"
        );
    }

    #[test]
    fn finder_junk_and_sidecars_are_not_photos() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("IMG_0001.CR3.xmp"), b"<x/>").unwrap();
        std::fs::write(dir.path().join("._IMG_0001.CR3"), b"junk").unwrap();
        std::fs::write(dir.path().join(".DS_Store"), b"junk").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"hello").unwrap();
        let result = scan_folder(dir.path()).expect("a scan");
        assert!(result.is_empty(), "{:?}", result);
    }

    #[test]
    fn the_group_key_keeps_subfolders_apart_even_when_they_contain_a_dot() {
        // `photos.group_key` is an indexed column that the session fills from this same
        // function, so the definition lives here and nowhere else.
        assert_eq!(group_key("IMG_0002.CR3"), "IMG_0002");
        assert_eq!(group_key("Sat.1/IMG_0002.CR3"), "Sat.1/IMG_0002");
        assert_eq!(
            group_key("Sat.1/IMG_0002.CR3"),
            group_key("Sat.1/IMG_0002.JPG"),
            "a raw and its jpeg are one photo"
        );
        assert_ne!(
            group_key("Sat.1/IMG_0002.CR3"),
            group_key("Sun.1/IMG_0002.CR3"),
            "the same name in two folders is two photos"
        );
        // The bug this replaced: splitting the whole path on its first dot.
        assert_ne!(group_key("Sat.1/IMG_0002.CR3"), "Sat");
    }

    #[test]
    fn the_same_file_name_in_two_folders_is_two_photos() {
        // A card shot over two days has IMG_0001 in every subfolder. Keying the groups on the
        // bare file name merged them into one photo, and the second RAW was then dropped without
        // even a `skipped` entry, so the shoot silently lost frames.
        let dir = tempfile::tempdir().unwrap();
        for day in ["Sat.1", "Sun.1"] {
            std::fs::create_dir(dir.path().join(day)).unwrap();
            std::fs::write(dir.path().join(day).join("IMG_0001.CR3"), b"junk").unwrap();
        }
        let result = scan_folder(dir.path()).expect("a scan");
        assert_eq!(
            result.skipped.len(),
            2,
            "each file is reported on its own: {:?}",
            result.skipped
        );
        assert_eq!(
            result
                .skipped
                .iter()
                .map(|s| s.rel_path.as_str())
                .collect::<Vec<_>>(),
            ["Sat.1/IMG_0001.CR3", "Sun.1/IMG_0001.CR3"],
            "a dot in the folder name does not merge the folders either"
        );
    }

    #[test]
    fn a_second_raw_beside_the_first_is_reported_not_swallowed() {
        // Two raws share a stem, so they land in one group. Only one can be the primary file, and
        // the other has to be visible somewhere rather than vanish.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("IMG_0001.CR3"), b"junk").unwrap();
        std::fs::write(dir.path().join("IMG_0001.NEF"), b"junk").unwrap();
        let result = scan_folder(dir.path()).expect("a scan");
        let reported: Vec<&str> = result.skipped.iter().map(|s| s.rel_path.as_str()).collect();
        assert!(
            reported.contains(&"IMG_0001.CR3") || reported.contains(&"IMG_0001.NEF"),
            "at least one of the pair is reported with a reason: {reported:?}"
        );
        assert!(
            result
                .skipped
                .iter()
                .any(|s| s.reason.contains("second raw")),
            "the reason says why the other raw could not be the primary: {:?}",
            result.skipped
        );
    }
}
