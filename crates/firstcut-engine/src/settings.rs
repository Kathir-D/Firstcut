//! Settings persistence, a port of v1 `SettingsModel.swift` + `SettingsStore`
//! (docs/v2-plan.md §4.3; lands in M1 so every surface can outlive its process).
//!
//! The file is v1's `settings.json` in Application Support/Firstcut: a user who upgrades from v1
//! keeps every setting. Three translation rules make that true (v1 §"Optional keys"):
//!
//! * **Every key is optional.** A file written before a setting existed still reads, and keeps
//!   everything it *does* say (v1's `decodeIfPresent … ?? default`, field by field).
//! * **Any value the file cannot express resets everything.** v1's decoder threw on an unknown
//!   enum case and the `try?` around it fell back to a fresh `AppSettings()` — not just the
//!   broken section. [`SettingsStore::at`] mirrors that all-or-nothing rule.
//! * **Enums with associated values are keyed by their Swift case name**
//!   (`{"moveToSubfolder": {"_0": "_Not kept"}}`), measured against the Swift runtime rather than
//!   guessed, because Swift's synthesized `Codable` refuses a bare case name when any sibling
//!   case carries a payload.
//!
//! Unreadable / unparseable files produce defaults silently (v1's `try? Data(contentsOf:)`), and
//! saves go through a temp file + rename, so a crash mid-write loses nothing that was there.

use std::{
    fmt,
    io::Write,
    path::{Path, PathBuf},
};

use firstcut_types::RatingMode;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value as Json};

use crate::model::{ArrowBehavior, EnteringBatchBehavior, GeneralSettings};

// ── the file's shape ────────────────────────────────────────────────────────

/// Everything v1 wrote, verbatim. The model reads the `general` half into
/// [`GeneralSettings`]; the other halves ride along untouched until their surfaces exist.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsJson {
    #[serde(default = "default_version")]
    pub version: u64,
    #[serde(default)]
    pub general: GeneralJson,
    #[serde(default)]
    pub viewer: ViewerJson,
    #[serde(default)]
    pub metadata: MetadataJson,
    #[serde(default)]
    pub performance: PerformanceJson,
}

fn default_version() -> u64 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralJson {
    #[serde(default = "default_rating_mode")]
    pub rating_mode: RatingModeJson,
    #[serde(default)]
    pub auto_advance: bool,
    #[serde(default = "default_arrow_behavior")]
    pub arrow_behavior_at_batch_end: ArrowBehaviorJson,
    #[serde(default = "default_entering_behavior")]
    pub entering_batch_behavior: EnteringBatchBehaviorJson,
    #[serde(default = "default_unkept")]
    pub finish_unkept: UnkeptActionJson,
    #[serde(default = "default_kept")]
    pub finish_kept: KeptActionJson,
    #[serde(default = "yes")]
    pub confirm_before_finish: bool,
    #[serde(default = "yes")]
    pub confirm_permanent_delete: bool,
    #[serde(default = "default_keep_threshold")]
    pub keep_threshold: u8,
    #[serde(default)]
    pub reject_whole_batch: bool,
}

impl Default for GeneralJson {
    fn default() -> Self {
        Self {
            rating_mode: RatingModeJson::Stars,
            auto_advance: false,
            arrow_behavior_at_batch_end: ArrowBehaviorJson::ContinueIntoNextBatch,
            entering_batch_behavior: EnteringBatchBehaviorJson::FirstPhoto,
            finish_unkept: UnkeptActionJson::default(),
            finish_kept: KeptActionJson::Nothing,
            confirm_before_finish: true,
            confirm_permanent_delete: true,
            keep_threshold: default_keep_threshold(),
            reject_whole_batch: false,
        }
    }
}

fn default_rating_mode() -> RatingModeJson {
    RatingModeJson::Stars
}

fn default_arrow_behavior() -> ArrowBehaviorJson {
    ArrowBehaviorJson::ContinueIntoNextBatch
}

fn default_entering_behavior() -> EnteringBatchBehaviorJson {
    EnteringBatchBehaviorJson::FirstPhoto
}

fn default_unkept() -> UnkeptActionJson {
    UnkeptActionJson::default()
}

fn default_kept() -> KeptActionJson {
    KeptActionJson::Nothing
}

fn yes() -> bool {
    true
}

fn default_keep_threshold() -> u8 {
    4
}

/// v1's `String`-rawValue enums ride JSON as bare case names; serde's `camelCase` renaming
/// reproduces each of them exactly (`ContinueIntoNextBatch` → `"continueIntoNextBatch"`). An
/// unknown name is an error, which is what makes the all-or-nothing rule reachable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RatingModeJson {
    Stars,
    Keep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArrowBehaviorJson {
    Stop,
    ContinueIntoNextBatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnteringBatchBehaviorJson {
    FirstPhoto,
    LastViewed,
}

/// v1's `ColorLabel: String, Codable` raw values are lower-case — note they are NOT the
/// capitalized `xmp:Label` spellings the engine's `ColorLabel::as_str` writes into sidecars.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorLabelJson {
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

/// What a keep is written as in XMP (v1 `KeepMapping`): a rating or a colour label, as the
/// two-word shape `{"type": "rating", "stars": n}` / `{"type": "colorLabel", "label": "red"}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum KeepMappingJson {
    #[serde(rename = "rating")]
    Rating {
        #[serde(default = "default_mapping_stars")]
        stars: u8,
    },
    #[serde(rename = "colorLabel")]
    ColorLabelVariant {
        #[serde(default = "default_mapping_label")]
        label: ColorLabelJson,
    },
}

fn default_mapping_stars() -> u8 {
    1
}

fn default_mapping_label() -> ColorLabelJson {
    ColorLabelJson::Red
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum XmpWriteModeJson {
    #[default]
    Merge,
    Overwrite,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataJson {
    #[serde(default = "yes")]
    pub write_xmp: bool,
    #[serde(default = "default_keep_mapping")]
    pub keep_mapping: KeepMappingJson,
    #[serde(default)]
    pub xmp_write_mode: XmpWriteModeJson,
    /// Writing into DNG is off by default: it modifies the original file (todo.md §11).
    #[serde(default)]
    pub write_ratings_into_dng: bool,
    #[serde(default = "yes")]
    pub write_sidecars_for_jpegs: bool,
}

fn default_keep_mapping() -> KeepMappingJson {
    KeepMappingJson::Rating { stars: 1 }
}

impl Default for MetadataJson {
    fn default() -> Self {
        Self {
            write_xmp: true,
            keep_mapping: default_keep_mapping(),
            xmp_write_mode: XmpWriteModeJson::Merge,
            write_ratings_into_dng: false,
            write_sidecars_for_jpegs: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerJson {
    #[serde(default = "default_background_gray")]
    pub background_gray: f64,
    #[serde(default)]
    pub zoom_lock: bool,
    #[serde(default)]
    pub af_overlay: bool,
    #[serde(default = "default_clipping_shadow")]
    pub clipping_shadow_threshold: f64,
    #[serde(default = "default_clipping_highlight")]
    pub clipping_highlight_threshold: f64,
    #[serde(default = "all_info_fields")]
    pub info_fields: Vec<InfoFieldJson>,
    #[serde(default = "yes")]
    pub hud_visible: bool,
    /// T4 "Exact RAW" decode default (the pipeline's job once it exists; M2).
    #[serde(default)]
    pub exact_raw: bool,
}

fn default_background_gray() -> f64 {
    0.12
}

fn default_clipping_shadow() -> f64 {
    5.0 / 255.0
}

fn default_clipping_highlight() -> f64 {
    250.0 / 255.0
}

fn all_info_fields() -> Vec<InfoFieldJson> {
    vec![
        InfoFieldJson::FileName,
        InfoFieldJson::CaptureTime,
        InfoFieldJson::Camera,
        InfoFieldJson::Lens,
        InfoFieldJson::FocalLength,
        InfoFieldJson::Shutter,
        InfoFieldJson::Aperture,
        InfoFieldJson::Iso,
        InfoFieldJson::ExposureCompensation,
        InfoFieldJson::Metering,
        InfoFieldJson::AfMode,
        InfoFieldJson::DriveMode,
        InfoFieldJson::ShutterMode,
        InfoFieldJson::ShutterCount,
        InfoFieldJson::Dimensions,
        InfoFieldJson::FileSize,
        InfoFieldJson::FolderPath,
        InfoFieldJson::Rating,
        InfoFieldJson::Batch,
        InfoFieldJson::Histogram,
    ]
}

impl Default for ViewerJson {
    fn default() -> Self {
        Self {
            background_gray: default_background_gray(),
            zoom_lock: false,
            af_overlay: false,
            clipping_shadow_threshold: default_clipping_shadow(),
            clipping_highlight_threshold: default_clipping_highlight(),
            info_fields: all_info_fields(),
            hud_visible: true,
            exact_raw: false,
        }
    }
}

/// The info-panel fields (v1 §9.5). A `Set` at source: order-agnostic on the way in, stable
/// declaration order on the way out — a v1 file keeps every value, never a v1 hash order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InfoFieldJson {
    FileName,
    CaptureTime,
    Camera,
    Lens,
    FocalLength,
    Shutter,
    Aperture,
    Iso,
    ExposureCompensation,
    Metering,
    AfMode,
    DriveMode,
    ShutterMode,
    ShutterCount,
    Dimensions,
    FileSize,
    FolderPath,
    Rating,
    Batch,
    Histogram,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceJson {
    /// 40% of physical RAM (todo.md §7.1); the M2 budget multiplies it by the real amount.
    #[serde(default = "default_memory_budget_fraction")]
    pub memory_budget_fraction: f64,
    #[serde(default = "default_look_ahead")]
    pub look_ahead_batches: u32,
    #[serde(default = "default_thumbnail_pixels")]
    pub thumbnail_pixels: u32,
    /// 0 = auto; the measured knee beats any fixed core count (v1 `decodeThreads`).
    #[serde(default)]
    pub decode_threads: u32,
    /// The key is `debugHUD` in v1 — all caps, not `debugHud` (measured bytes).
    #[serde(default, rename = "debugHUD")]
    pub debug_hud: bool,
}

fn default_memory_budget_fraction() -> f64 {
    0.40
}

fn default_look_ahead() -> u32 {
    2
}

fn default_thumbnail_pixels() -> u32 {
    256
}

impl Default for PerformanceJson {
    fn default() -> Self {
        Self {
            memory_budget_fraction: default_memory_budget_fraction(),
            look_ahead_batches: default_look_ahead(),
            thumbnail_pixels: default_thumbnail_pixels(),
            decode_threads: 0,
            debug_hud: false,
        }
    }
}

// ── Swift's keyed enum shape, for the Finish actions ────────────────────────

/// What Finish does with a not-kept photo (v1 `UnkeptAction`; M3 consumes it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnkeptActionJson {
    MarkRejectedInXmp,
    MoveToSubfolder(String),
    MoveToTrash,
    DeletePermanently,
    Nothing,
}

impl Default for UnkeptActionJson {
    /// v1's `UnkeptAction.default`: `_Not kept`, moved (as a subfolder) by the finish run.
    fn default() -> Self {
        Self::MoveToSubfolder("_Not kept".to_string())
    }
}

impl UnkeptActionJson {
    /// `(case name, payload)` in v1's spelling, for the shared writer.
    fn case_name(&self) -> (&'static str, Option<&String>) {
        match self {
            Self::MarkRejectedInXmp => ("markRejectedInXmp", None),
            Self::MoveToSubfolder(path) => ("moveToSubfolder", Some(path)),
            Self::MoveToTrash => ("moveToTrash", None),
            Self::DeletePermanently => ("deletePermanently", None),
            Self::Nothing => ("nothing", None),
        }
    }

    fn from_case(name: &str, value: &Json) -> Result<Self, String> {
        let payload = |case: &Json| -> Option<String> {
            case.as_object().and_then(|fields| fields.get("_0")).and_then(Json::as_str).map(|path| path.to_string())
        };
        match name {
            "markRejectedInXmp" | "moveToTrash" | "deletePermanently" | "nothing" if value.as_object().is_some() => match name {
                "markRejectedInXmp" => Ok(Self::MarkRejectedInXmp),
                "moveToTrash" => Ok(Self::MoveToTrash),
                "deletePermanently" => Ok(Self::DeletePermanently),
                _ => Ok(Self::Nothing),
            },
            "moveToSubfolder" => Ok(Self::MoveToSubfolder(payload(value).ok_or("moveToSubfolder: expected a folder name (_0)")?)),
            _ => Err(format!("unknown unkept action case {name:?}")),
        }
    }
}

impl Serialize for UnkeptActionJson {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut case = Map::new();
        match self.case_name() {
            (name, None) => {
                case.insert(name.to_string(), Json::Object(Map::new()));
            }
            (name, Some(path)) => {
                let mut fields = Map::new();
                fields.insert("_0".to_string(), Json::String(path.clone()));
                case.insert(name.to_string(), Json::Object(fields));
            }
        }
        Json::Object(case).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for UnkeptActionJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_keyed_case(deserializer, "unkept action", UnkeptActionJson::from_case)
    }
}

/// What Finish does with kept photos (v1 `KeptAction`; M3 consumes it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeptActionJson {
    CopyTo(String),
    MoveTo(String),
    SplitByTier(String),
    SplitByStars(String),
    WriteList(String),
    Nothing,
}

impl KeptActionJson {
    fn case_name(&self) -> (&'static str, Option<&String>) {
        match self {
            Self::CopyTo(path) => ("copyTo", Some(path)),
            Self::MoveTo(path) => ("moveTo", Some(path)),
            Self::SplitByTier(path) => ("splitByTier", Some(path)),
            Self::SplitByStars(path) => ("splitByStars", Some(path)),
            Self::WriteList(path) => ("writeList", Some(path)),
            Self::Nothing => ("none", None),
        }
    }

    fn from_case(name: &str, value: &Json) -> Result<Self, String> {
        let payload = |case: &Json| -> Option<String> {
            case.as_object().and_then(|fields| fields.get("_0")).and_then(Json::as_str).map(|path| path.to_string())
        };
        match name {
            "none" if value.as_object().is_some() => Ok(Self::Nothing),
            "copyTo" => Ok(Self::CopyTo(payload(value).ok_or("copyTo: expected a folder path (_0)")?)),
            "moveTo" => Ok(Self::MoveTo(payload(value).ok_or("moveTo: expected a folder path (_0)")?)),
            "splitByTier" => Ok(Self::SplitByTier(payload(value).ok_or("splitByTier: expected a folder path (_0)")?)),
            "splitByStars" => Ok(Self::SplitByStars(payload(value).ok_or("splitByStars: expected a folder path (_0)")?)),
            "writeList" => Ok(Self::WriteList(payload(value).ok_or("writeList: expected a file path (_0)")?)),
            _ => Err(format!("unknown kept action case {name:?}")),
        }
    }
}

impl Serialize for KeptActionJson {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut case = Map::new();
        match self.case_name() {
            (name, None) => {
                case.insert(name.to_string(), Json::Object(Map::new()));
            }
            (name, Some(path)) => {
                let mut fields = Map::new();
                fields.insert("_0".to_string(), Json::String(path.clone()));
                case.insert(name.to_string(), Json::Object(fields));
            }
        }
        Json::Object(case).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for KeptActionJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_keyed_case(deserializer, "kept action", KeptActionJson::from_case)
    }
}

/// Both keyed enums share one picture of Swift's bytes: an object with exactly one key, whose
/// value is an object (`{}` for a unit case, `{"_0": "…"}` for the payload one).
fn deserialize_keyed_case<'de, D: Deserializer<'de>, T>(
    deserializer: D,
    what: &'static str,
    from_case: impl Fn(&str, &Json) -> Result<T, String>,
) -> Result<T, D::Error> {
    let value = Json::deserialize(deserializer)?;
    let case = value.as_object().ok_or_else(|| serde::de::Error::custom(format!("{what} must be a keyed object")))?;
    let mut entries = case.iter();
    let Some((name, payload)) = entries.next() else {
        return Err(serde::de::Error::custom(format!("{what} names no case")));
    };
    if entries.next().is_some() {
        return Err(serde::de::Error::custom(format!("{what} names more than one case")));
    }
    from_case(name, payload).map_err(serde::de::Error::custom)
}

// ── translation into what the model reads ───────────────────────────────────

impl AppSettingsJson {
    /// The General half as [`GeneralSettings`], the shape the model works in. `keep_threshold`
    /// is v1's name for the same thing the model calls `keep_stars`.
    pub fn general_settings(&self) -> GeneralSettings {
        let general = &self.general;
        GeneralSettings {
            rating_mode: match general.rating_mode {
                RatingModeJson::Stars => RatingMode::Stars,
                RatingModeJson::Keep => RatingMode::KeepNotKeep,
            },
            auto_advance: general.auto_advance,
            arrow_behavior: match general.arrow_behavior_at_batch_end {
                ArrowBehaviorJson::Stop => ArrowBehavior::Stop,
                ArrowBehaviorJson::ContinueIntoNextBatch => ArrowBehavior::ContinueIntoNextBatch,
            },
            entering_behavior: match general.entering_batch_behavior {
                EnteringBatchBehaviorJson::FirstPhoto => EnteringBatchBehavior::FirstPhoto,
                EnteringBatchBehaviorJson::LastViewed => EnteringBatchBehavior::LastViewed,
            },
            reject_whole_batch: general.reject_whole_batch,
            keep_stars: general.keep_threshold,
        }
    }

    /// Re-places in the file exactly what the model changed, keeping everything else. The
    /// finish/confirm keys stay as loaded: their surfaces do not exist until M3, and persisting a
    /// loaded field is better than persisting a reset one.
    pub fn set_general_from(&mut self, general: GeneralSettings) {
        self.general.rating_mode = match general.rating_mode {
            RatingMode::Stars => RatingModeJson::Stars,
            RatingMode::KeepNotKeep => RatingModeJson::Keep,
        };
        self.general.auto_advance = general.auto_advance;
        self.general.arrow_behavior_at_batch_end = match general.arrow_behavior {
            ArrowBehavior::Stop => ArrowBehaviorJson::Stop,
            ArrowBehavior::ContinueIntoNextBatch => ArrowBehaviorJson::ContinueIntoNextBatch,
        };
        self.general.entering_batch_behavior = match general.entering_behavior {
            EnteringBatchBehavior::FirstPhoto => EnteringBatchBehaviorJson::FirstPhoto,
            EnteringBatchBehavior::LastViewed => EnteringBatchBehaviorJson::LastViewed,
        };
        self.general.keep_threshold = general.keep_stars;
        self.general.reject_whole_batch = general.reject_whole_batch;
    }

    /// The sidecar switches as the session takes them. (v1's `XMPWriteMode`, `KeepMapping`,
    /// and `writeRatingsIntoDng` reach the writer through later plumbing.)
    pub fn session_xmp(&self) -> crate::session::XmpSettings {
        crate::session::XmpSettings { write_sidecars: self.metadata.write_xmp, sidecars_for_non_raw: self.metadata.write_sidecars_for_jpegs }
    }
}

// ── the store ───────────────────────────────────────────────────────────────

/// Where the settings file lives: the Application Support directory the sessions directory also
/// sits in, which is v1's `KeymapStore.defaultDirectory` (`…/Firstcut/settings.json`).
pub fn settings_path_in(home: &Path) -> PathBuf {
    let mut directory = firstcut_catalog::db::default_sessions_dir(home);
    directory.pop(); // …/Firstcut/Sessions → …/Firstcut
    directory.join("settings.json")
}

#[derive(Debug)]
pub enum SettingsError {
    /// Shown, never thrown into the user's photo tree (the "never crash outranks feature" rule).
    Write { path: String, source: std::io::Error },
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Write { path, source } => write!(f, "writing {path}: {source}"),
        }
    }
}

/// The loaded file plus the path it came from, so the model's "one place persists" (v1's
/// `updateSettings`) writes back the file it read, with everything it does not interpret intact.
#[derive(Debug)]
pub struct SettingsStore {
    path: PathBuf,
    current: AppSettingsJson,
}

impl SettingsStore {
    /// Reads `path`, falling back to defaults the way v1's `try?` did: an unreadable file is a
    /// fresh `AppSettings()`, whoever's fault the unreadability is.
    pub fn at(path: &Path) -> Self {
        let current = std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice::<AppSettingsJson>(&bytes).ok()).unwrap_or_default();
        Self { path: path.to_path_buf(), current }
    }

    /// The file's settings as they stand, either from disk or the fallback above.
    pub fn current(&self) -> &AppSettingsJson {
        &self.current
    }

    /// The General half, ready for the model or whoever asks first.
    pub fn general(&self) -> GeneralSettings {
        self.current.general_settings()
    }

    /// Replaces the general half and writes the file. The other halves are exactly what was
    /// loaded, so a v1 user's finish actions outlive this version of the app.
    pub fn set_general_from(&mut self, general: GeneralSettings) -> Result<(), SettingsError> {
        self.current.set_general_from(general);
        self.save()
    }

    /// Replaces one half the Settings screens edit. The halves they do not touch stay exactly as
    /// loaded (a v1 file's Finish actions are never rewritten because a checkbox moved).
    pub fn set_viewer(&mut self, viewer: ViewerJson) -> Result<(), SettingsError> {
        self.current.viewer = viewer;
        self.save()
    }

    pub fn set_metadata(&mut self, metadata: MetadataJson) -> Result<(), SettingsError> {
        self.current.metadata = metadata;
        self.save()
    }

    pub fn set_performance(&mut self, performance: PerformanceJson) -> Result<(), SettingsError> {
        self.current.performance = performance;
        self.save()
    }

    /// One temp file + rename, so a crash mid-save leaves the previous file intact.
    pub fn save(&self) -> Result<(), SettingsError> {
        let path_string = self.path.display().to_string();
        let bytes = serde_json::to_vec_pretty(&self.current)
            .map_err(|_| io_message("could not serialize the settings"))
            .map_err(|source| SettingsError::Write { path: path_string.clone(), source })?;
        let directory = self
            .path
            .parent()
            .ok_or_else(|| io_message("settings: no directory to write in"))
            .map_err(|source| SettingsError::Write { path: path_string.clone(), source })?;
        let temporary =
            tempfile::Builder::new().tempfile_in(directory).map_err(|source| SettingsError::Write { path: path_string.clone(), source })?;
        (&temporary).write_all(&bytes).map_err(|source| SettingsError::Write { path: path_string.clone(), source })?;
        temporary.persist(&self.path).map(|_| ()).map_err(|error| SettingsError::Write { path: path_string, source: error.error })
    }
}

fn io_message(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

// ── tests: the file contract against Swift's own bytes ───────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A settings file inside a live temp directory. The guard must outlive the store: dropping
    /// it deletes the directory the file lives in.
    fn store_with(json: &str) -> (tempfile::TempDir, SettingsStore) {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("settings.json");
        std::fs::write(&file, json).unwrap();
        let store = SettingsStore::at(&file);
        (directory, store)
    }

    /// The compact spelling, exactly what the substring assertions pin.
    fn dump(store: &SettingsStore) -> String {
        serde_json::to_string(store.current()).unwrap()
    }

    #[test]
    fn an_old_file_still_reads_every_setting_it_names() {
        // A file written before, say, `confirmPermanentDelete` existed. What it says it keeps;
        // the rest is v1's defaults — never a bulk reset of General.
        let (_guard, store) = store_with("{\n  \"general\" : {\"ratingMode\" : \"keep\", \"keepThreshold\" : 5}\n}");
        let general = &store.current.general;
        assert_eq!(general.rating_mode, RatingModeJson::Keep);
        assert_eq!(general.keep_threshold, 5);
        assert!(general.confirm_permanent_delete, "a newer key rides v1's default");
        assert!(!general.auto_advance);
        assert_eq!(general.finish_unkept, UnkeptActionJson::MoveToSubfolder("_Not kept".to_string()));
        assert_eq!(store.current.version, 1);
    }

    #[test]
    fn a_future_key_is_ignored_not_fatal() {
        let (_guard, store) = store_with(r#"{"version":1,"general":{"ratingMode":"stars","someSettingFromV2":true},"viewer":{"neverHeardOfIt":3}}"#);
        assert_eq!(store.current.general.rating_mode, RatingModeJson::Stars);
        assert_eq!(store.current.viewer, ViewerJson::default(), "unknown keys ride their defaults");
    }

    #[test]
    fn the_swift_keyed_shape_is_the_file_shape() {
        // The measured Swift bytes: {"case": {}} for a unit case and {"_0": …} for the payload
        // one — never a bare string, which Swift's decoder rejects when any case has a payload.
        let (_guard, store) = store_with(r#"{"general":{"finishUnkept":{"moveToSubfolder":{"_0":"_Not kept"}},"finishKept":{"none":{}}}}"#);
        assert_eq!(store.current.general.finish_unkept, UnkeptActionJson::MoveToSubfolder("_Not kept".to_string()));
        assert_eq!(store.current.general.finish_kept, KeptActionJson::Nothing);
        let written = dump(&store);
        assert!(written.contains(r#""finishUnkept":{"moveToSubfolder":{"_0":"_Not kept"}}"#), "{written}");
        assert!(written.contains(r#""finishKept":{"none":{}}"#), "{written}");
    }

    #[test]
    fn an_unreadable_value_resets_everything_the_way_v1_did() {
        // v1's one-bad-key rule: the whole AppSettings falls back, not just the broken section.
        let (_guard, store) = store_with(r#"{"general":{"ratingMode":"stars5","autoAdvance":true},"viewer":{"zoomLock":true}}"#);
        assert_eq!(store.current, AppSettingsJson::default());
        assert!(!store.current.general.auto_advance, "auto advance did not survive its neighbour's garbage");
    }

    #[test]
    fn a_garbage_file_resets_to_defaults() {
        let (_guard, store) = store_with("not json at all");
        assert_eq!(store.current, AppSettingsJson::default());
    }

    #[test]
    fn a_full_file_round_trips() {
        let general = GeneralSettings {
            rating_mode: RatingMode::KeepNotKeep,
            keep_stars: 5,
            arrow_behavior: ArrowBehavior::Stop,
            entering_behavior: EnteringBatchBehavior::LastViewed,
            auto_advance: true,
            reject_whole_batch: true,
        };

        let mut settings = AppSettingsJson::default();
        settings.metadata.keep_mapping = KeepMappingJson::ColorLabelVariant { label: ColorLabelJson::Green };
        settings.set_general_from(general);

        let directory = tempfile::tempdir().unwrap(); // held: the save lands inside it
        let file = directory.path().join("settings.json");
        SettingsStore { path: file.clone(), current: settings }.save().unwrap();

        let reloaded = SettingsStore::at(&file);
        assert_eq!(reloaded.general(), general, "the model's general half travels through the file");
        match reloaded.current.metadata.keep_mapping {
            KeepMappingJson::ColorLabelVariant { label } => assert_eq!(label, ColorLabelJson::Green),
            other => panic!("the keep mapping did not survive: {other:?}"),
        }
    }

    #[test]
    fn the_session_takes_the_sidecar_switches() {
        let (_guard, store) = store_with(r#"{"metadata":{"writeXmp":false,"writeSidecarsForJpegs":false}}"#);
        let xmp = store.current.session_xmp();
        assert!(!xmp.write_sidecars);
        assert!(!xmp.sidecars_for_non_raw);
    }

    /// `set_general_from` replaces exactly the model-owned keys and preserves the M3 ones a v1
    /// file carries — the point of the "everything else untouched" rule.
    #[test]
    fn a_save_keeps_a_v1_users_finish_choices() {
        let (guard, mut store) = store_with(r#"{"general":{"finishUnkept":{"moveToTrash":{}},"confirmBeforeFinish":false}}"#);
        store.current.set_general_from(GeneralSettings::default());
        store.save().unwrap();

        // Re-read through the store: the finish action outlives a settings change untouched.
        let reloaded = SettingsStore::at(&guard.path().join("settings.json"));
        assert_eq!(reloaded.current.general.finish_unkept, UnkeptActionJson::MoveToTrash);
        assert!(!reloaded.current.general.confirm_before_finish);
        assert_eq!(reloaded.current.general.finish_kept, KeptActionJson::Nothing, "a key the file did not name is the default, not a reset");
        // And the bytes are Swift's keyed shape, not a bare string.
        let written = serde_json::to_string(reloaded.current()).unwrap();
        assert!(written.contains(r#""finishUnkept":{"moveToTrash":{}}"#), "{written}");
    }

    /// The model seeds from the store: the surfaces' construction order, load, attach, then
    /// setters persist as they go.
    #[test]
    fn the_model_seeds_from_the_store_and_persists_changes() {
        let (guard, store) = store_with(r#"{"general":{"ratingMode":"keep","keepThreshold":5}}"#);
        let mut model = crate::model::Model::new();
        model.attach_settings_store(store);
        assert_eq!(model.settings().rating_mode, RatingMode::KeepNotKeep);
        assert_eq!(model.settings().keep_stars, 5, "v1's keepThreshold is the model's keep stars");

        model.set_rating_mode(RatingMode::Stars);
        let reloaded = SettingsStore::at(&guard.path().join("settings.json"));
        let general = &reloaded.current.general;
        assert_eq!(general.rating_mode, RatingModeJson::Stars, "the setter reached the file");
        assert_eq!(general.keep_threshold, 5, "a setting the model did not touch is untouched");
    }

    /// The real bytes a v1 app wrote on this machine (`…"language"…`-free, 1470 bytes), pinned as
    /// a fixture so the port's shapes are checked against what Swift's encoder actually emitted —
    /// not against what the port believes it would emit. Boundaries are never synthesised.
    #[test]
    fn the_real_v1_file_every_section_loads() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/v1-settings.json");
        let raw = std::fs::read_to_string(path).expect("the real v1 settings fixture is in the repo");
        let settings: AppSettingsJson = serde_json::from_str(&raw).unwrap_or_default();

        // General.
        assert_eq!(settings.general.rating_mode, RatingModeJson::Keep);
        assert_eq!(settings.general.keep_threshold, 4);
        assert!(settings.general.reject_whole_batch, "the user finished rejects whole batches");
        assert!(settings.general.auto_advance);
        assert_eq!(settings.general.finish_unkept, UnkeptActionJson::MoveToSubfolder("_Not kept".to_string()));
        assert_eq!(settings.general.finish_kept, KeptActionJson::Nothing);

        // Metadata.
        assert_eq!(settings.metadata.keep_mapping, KeepMappingJson::Rating { stars: 5 });
        assert_eq!(settings.metadata.xmp_write_mode, XmpWriteModeJson::Merge);
        assert!(settings.metadata.write_sidecars_for_jpegs);
        assert!(!settings.metadata.write_ratings_into_dng);

        // Viewer — including the two keys a synthesised file would likely have omitted.
        assert_eq!(settings.viewer.background_gray, 0.12);
        assert!(settings.viewer.hud_visible);
        assert!(settings.viewer.exact_raw);
        assert_eq!(settings.viewer.clipping_highlight_threshold, 1.0);
        assert_eq!(settings.viewer.info_fields.len(), 20);

        // Performance — the all-caps key the camelCase rule would have misnamed.
        assert!(!settings.performance.debug_hud);
        assert_eq!(settings.performance.decode_threads, 0);
        assert_eq!(settings.performance.look_ahead_batches, 2);
        assert_eq!(settings.performance.thumbnail_pixels, 256);
        assert_eq!(settings.performance.memory_budget_fraction, 0.4);
        assert_eq!(settings.version, 1);

        // And the round trip emits the same keyed shapes back (a save must not lose the user's).
        let rewritten = serde_json::to_string(&settings).unwrap_or_default();
        assert!(
            rewritten.contains(r#""finishUnkept":{"moveToSubfolder":{"_0":"_Not kept"}}"#)
                && rewritten.contains(r#""debugHUD":false"#)
                && rewritten.contains(r#""keepMapping":{"type":"rating","stars":5}"#),
            "the round trip keeps v1's shapes: {rewritten}",
        );
    }
}
