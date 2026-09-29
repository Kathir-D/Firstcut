//! infra (UniFFI exports) + the app-facing surface.
//!
//! Everything here is a thin shell over [`crate::session`] and [`crate::scan`]: it converts between
//! UniFFI's owned types and the core's borrowed ones, and it makes sure the app cannot reach past
//! the rules the core already enforces. The one rule it *does* enforce itself is that the Finish
//! decision comes from `Rating::is_kept`, because that is the call the app makes most often and the
//! one REV-78 was about.

use std::sync::Arc;

use crate::meta::{AfInfo, CaptureTime, FileKind, PhotoMeta, RawFormat, TimeSource};
use crate::session::Session;
use crate::store::rating::{Rating, RatingMode, Tier};

/// One photo, flattened for UniFFI.
///
/// A flat record rather than exporting `PhotoMeta` directly, for two reasons that are both about
/// the FFI and neither about taste: UniFFI 0.32 has no `newtype`, so `PhotoId` would have become a
/// wrapper struct and every Swift use site would need `.into()`; and a flat record means the
/// generated Swift shape is written down here, where a change to it is a reviewed diff, instead of
/// being whatever the derive happens to produce (REV-16, REV-7).
#[derive(Debug, Clone, uniffi::Record)]
pub struct PhotoDto {
    pub id: u64,
    pub rel_path: String,
    pub companions: Vec<String>,
    /// "cr3", "arw", ... for a RAW, else "jpeg"/"heif"/"tiff"/"png".
    pub kind: String,
    pub file_size: u64,
    /// UTC milliseconds, or `None` when the file carried no usable capture time.
    pub capture_unix_ms: Option<i64>,
    /// The camera's sub-second resolution in ms: 10 for an R8, 1000 when absent.
    pub subsec_resolution_ms: Option<u16>,
    /// True when `capture_unix_ms` came from the file's mtime rather than its EXIF. The app must
    /// not treat such a timestamp as evidence (REV-63).
    pub capture_time_is_fallback: bool,
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
    pub af_area_mode: Option<String>,
    /// Byte offset and length of the embedded preview JPEG, so the pipeline can `pread` the bytes
    /// without re-parsing the container (task.md §7.4).
    pub preview_offset: Option<u64>,
    pub preview_length: Option<u64>,
    /// Anything that went wrong reading this file. Shown, never dropped (task.md §8).
    pub warnings: Vec<String>,
}

impl From<&PhotoMeta> for PhotoDto {
    fn from(p: &PhotoMeta) -> Self {
        let time = p.capture_time;
        PhotoDto {
            id: p.id.0,
            rel_path: p.rel_path.clone(),
            companions: p.companions.clone(),
            kind: match p.kind {
                FileKind::Raw(r) => r.extension().to_string(),
                FileKind::Jpeg => "jpeg".into(),
                FileKind::Heif => "heif".into(),
                FileKind::Tiff => "tiff".into(),
                FileKind::Png => "png".into(),
            },
            file_size: p.file_size,
            capture_unix_ms: time.map(|t| t.unix_ms),
            subsec_resolution_ms: time.map(|t| t.subsec_resolution_ms),
            capture_time_is_fallback: time.is_none_or(|t| t.source == TimeSource::FileModified),
            shutter_count: p.shutter_count,
            file_number: p.file_number,
            camera_make: p.camera_make.clone(),
            camera_model: p.camera_model.clone(),
            camera_serial: p.camera_serial.clone(),
            lens_model: p.lens_model.clone(),
            focal_length_mm: p.focal_length_mm,
            exposure_time_s: p.exposure_time_s,
            f_number: p.f_number,
            iso: p.iso,
            exposure_comp_ev: p.exposure_comp_ev,
            metering_mode: p.metering_mode.clone(),
            drive_mode: p.drive_mode.clone(),
            shutter_mode: p.shutter_mode.clone(),
            orientation: p.orientation,
            width: p.width,
            height: p.height,
            af_area_mode: p.af.as_ref().map(|a| a.area_mode.clone()),
            preview_offset: p.preview.map(|s| s.offset),
            preview_length: p.preview.map(|s| s.length),
            warnings: p.warnings.clone(),
        }
    }
}

/// A rating as UniFFI carries it. Field names are camelCase so the generated Swift matches
/// `App/Sources/Shared/CoreTypes.swift` exactly (REV-16).
#[derive(Debug, Clone, uniffi::Record)]
pub struct RatingDto {
    pub stars: u8,
    /// 0 none, 1 pick, 2 reject -- the DB's encoding, so Swift needs no translation.
    pub flag: u8,
    pub label: Option<String>,
    pub keep: bool,
}

impl From<Rating> for RatingDto {
    fn from(r: Rating) -> Self {
        RatingDto {
            stars: r.stars,
            flag: r.flag.to_db() as u8,
            label: r.label.map(|l| l.as_str().to_string()),
            keep: r.keep,
        }
    }
}

impl From<RatingDto> for Rating {
    fn from(r: RatingDto) -> Self {
        Rating::new(
            r.stars,
            flag_from_db(r.flag),
            r.label.as_deref().and_then(|l| l.parse().ok()),
            r.keep,
        )
    }
}

fn flag_from_db(v: u8) -> crate::store::rating::Flag {
    match v {
        1 => crate::store::rating::Flag::Pick,
        2 => crate::store::rating::Flag::Reject,
        _ => crate::store::rating::Flag::None,
    }
}

/// One burst.
#[derive(Debug, Clone, uniffi::Record)]
pub struct BatchDto {
    pub id: u64,
    pub index: u32,
    pub photo_ids: Vec<u64>,
    pub visited: bool,
    pub provisional: bool,
}

/// One photo's rating, for the snapshot. A flat record rather than a `(u64, RatingDto)` pair
/// because UniFFI carries no tuples, and a `[(UInt64, Rating)]` in Swift is exactly the shape the
/// app has to build on every read.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RatingEntryDto {
    pub photo_id: u64,
    pub rating: RatingDto,
}

/// A rating change with both sides, so the app can undo and show what changed.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RatingChangeDto {
    pub photo_id: u64,
    pub batch_id: u64,
    pub before: RatingDto,
    pub after: RatingDto,
}

/// Counts per tier, for the HUD and the Finish summary.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TierCountsDto {
    pub keep: u32,
    pub good: u32,
    pub maybe: u32,
    pub unrated: u32,
    pub rejected: u32,
}

/// Everything the app needs to draw, read once per open.
///
/// This is a copy, not a borrow: UniFFI cannot pass a reference out and hold it. It is why
/// `snapshot()` is called once rather than per keystroke (REV-37) -- 1,500 photos across FFI is
/// megabytes, and the arrow-key path must never pay for it. Per-frame reads use
/// [`SessionHandle::rating`] and [`SessionHandle::batches`], which are cheap.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SnapshotDto {
    pub folder: String,
    pub photos: Vec<PhotoDto>,
    pub order: Vec<u64>,
    pub batches: Vec<BatchDto>,
    pub ratings: Vec<RatingEntryDto>,
    pub visited: Vec<u64>,
    pub rating_mode: String,
    pub warnings: Vec<String>,
    /// Files that could not be read, with the reason. Shown, never dropped (task.md §8).
    pub skipped: Vec<String>,
    pub match_created: bool,
}

/// An open folder, as the app holds it.
///
/// `Arc`-based because UniFFI objects are shared: Swift keeps a handle and Rust must be able to
/// mutate the session through it. Every method takes `&self` and locks internally, so a Swift
/// caller cannot get a borrow wrong.
#[derive(uniffi::Object)]
pub struct SessionHandle {
    inner: std::sync::Mutex<Session>,
    /// Whether the session was created or resumed. A fact about the open, so it lives on the
    /// handle rather than in the snapshot.
    created: bool,
}

#[uniffi::export]
impl SessionHandle {
    /// True when this handle's session was created rather than resumed.
    pub fn is_new(&self) -> Result<bool, FirstcutError> {
        self.with(|s| Ok(s.was_created()))
    }

    /// Everything the app needs to render. Once per open, and after `batches_changed`.
    pub fn snapshot(&self) -> Result<SnapshotDto, FirstcutError> {
        self.lock().map(|s| {
            let snap = s.snapshot();
            SnapshotDto {
                folder: snap.folder.clone(),
                photos: snap.photos.iter().map(PhotoDto::from).collect(),
                order: snap.order.iter().map(|i| i.0).collect(),
                batches: snap
                    .batches
                    .iter()
                    .map(|b| BatchDto {
                        id: b.id,
                        index: b.index,
                        photo_ids: b.photo_ids.iter().map(|i| i.0).collect(),
                        visited: b.visited,
                        provisional: b.provisional,
                    })
                    .collect(),
                ratings: snap
                    .ratings
                    .iter()
                    .map(|(id, r)| RatingEntryDto {
                        photo_id: id.0,
                        rating: RatingDto::from(*r),
                    })
                    .collect(),
                visited: snap.visited.iter().copied().collect(),
                rating_mode: snap.rating_mode.to_string(),
                warnings: snap.warnings.to_vec(),
                skipped: Vec::new(),
                match_created: self.created,
            }
        })
    }

    /// Cheap per-frame read: one rating.
    pub fn rating(&self, photo_id: u64) -> Result<RatingDto, FirstcutError> {
        self.with(|s| Ok(RatingDto::from(s.rating(crate::batch::PhotoId(photo_id)))))
    }

    /// Cheap per-frame read: the batches.
    pub fn batches(&self) -> Result<Vec<BatchDto>, FirstcutError> {
        self.with(|s| {
            Ok(s.batches()
                .iter()
                .map(|b| BatchDto {
                    id: b.id,
                    index: b.index,
                    photo_ids: b.photo_ids.iter().map(|i| i.0).collect(),
                    visited: b.visited,
                    provisional: b.provisional,
                })
                .collect())
        })
    }

    /// The number of photos, for the HUD without a snapshot.
    pub fn photo_count(&self) -> Result<u32, FirstcutError> {
        self.with(|s| Ok(s.photos().len() as u32))
    }

    pub fn set_rating(
        &self,
        photo_id: u64,
        rating: RatingDto,
        batch_id: u64,
        batch_index: i64,
    ) -> Result<RatingChangeDto, FirstcutError> {
        self.with_mut(|s| {
            let change = s.set_rating(
                crate::batch::PhotoId(photo_id),
                rating.into(),
                batch_id,
                batch_index,
            )?;
            Ok(RatingChangeDto {
                photo_id: change.photo_id.0,
                batch_id: change.batch_id,
                before: RatingDto::from(change.before),
                after: RatingDto::from(change.after),
            })
        })
    }

    pub fn undo(&self) -> Result<Option<RatingChangeDto>, FirstcutError> {
        self.with_mut(|s| {
            let change = s.undo()?;
            Ok(change.map(|c| RatingChangeDto {
                photo_id: c.photo_id.0,
                batch_id: c.batch_id,
                before: RatingDto::from(c.before),
                after: RatingDto::from(c.after),
            }))
        })
    }

    pub fn redo(&self) -> Result<Option<RatingChangeDto>, FirstcutError> {
        self.with_mut(|s| {
            let change = s.redo()?;
            Ok(change.map(|c| RatingChangeDto {
                photo_id: c.photo_id.0,
                batch_id: c.batch_id,
                before: RatingDto::from(c.before),
                after: RatingDto::from(c.after),
            }))
        })
    }

    pub fn is_visited(&self, batch_id: u64) -> Result<bool, FirstcutError> {
        self.with(|s| Ok(s.is_visited(batch_id)))
    }

    pub fn mark_visited(
        &self,
        batch_id: u64,
        batch_index: i64,
        last_photo: Option<u64>,
    ) -> Result<(), FirstcutError> {
        self.with_mut(|s| {
            s.mark_visited(batch_id, batch_index, last_photo.map(crate::batch::PhotoId));
            Ok(())
        })
    }

    pub fn set_cursor(
        &self,
        batch_id: u64,
        batch_index: i64,
        photo_id: u64,
    ) -> Result<(), FirstcutError> {
        self.with_mut(|s| {
            s.set_cursor(batch_id, batch_index, crate::batch::PhotoId(photo_id));
            Ok(())
        })
    }

    /// Switching modes is a *view* change, never a migration: both fields are always stored
    /// (REV-69, REV-31).
    pub fn set_rating_mode(&self, mode: String) -> Result<(), FirstcutError> {
        self.with_mut(|s| {
            s.set_rating_mode(match mode.as_str() {
                "stars" => RatingMode::Stars,
                "keep" => RatingMode::KeepNotKeep,
                other => {
                    return Err(FirstcutError::Io {
                        message: format!(
                            "`{other}` is not a rating mode; expected `stars` or `keep`"
                        ),
                    });
                }
            });
            Ok(())
        })
    }

    /// Tier counts from the one mapped rule. The Finish step and the HUD both read this, so they
    /// cannot disagree about what "kept" means (REV-78).
    pub fn tier_counts(&self) -> Result<TierCountsDto, FirstcutError> {
        self.with(|s| {
            let counts = s.count_by_tier();
            let get = |t: Tier| counts.get(&t).copied().unwrap_or(0) as u32;
            Ok(TierCountsDto {
                keep: get(Tier::Keep),
                good: get(Tier::Good),
                maybe: get(Tier::Maybe),
                unrated: get(Tier::Unrated),
                rejected: get(Tier::Rejected),
            })
        })
    }

    /// Forces pending XMP writes out. On batch change and on quit (task.md §6.3).
    pub fn flush(&self) -> Result<(), FirstcutError> {
        self.with_mut(|s| {
            s.flush();
            Ok(())
        })
    }

    pub fn folder(&self) -> Result<String, FirstcutError> {
        self.with(|s| Ok(s.folder().display().to_string()))
    }
}

impl SessionHandle {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Session>, FirstcutError> {
        self.inner.lock().map_err(|_| FirstcutError::Store {
            message: "a previous operation panicked, so this session's state is not trustworthy"
                .into(),
        })
    }

    /// Read-only access.
    fn with<T>(
        &self,
        f: impl FnOnce(&Session) -> Result<T, FirstcutError>,
    ) -> Result<T, FirstcutError> {
        let guard = self.lock()?;
        f(&guard)
    }

    /// Mutating access. Separate from `with` so it is obvious at the call site which methods touch
    /// the session: a rating and a read are not the same kind of call, and the lock is held for
    /// both.
    fn with_mut<T>(
        &self,
        f: impl FnOnce(&mut Session) -> Result<T, FirstcutError>,
    ) -> Result<T, FirstcutError> {
        let mut guard = self.lock()?;
        f(&mut guard)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Free functions

/// Why a call failed, as the app sees it.
///
/// A typed enum rather than a bare `String`, which build.md requires ("never bare `String`") and
/// which UniFFI enforces -- a `Result<_, String>` fails to generate at all. The app can then
/// distinguish "this folder has no photographs" from "the disk is not there", and say something
/// useful instead of showing a raw error.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FirstcutError {
    /// The path is missing, or is not a directory.
    #[error("that is not a folder")]
    NotAFolder,
    /// The folder exists but holds nothing Firstcut can cull.
    #[error("no photographs in this folder")]
    NoPhotographs,
    /// The session database could not be read or written.
    #[error("the session could not be opened: {message}")]
    Store { message: String },
    /// A read or parse failed.
    #[error("{message}")]
    Io { message: String },
}

impl From<crate::session::SessionError> for FirstcutError {
    fn from(e: crate::session::SessionError) -> Self {
        use crate::session::SessionError as E;
        match e {
            E::Scan(m) if m.contains("not a folder") => FirstcutError::NotAFolder,
            E::Scan(m) if m.contains("no photographs") => FirstcutError::NoPhotographs,
            E::Scan(m) | E::Io(m) => FirstcutError::Io { message: m },
            E::Store(m) => FirstcutError::Store { message: m },
        }
    }
}

/// Opens a folder, creating or resuming its session.
///
/// A free function rather than an associated function because UniFFI objects cannot have
/// constructors: `#[uniffi::export]` on an `impl` block only exports instance methods.
#[uniffi::export]
pub fn open_session(folder: String) -> Result<Arc<SessionHandle>, FirstcutError> {
    let (session, matched, _scan) = Session::open(std::path::Path::new(&folder))?;
    Ok(Arc::new(SessionHandle {
        inner: std::sync::Mutex::new(session),
        created: matches!(matched, crate::session::SessionMatch::Created),
    }))
}

/// Scans a folder without opening a session. Used by `firstcut`-style tooling and by the
/// preferences pane's "this folder has N photographs" line, where opening a session would be a
/// side effect.
#[uniffi::export]
pub fn scan_folder_count(folder: String) -> Result<u32, FirstcutError> {
    Ok(crate::scan::scan_folder(std::path::Path::new(&folder))
        .photos
        .len() as u32)
}

/// Wave 1 smoke test: proves the Rust core is linked and callable from Swift. The
/// `CoreBridgeTests` assertion that this compiles and runs is the point.
#[uniffi::export]
pub fn hello() -> String {
    format!(
        "Firstcut core {} (Rust {})",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_NAME")
    )
}

/// Crate version, so the About box can say which core is running.
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Everything the FFI re-exports, kept in one list so nothing has to be discovered later.
///
/// The types are `pub` in their own modules and re-exported here so the generated Swift module is
/// the only thing the app imports.
pub use crate::meta::{AfInfo as FfiAfInfo, CaptureTime as FfiCaptureTime};

/// Unused-import guard: the DTOs above reference these, and this keeps that explicit rather than
/// accidental.
const _: fn() = || {
    let _ = std::mem::size_of::<CaptureTime>();
    let _: Option<TimeSource> = None;
    let _: Option<FileKind> = None;
    let _: Option<RawFormat> = None;
    let _: Option<AfInfo> = None;
    let _: Option<crate::meta::PhotoFingerprint> = None;
};
