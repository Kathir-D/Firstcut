//! Owner: core-meta. `PhotoMeta` and the types that cross the FFI (docs/contracts/photo-meta.md).
//!
//! This is the **one** definition of these types. They used to exist twice -- here-adjacent in
//! `batch/fixture.rs` as core-batch's stand-in, and again in the CLI crate's `fixtures.rs` -- which
//! meant a scanner in the library could not produce the same struct the CLI and the tests read.
//! Two definitions of `PhotoMeta` is a permanent adapter, and the adapters are where the bugs live
//! (REV-56). `batch::fixture` now re-exports these rather than declaring its own.
//!
//! JSON is **camelCase** with `null` for absent fields (`serde(rename_all = "camelCase")` on every
//! type here), matching `App/Sources/Shared/CoreTypes.swift`, so a Swift decoder can read a dump
//! with no adapter of its own. `ScanDump` is the versioned envelope the fixture files use:
//! `{"schema": "photo-meta/1", "photos": [...]}`, so the format can change later without every
//! consumer having to guess which version it is holding.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::batch::view::Photo;
use crate::batch::{PhotoId, photo_id};

/// Capture time, normalised to UTC, at the resolution the camera actually recorded.
///
/// `subsec_resolution_ms` is the *camera's* resolution, not a convenient one: a Canon R8 records
/// 10 ms, and 840 ms written as "84" means something different from 84 ms. The batcher has to round
/// every Δt to this before comparing against a threshold, or a rounding artefact invents a burst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTime {
    /// Milliseconds since the Unix epoch, UTC.
    pub unix_ms: i64,
    /// 10 for a Canon R8, 1000 when the file carries no sub-seconds.
    pub subsec_resolution_ms: u16,
    /// Minutes east of UTC, as recorded. Kept so the UI can show the photographer's local time.
    pub offset_minutes: Option<i16>,
    pub source: TimeSource,
}

/// Where a timestamp came from. `FileModified` is a *fallback*: it is rewritten by every copy and
/// restore, so a pair that relies on it can never be a hard join (REV-63).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeSource {
    Exif,
    FileModified,
}

/// Everything a header parse can produce for one photo.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoMeta {
    pub id: PhotoId,
    /// Path relative to the session folder. This is a *display and grouping* path; identity for
    /// rename-reconciliation is the fingerprint below.
    pub rel_path: String,
    /// Image files that are part of this photo: a RAW+JPEG pair is one photo (task.md §5.4).
    #[serde(default)]
    pub companions: Vec<String>,
    pub kind: FileKind,
    pub file_size: u64,
    pub capture_time: Option<CaptureTime>,
    /// Canon's mechanical+electronic shutter count. Strictly monotonic in capture time on all four
    /// test games, so it is ordering evidence and a soft boundary signal (deleted frames).
    pub shutter_count: Option<u64>,
    pub file_number: Option<u32>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    /// The durable part of a photo's identity: a rename must not lose the rating.
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
    /// Byte offset and length of the embedded preview JPEG, so Swift can read the bytes directly
    /// rather than re-parsing the container (task.md §7.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<PreviewSpan>,
    /// Anything that went wrong reading this file. A file is never dropped for having one.
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Default for PhotoMeta {
    fn default() -> Self {
        Self {
            id: PhotoId(0),
            rel_path: String::new(),
            companions: Vec::new(),
            kind: FileKind::Jpeg,
            file_size: 0,
            capture_time: None,
            shutter_count: None,
            file_number: None,
            camera_make: None,
            camera_model: None,
            camera_serial: None,
            lens_model: None,
            focal_length_mm: None,
            exposure_time_s: None,
            f_number: None,
            iso: None,
            exposure_comp_ev: None,
            metering_mode: None,
            drive_mode: None,
            shutter_mode: None,
            orientation: 1,
            width: 0,
            height: 0,
            af: None,
            preview: None,
            warnings: Vec::new(),
        }
    }
}

impl PhotoMeta {
    /// The durable identity a rename must preserve: camera, instant, shutter count, size.
    ///
    /// task.md §11 requires a renamed file to keep its rating. Keying on the path cannot do that --
    /// the rename *is* the change -- so the store matches on this instead and falls back to
    /// `rel_path` only when a camera omitted the fields (REV-15, REV-68).
    #[must_use]
    pub fn fingerprint(&self) -> Option<PhotoFingerprint> {
        Some(PhotoFingerprint {
            camera_serial: self.camera_serial.clone()?,
            capture_unix_ms: self.capture_time?.unix_ms,
            shutter_count: self.shutter_count?,
            file_size: self.file_size,
        })
    }
}

/// The part of a photo's identity that survives a rename.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoFingerprint {
    pub camera_serial: String,
    pub capture_unix_ms: i64,
    pub shutter_count: u64,
    pub file_size: u64,
}

/// Where the embedded preview lives, so Swift can `pread` the JPEG without parsing anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSpan {
    pub offset: u64,
    pub length: u64,
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

impl RawFormat {
    /// From a file extension, case-insensitively. `None` for something we do not read, which is
    /// how a folder of JPEGs next to a folder of RAWs is handled without guessing.
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "cr3" | "cr2" | "crw" => Self::Cr3,
            "arw" | "sr2" | "srf" => Self::Arw,
            "nef" | "nrw" => Self::Nef,
            "raf" => Self::Raf,
            "rw2" => Self::Rw2,
            "orf" => Self::Orf,
            "pef" => Self::Pef,
            "dng" => Self::Dng,
            "3fr" => Self::ThreeFr,
            "fff" => Self::Fff,
            "iiq" => Self::Iiq,
            "srw" => Self::Srw,
            "dcr" => Self::Dcr,
            "kdc" => Self::Kdc,
            "erf" => Self::Erf,
            "mef" => Self::Mef,
            "mos" => Self::Mos,
            "gpr" => Self::Gpr,
            "x3f" => Self::X3f,
            _ => return None,
        })
    }

    /// The extension this format is written with. Variants that are the same container with a
    /// different brand of lens (CR2/CRW, SR2/SRF, NRW) share their family's extension, because the
    /// extension is all anything downstream uses it for.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Cr3 | Self::Cr2 | Self::Crw => "cr3",
            Self::Arw | Self::Sr2 | Self::Srf => "arw",
            Self::Nef | Self::Nrw => "nef",
            Self::Raf => "raf",
            Self::Rw2 => "rw2",
            Self::Orf => "orf",
            Self::Pef => "pef",
            Self::Rwl => "rwl",
            Self::Dng => "dng",
            Self::ThreeFr => "3fr",
            Self::Fff => "fff",
            Self::Iiq => "iiq",
            Self::Srw => "srw",
            Self::Dcr => "dcr",
            Self::Kdc => "kdc",
            Self::Erf => "erf",
            Self::Mef => "mef",
            Self::Mos => "mos",
            Self::Gpr => "gpr",
            Self::X3f => "x3f",
        }
    }
}

impl FileKind {
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "heic" | "heif" => Some(Self::Heif),
            "tif" | "tiff" => Some(Self::Tiff),
            "png" => Some(Self::Png),
            other => RawFormat::from_extension(other).map(Self::Raw),
        }
    }

    /// True for the formats §7.4 requires a real parser for. Everything else is best-effort.
    #[must_use]
    pub fn is_raw(self) -> bool {
        matches!(self, Self::Raw(_))
    }
}

/// Autofocus area information, for the AF overlay (task.md §9.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfInfo {
    pub area_mode: String,
    /// **Sensor-space, pre-orientation, and may be negative or outside the frame.** Canon's
    /// `AFAreaXPositions` is a signed value in a 6000×4000 space, and it really is -92 on real
    /// files, so "normalized 0..1" would be a lie and clamping would hide the point. §9.2's
    /// transform has to do that work, and it belongs in the contract (REV-21).
    #[serde(default)]
    pub points: Vec<AfPoint>,
    /// Sensor width/height the coordinates are in, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_height: Option<u32>,
}

/// `AFPointsInFocus` is 0 on many frames in AI Servo, so the overlay draws the *selected* area when
/// there is no in-focus point. Measured, not assumed: see task.md §3.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfPoint {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub in_focus: bool,
}

/// A file the scanner could not read. Task.md §8: a corrupt file still shows up, with a reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFile {
    pub rel_path: String,
    pub reason: String,
}

/// The versioned envelope the committed dumps use. An object rather than a bare array so the
/// format can be versioned (REV-16).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanDump {
    pub schema: String,
    /// Sorted by `rel_path` so the dump is byte-identical between runs of the same folder
    /// (REV-17). Unordered output makes every diff noise.
    pub photos: Vec<PhotoMeta>,
    #[serde(default)]
    pub skipped: Vec<SkippedFile>,
}

impl ScanDump {
    #[must_use]
    pub fn new(photos: Vec<PhotoMeta>, skipped: Vec<SkippedFile>) -> Self {
        let mut photos = photos;
        photos.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        Self {
            schema: "photo-meta/1".into(),
            photos,
            skipped,
        }
    }
}

/// What `scan_folder` returns.
#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub photos: Vec<PhotoMeta>,
    pub skipped: Vec<SkippedFile>,
    pub warnings: Vec<String>,
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
        // The batcher rounds every dt to this before comparing against a threshold, so an unknown
        // resolution must not read as 1 ms (which would invent bursts) or 1000 (which would hide
        // them). Unknown is reported as 1000: the coarser, more forgiving of the two.
        self.capture_time.map_or(1000, |c| c.subsec_resolution_ms)
    }
    fn file_mtime_ms(&self) -> Option<i64> {
        // A photo whose EXIF time was missing falls back to its mtime, and says so: the batcher
        // treats a fallback time as evidence of nothing (REV-63).
        match self.capture_time {
            Some(c) if c.source == TimeSource::FileModified => Some(c.unix_ms),
            _ => None,
        }
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
    pub fn from_photos(mut photos: Vec<PhotoMeta>) -> Self {
        let mut by_path = HashMap::new();
        let mut by_id = HashMap::new();
        for (i, p) in photos.iter().enumerate() {
            by_path.insert(p.rel_path.clone(), i);
            by_id.insert(p.id, i);
        }
        photos.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        // Re-index: sorting moved things.
        let mut folder = Self {
            photos,
            by_path,
            by_id,
        };
        folder.reindex();
        folder
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        // Accept both the versioned envelope and a bare array, so a hand-written fixture works.
        let photos: Vec<PhotoMeta> = match serde_json::from_str::<ScanDump>(&text) {
            Ok(dump) => dump.photos,
            // A bare array is accepted too, so a hand-written one-file fixture just works.
            Err(_) => serde_json::from_str(&text)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
        };
        Ok(Self::from_photos(photos))
    }

    fn reindex(&mut self) {
        self.by_path.clear();
        self.by_id.clear();
        for (i, p) in self.photos.iter().enumerate() {
            self.by_path.insert(p.rel_path.clone(), i);
            self.by_id.insert(p.id, i);
        }
    }

    pub fn get_by_path(&self, path: &str) -> Option<&PhotoMeta> {
        self.by_path.get(path).map(|i| &self.photos[*i])
    }

    pub fn get_by_id(&self, id: PhotoId) -> Option<&PhotoMeta> {
        self.by_id.get(&id).map(|i| &self.photos[*i])
    }
}

/// Stable id from a path, for fixtures built before a folder has been scanned.
#[must_use]
pub fn stable_id(path: &str) -> PhotoId {
    photo_id(path)
}
