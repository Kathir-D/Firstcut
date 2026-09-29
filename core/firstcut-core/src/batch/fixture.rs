//! Compatibility re-export. The types live in [`crate::meta`] now.
//!
//! This used to declare `PhotoMeta`, `CaptureTime`, `FileKind` and friends itself, as core-batch's
//! stand-in while the real scanner did not exist. It does not any more: the scanner, the CLI, the
//! tests and the FFI all need the *same* type, and two definitions means a permanent adapter
//! between them -- which is exactly the shape of bug REV-56 describes, where a photo reads as
//! Keep in one layer and Unrated in another.
//!
//! The re-exports keep every existing `use crate::batch::fixture::PhotoMeta` working, so this is a
//! move rather than a rewrite. `meta` is the one definition; nothing here adds to it.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::batch::view::Photo;
use crate::batch::{PhotoId, photo_id};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTime {
    pub unix_ms: i64,
    /// 10 for Canon R8; 1000 when the file has no sub-seconds.
    pub subsec_resolution_ms: u16,
    pub offset_minutes: Option<i16>,
    pub source: TimeSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeSource {
    Exif,
    FileModified,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoMeta {
    pub id: PhotoId,
    pub rel_path: String,
    #[serde(default)]
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
    pub orientation: u8,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub af: Option<AfInfo>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfInfo {
    pub area_mode: String,
    pub points: Vec<AfPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfPoint {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub in_focus: bool,
}

impl Photo for PhotoMeta {
    fn id(&self) -> PhotoId {
        self.id
    }
    fn rel_path(&self) -> &str {
        &self.rel_path
    }
    fn camera_serial(&self) -> Option<&str> {
        self.camera_serial.as_deref()
    }
    fn capture_unix_ms(&self) -> Option<i64> {
        self.capture_time.as_ref().map(|c| c.unix_ms)
    }
    fn subsec_resolution_ms(&self) -> u16 {
        self.capture_time
            .as_ref()
            .map_or(1000, |c| c.subsec_resolution_ms)
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

/// A folder's metadata, by file name, for turning batches back into something a human can read.
#[derive(Debug, Default)]
pub struct Folder {
    pub photos: Vec<PhotoMeta>,
    by_path: HashMap<String, usize>,
    by_id: HashMap<PhotoId, usize>,
}

impl Folder {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        let photos: Vec<PhotoMeta> =
            serde_json::from_str(&text).map_err(|e| format!("parsing {}: {e}", path.display()))?;
        Ok(Self::assemble(photos))
    }

    pub fn assemble(photos: Vec<PhotoMeta>) -> Self {
        let mut folder = Self {
            by_path: HashMap::with_capacity(photos.len()),
            by_id: HashMap::with_capacity(photos.len()),
            photos,
        };
        for (i, p) in folder.photos.iter().enumerate() {
            folder.by_path.insert(p.rel_path.clone(), i);
            folder.by_id.insert(p.id, i);
        }
        folder
    }

    #[must_use]
    pub fn get_by_id(&self, id: PhotoId) -> Option<&PhotoMeta> {
        self.by_id.get(&id).map(|&i| &self.photos[i])
    }
}

/// `exiftool -j` output for one photo, with the tags this batcher uses.
#[derive(Debug, Clone, Deserialize)]
struct ExifToolRecord {
    #[serde(rename = "FileName")]
    file_name: String,
    #[serde(rename = "FileSize")]
    file_size: Option<u64>,
    #[serde(rename = "SubSecDateTimeOriginal")]
    subsec: Option<String>,
    #[serde(rename = "OffsetTimeOriginal")]
    offset: Option<String>,
    #[serde(rename = "ShutterCount")]
    shutter_count: Option<u64>,
    #[serde(rename = "Make")]
    make: Option<String>,
    #[serde(rename = "Model")]
    model: Option<String>,
    #[serde(rename = "SerialNumber")]
    serial: Option<serde_json::Value>,
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
    exposure_comp: Option<f32>,
    #[serde(rename = "MeteringMode")]
    metering_mode: Option<String>,
    #[serde(rename = "DriveMode")]
    drive_mode: Option<String>,
    #[serde(rename = "ContinuousDrive")]
    continuous_drive: Option<String>,
    #[serde(rename = "ShutterMode")]
    shutter_mode: Option<String>,
    #[serde(rename = "Orientation")]
    orientation: Option<u32>,
    #[serde(rename = "ImageWidth")]
    width: Option<u32>,
    #[serde(rename = "ImageHeight")]
    height: Option<u32>,
    #[serde(rename = "AFAreaMode")]
    af_area_mode: Option<String>,
}

/// Convert one exiftool dump into the contract's shape.
fn from_exiftool(r: ExifToolRecord) -> Result<PhotoMeta, String> {
    let kind = FileKind::Raw(
        raw_format_of(&r.file_name)
            .ok_or_else(|| format!("{}: unsupported extension", r.file_name))?,
    );

    let (unix_ms, subsec_resolution_ms, offset_minutes) = match &r.subsec {
        Some(s) => parse_subsec_datetime(s),
        None => return Err(format!("{}: no SubSecDateTimeOriginal", r.file_name)),
    };

    let warnings = Vec::new();
    Ok(PhotoMeta {
        id: photo_id(&r.file_name),
        rel_path: r.file_name.clone(),
        companions: Vec::new(),
        kind,
        file_size: r.file_size.unwrap_or(0),
        capture_time: Some(CaptureTime {
            unix_ms,
            subsec_resolution_ms,
            offset_minutes: offset_minutes.or_else(|| r.offset.as_deref().and_then(parse_offset)),
            source: TimeSource::Exif,
        }),
        shutter_count: r.shutter_count,
        file_number: file_number_of(&r.file_name),
        camera_make: r.make,
        camera_model: r.model,
        camera_serial: r.serial.and_then(stringify),
        lens_model: r.lens_model,
        focal_length_mm: r.focal_length,
        exposure_time_s: r.exposure_time,
        f_number: r.f_number,
        iso: r.iso,
        exposure_comp_ev: r.exposure_comp,
        metering_mode: r.metering_mode,
        // exiftool reports both DriveMode and the continuous rate; the contract has one field, and
        // the continuous rate is the one §3 says varies per burst.
        drive_mode: r.continuous_drive.or(r.drive_mode),
        shutter_mode: r.shutter_mode,
        orientation: r.orientation.unwrap_or(1).min(8) as u8,
        width: r.width.unwrap_or(0),
        height: r.height.unwrap_or(0),
        af: r.af_area_mode.map(|area_mode| AfInfo {
            area_mode,
            points: Vec::new(),
        }),
        warnings,
    })
}

/// Load and convert an exiftool JSON dump.
pub fn load_exiftool(path: &Path) -> Result<Vec<PhotoMeta>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let records: Vec<ExifToolRecord> =
        serde_json::from_str(&text).map_err(|e| format!("parsing {}: {e}", path.display()))?;
    let mut photos = Vec::with_capacity(records.len());
    for r in records {
        photos.push(from_exiftool(r)?);
    }
    Ok(photos)
}

/// `2026:08:27 19:54:49.84-06:00` → (UTC milliseconds, sub-second resolution, offset in minutes).
///
/// Splitting on the fixed `YYYY:MM:DD hh:mm:ss` separators keeps this obvious at a glance, which a
/// hand-rolled character loop over six different field widths would not.
fn parse_subsec_datetime(s: &str) -> (i64, u16, Option<i16>) {
    let bad = (0, 1000, None);
    let Some((date, rest)) = s.split_once(' ') else {
        return bad;
    };
    let mut date_fields = date.split(':');
    let (Some(y), Some(mo), Some(d)) = (date_fields.next(), date_fields.next(), date_fields.next())
    else {
        return bad;
    };
    if date_fields.next().is_some() {
        return bad;
    }

    // `hh:mm:ss[.fff][±HH:MM]`
    let (clock, offset) = match rest.rfind(['+', '-']) {
        Some(i) if i > 0 => (&rest[..i], parse_offset_tail(&rest[i..]).0),
        _ => (rest, None),
    };
    let (clock, frac) = match clock.split_once('.') {
        Some((c, f)) => (c, Some(f)),
        None => (clock, None),
    };
    let mut clock_fields = clock.split(':');
    let (Some(h), Some(mi), Some(sec)) = (
        clock_fields.next(),
        clock_fields.next(),
        clock_fields.next(),
    ) else {
        return bad;
    };
    if clock_fields.next().is_some() {
        return bad;
    }
    let (Ok(y), Ok(mo), Ok(d), Ok(h), Ok(mi), Ok(sec)) = (
        y.parse::<i64>(),
        mo.parse::<i64>(),
        d.parse::<i64>(),
        h.parse::<i64>(),
        mi.parse::<i64>(),
        sec.parse::<i64>(),
    ) else {
        return bad;
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return bad;
    }

    // exiftool pads the fraction to the camera's resolution: `.8` means 800 ms, `.84` means 840.
    let frac_digits = frac.map_or(0, str::len);
    let frac_value: i64 = frac
        .and_then(|f| f.parse().ok())
        .or(if frac_digits == 0 { Some(0) } else { None })
        .unwrap_or(0);
    let frac_ms = if frac_digits == 0 {
        0
    } else {
        frac_value * 1_000 / 10_i64.pow(frac_digits.min(9) as u32)
    };
    let resolution = match frac_digits {
        0 | 1 => 1000,
        2 => 10,
        _ => 1,
    };

    let local_ms = days_from_civil(y, mo, d) * 86_400_000
        + h * 3_600_000
        + mi * 60_000
        + sec * 1_000
        + frac_ms;

    // Local wall-clock time → UTC. Without an offset the local reading is all we have; ordering
    // inside a folder is unaffected either way, only the absolute instant moves.
    (
        local_ms - i64::from(offset.unwrap_or(0)) * 60_000,
        resolution,
        offset,
    )
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

/// Parse a `+HH:MM` / `-HH:MM` offset into signed minutes.
fn parse_offset_tail(s: &str) -> (Option<i16>, &str) {
    let bytes = s.as_bytes();
    let ok = bytes.len() >= 6 && bytes[3] == b':';
    if !ok {
        return (None, s);
    }
    let sign = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return (None, s),
    };
    let hours = s.get(1..3).and_then(|d| d.parse::<i64>().ok());
    let minutes = s.get(4..6).and_then(|d| d.parse::<i64>().ok());
    match (hours, minutes) {
        (Some(h), Some(m)) if h <= 14 && m < 60 => (Some((sign * (h * 60 + m)) as i16), &s[6..]),
        _ => (None, s),
    }
}

fn parse_offset(s: &str) -> Option<i16> {
    parse_offset_tail(s).0
}

fn stringify(v: serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `IMG_0451.CR3` → 451. The number in the name, not its rank: it survives a rollover.
///
/// Only a tie-breaker of last resort (task.md §5.1). Used here because the exiftool dumps carry no
/// Canon `FileNumber` tag.
fn file_number_of(name: &str) -> Option<u32> {
    let stem = name.rsplit_once('.')?.0;
    let digits: String = stem.chars().skip_while(|c| !c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn raw_format_of(name: &str) -> Option<RawFormat> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    let fmt = match ext.as_str() {
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
    };
    Some(fmt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::view::Photo;

    /// The id has to be a function of the path so the same folder scanned twice produces the same
    /// ids -- resume, the ground-truth F1 numbers and the CI regression test all depend on it.
    #[test]
    fn the_id_is_derived_from_the_path() {
        let mut a = PhotoMeta::default();
        a.rel_path = "IMG_0001.CR3".into();
        let mut b = PhotoMeta::default();
        b.rel_path = "IMG_0002.CR3".into();
        assert_ne!(a.id(), b.id());
        assert_eq!(a.id(), crate::meta::stable_id("IMG_0001.CR3"));
    }

    /// A `capture_time` whose source is `FileModified` is a *fallback*: `file_mtime_ms` has to
    /// report it so the batcher can refuse to hard-join on it (REV-63).
    #[test]
    fn a_fallback_time_is_reported_as_an_mtime() {
        let mut p = PhotoMeta::default();
        p.capture_time = Some(CaptureTime {
            unix_ms: 1_000,
            subsec_resolution_ms: 1000,
            offset_minutes: None,
            source: TimeSource::FileModified,
        });
        assert_eq!(p.file_mtime_ms(), Some(1_000));
        assert!(!crate::batch::view::time_is_fallback(&p) == false);
    }

    /// ...and an EXIF time must not be mistaken for one.
    #[test]
    fn an_exif_time_is_not_a_fallback() {
        let mut p = PhotoMeta::default();
        p.capture_time = Some(CaptureTime {
            unix_ms: 1_000,
            subsec_resolution_ms: 10,
            offset_minutes: Some(-360),
            source: TimeSource::Exif,
        });
        assert_eq!(p.file_mtime_ms(), None);
        assert!(!crate::batch::view::time_is_fallback(&p));
    }

    /// The dump is sorted, so the same folder scanned twice is byte-identical (REV-17).
    #[test]
    fn the_dump_is_sorted_by_path() {
        let mut a = PhotoMeta::default();
        a.rel_path = "IMG_0002.CR3".into();
        let mut b = PhotoMeta::default();
        b.rel_path = "IMG_0001.CR3".into();
        let dump = ScanDump::new(vec![a, b], vec![]);
        assert_eq!(dump.schema, "photo-meta/1");
        assert_eq!(dump.photos[0].rel_path, "IMG_0001.CR3");
    }

    /// A photo with no serial, no time or no shutter count has no fingerprint, and must say so
    /// rather than hash something incomplete into an identity that collides with another photo's.
    #[test]
    fn a_fingerprint_needs_every_component() {
        let mut p = PhotoMeta::default();
        assert!(p.fingerprint().is_none(), "nothing at all");
        p.camera_serial = Some("1".into());
        assert!(p.fingerprint().is_none(), "serial but no time");
        p.capture_time = Some(CaptureTime {
            unix_ms: 5,
            subsec_resolution_ms: 10,
            offset_minutes: None,
            source: TimeSource::Exif,
        });
        assert!(p.fingerprint().is_none(), "no shutter count");
        p.shutter_count = Some(7);
        p.file_size = 100;
        let fp = p.fingerprint().expect("complete");
        assert_eq!(fp.shutter_count, 7);
        assert_eq!(fp.camera_serial, "1");
    }
}
