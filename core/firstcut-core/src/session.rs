//! Owner: core-store. The `Session` object (docs/contracts/session-api.md).
//!
//! This is the object the app opens. It owns one folder and everything about it: the scanned
//! photos, their order, the batches, the ratings, the undo log and the XMP queue. Nothing else in
//! the system holds state that outlives a keystroke, so `Session` is the single place a rating can
//! be written and the single place it can be undone.
//!
//! ## Why it is one object
//!
//! Three earlier designs all lost a user's work in different ways, which is why this shape is what
//! it is:
//!
//! * **Ratings keyed by path** (REV-15/REV-68) lost the rating of any renamed file. Keyed by
//!   [`crate::meta::PhotoFingerprint`] instead, so a rename keeps it.
//! * **Undo keyed by batch** (REV-36) broke after a re-batch. Undo here applies **by photo**, and
//!   the batch is a hint for navigation only.
//! * **A finish step that decided "kept" from the raw field** (REV-78) trashed photos the UI had
//!   promised to keep. Every decision below goes through [`crate::store::rating::Rating::is_kept`],
//!   which is the one function that answers it.
//!
//! ## Threading
//!
//! `Session` is `Send` but not `Sync`: it is owned by whichever UniFFI object holds it and every
//! method takes `&mut self` or locks the database. The listener is a trait object so the same
//! session works under test without UniFFI in the way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::batch::{BatchParams, PhotoId, batch_with};
use crate::meta::{PhotoFingerprint, PhotoMeta, ScanResult, TimeSource};
use crate::order;
use crate::scan::scan_folder;
use crate::store::db::Db;
use crate::store::rating::{Rating, RatingMode, Tier};
use crate::store::records;

/// A batch, as the app sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBatch {
    pub id: u64,
    pub index: u32,
    pub photo_ids: Vec<PhotoId>,
    pub visited: bool,
    /// True until visual signatures have settled this batch's boundaries.
    pub provisional: bool,
}

/// A rating change, with both sides, so the app can put one back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatingChange {
    pub photo_id: PhotoId,
    /// Where the photo was when the change was made. A hint for navigation (task.md §6.3), never
    /// the key undo uses.
    pub batch_id: u64,
    pub before: Rating,
    pub after: Rating,
}

/// What `open` found. Reported rather than guessed at, because "matched an existing session" and
/// "created a new one" are different things to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionMatch {
    /// A new session was created for this folder.
    Created,
    /// An existing session was found and resumed.
    Resumed,
}

#[derive(Debug)]
pub enum SessionError {
    Io(String),
    Store(String),
    /// The folder could not be read at all. The message says why, in words a user can act on.
    Scan(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Io(m) => write!(f, "{m}"),
            SessionError::Store(m) => write!(f, "{m}"),
            SessionError::Scan(m) => write!(f, "could not read this folder: {m}"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<crate::store::error::StoreError> for SessionError {
    fn from(e: crate::store::error::StoreError) -> Self {
        SessionError::Store(e.to_string())
    }
}

/// One open folder.
pub struct Session {
    folder: PathBuf,
    db: Db,
    photos: Vec<PhotoMeta>,
    /// Capture order, as `PhotoId`s. The single ordering every other layer agrees with.
    order: Vec<PhotoId>,
    batches: Vec<SessionBatch>,
    ratings: HashMap<PhotoId, Rating>,
    visited: std::collections::HashSet<u64>,
    /// Signatures arrived so far, keyed by photo. Never iterated: `HashMap` order is randomised
    /// per process and determinism is what resume and the F1 numbers depend on (REV-24).
    sigs: HashMap<PhotoId, crate::batch::VisualSig>,
    /// Batches the user has already been through. Frozen: re-batching must not move them.
    frozen: Vec<crate::batch::Batch>,
    rating_mode: RatingMode,
    warnings: Vec<String>,
    /// Whether this session was created rather than resumed, recorded at open.
    created: bool,
}

impl std::fmt::Debug for Session {
    /// Enough to identify which session a panic was about, without dumping 1,500 photos.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("folder", &self.folder)
            .field("photos", &self.photos.len())
            .field("batches", &self.batches.len())
            .field("rated", &self.ratings.len())
            .field("rating_mode", &self.rating_mode)
            .finish()
    }
}

impl Session {
    /// Opens a folder, creating or resuming its session database.
    pub fn open(folder: &Path) -> Result<(Self, SessionMatch, ScanResult), SessionError> {
        if !folder.is_dir() {
            return Err(SessionError::Scan(format!(
                "{} is not a folder",
                folder.display()
            )));
        }
        let scan = scan_folder(folder);
        if scan.photos.is_empty() {
            return Err(SessionError::Scan(format!(
                "no photographs in {}",
                folder.display()
            )));
        }

        let db = Db::open(folder).map_err(|e| SessionError::Store(e.to_string()))?;
        let matched = db.matched().clone();

        let mut photos = scan.photos.clone();
        // A stable id from the durable fingerprint, so a rename keeps the same row (REV-68).
        for photo in &mut photos {
            photo.id = photo_id_for(photo);
        }
        let order = order::order(&photos);

        // Rows first, so a photo that was in the database but is not on disk any more is marked
        // rather than deleted: a temporarily missing drive must not lose ratings.
        let rows = to_photo_rows(&photos, &order);
        let present: Vec<u64> = photos.iter().map(|p| p.id.0).collect();
        records::upsert_photos(&db, &rows)?;
        for row in records::photos_not_in(&db, &present)? {
            records::set_photo_present(&db, row.id, false)?;
        }

        let mut session = Self {
            folder: folder.to_path_buf(),
            db,
            photos,
            ratings: HashMap::new(),
            visited: std::collections::HashSet::new(),
            sigs: HashMap::new(),
            frozen: Vec::new(),
            order,
            batches: Vec::new(),
            rating_mode: RatingMode::Stars,
            warnings: Vec::new(),
            created: false,
        };
        session.load_state()?;
        session.rebatch();

        let match_kind = match matched {
            crate::store::db::MatchKind::Created => SessionMatch::Created,
            _ => SessionMatch::Resumed,
        };
        session.created = matches!(match_kind, SessionMatch::Created);
        Ok((session, match_kind, scan))
    }

    /// Everything the app needs to render, read once per open.
    pub fn snapshot(&self) -> SessionSnapshot<'_> {
        SessionSnapshot {
            folder: self.folder.to_string_lossy().into_owned(),
            photos: &self.photos,
            order: &self.order,
            batches: &self.batches,
            ratings: &self.ratings,
            visited: &self.visited,
            rating_mode: self.rating_mode,
            warnings: &self.warnings,
        }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// True when this session was created rather than resumed.
    pub fn was_created(&self) -> bool {
        self.created
    }

    pub fn photos(&self) -> &[PhotoMeta] {
        &self.photos
    }

    pub fn batches(&self) -> &[SessionBatch] {
        &self.batches
    }

    pub fn rating(&self, id: PhotoId) -> Rating {
        self.ratings.get(&id).copied().unwrap_or_default()
    }

    pub fn ratings(&self) -> &HashMap<PhotoId, Rating> {
        &self.ratings
    }

    pub fn rating_mode(&self) -> RatingMode {
        self.rating_mode
    }

    /// Switching modes never migrates: both fields are always stored, and the mapping is a view
    /// (REV-69). This only records which view the user is looking at.
    pub fn set_rating_mode(&mut self, mode: RatingMode) {
        self.rating_mode = mode;
    }

    pub fn is_visited(&self, batch: u64) -> bool {
        self.visited.contains(&batch)
    }

    /// Writes a rating: DB now, XMP debounced (task.md §6). Both sides come back so the app can
    /// undo, and the change is recorded in the same transaction as the rating so a crash cannot
    /// leave a rating with no history entry.
    pub fn set_rating(
        &mut self,
        id: PhotoId,
        rating: Rating,
        batch: u64,
        batch_index: i64,
    ) -> Result<RatingChange, SessionError> {
        let before = self.rating(id);
        records::write_rating(
            &self.db,
            &records::RatingWrite {
                photo_id: id.0,
                rating,
                xmp_rating: Some(xmp_rating_value(&rating)),
                xmp_label: rating.label.map(|l| l.as_str().to_string()),
            },
        )?;
        records::push_history(
            &self.db,
            &records::HistoryEntry {
                seq: 0,
                photo_id: id.0,
                batch_id: batch,
                batch_index,
                before,
                after: rating,
                at_ms: now_ms(),
                undone: false,
            },
        )?;
        self.ratings.insert(id, rating);
        self.flush_xmp();
        Ok(RatingChange {
            photo_id: id,
            batch_id: batch,
            before,
            after: rating,
        })
    }

    /// Undo the most recent change. Applied **by photo**, so it still works after a re-batch has
    /// moved the photo into a different batch (REV-36).
    pub fn undo(&mut self) -> Result<Option<RatingChange>, SessionError> {
        let Some(entry) = self.last_undoable() else {
            return Ok(None);
        };
        records::write_rating(
            &self.db,
            &records::RatingWrite {
                photo_id: entry.photo_id,
                rating: entry.before,
                xmp_rating: Some(xmp_rating_value(&entry.before)),
                xmp_label: entry.before.label.map(|l| l.as_str().to_string()),
            },
        )?;
        records::set_history_undone(&self.db, entry.seq, true)?;
        self.ratings.insert(PhotoId(entry.photo_id), entry.before);
        self.flush_xmp();
        Ok(Some(RatingChange {
            photo_id: PhotoId(entry.photo_id),
            batch_id: entry.batch_id,
            before: entry.after,
            after: entry.before,
        }))
    }

    pub fn redo(&mut self) -> Result<Option<RatingChange>, SessionError> {
        let Some(entry) = records::last_redoable(&self.db)? else {
            return Ok(None);
        };
        records::write_rating(
            &self.db,
            &records::RatingWrite {
                photo_id: entry.photo_id,
                rating: entry.after,
                xmp_rating: Some(xmp_rating_value(&entry.after)),
                xmp_label: entry.after.label.map(|l| l.as_str().to_string()),
            },
        )?;
        records::set_history_undone(&self.db, entry.seq, false)?;
        self.ratings.insert(PhotoId(entry.photo_id), entry.after);
        self.flush_xmp();
        Ok(Some(RatingChange {
            photo_id: PhotoId(entry.photo_id),
            batch_id: entry.batch_id,
            before: entry.before,
            after: entry.after,
        }))
    }

    fn last_undoable(&self) -> Option<records::HistoryEntry> {
        records::last_undoable(&self.db).ok().flatten()
    }

    /// The cursor is a hint for resume only. It is never the key anything is stored by, which is
    /// what makes a re-batch harmless.
    pub fn set_cursor(&mut self, batch: u64, batch_index: i64, photo: PhotoId) {
        let _ = records::set_cursor(
            &self.db,
            &records::CursorRow {
                batch_id: Some(batch),
                batch_index: Some(batch_index),
                photo_id: Some(photo.0),
                view: None,
                updated_at_ms: now_ms(),
            },
        );
    }

    pub fn mark_visited(&mut self, batch: u64, batch_index: i64, last_photo: Option<PhotoId>) {
        self.visited.insert(batch);
        let _ = records::mark_visited(&self.db, batch, batch_index, last_photo.map(|p| p.0));
    }

    /// Visual signatures for some photos. Unvisited batches may be re-batched; visited ones may
    /// not, because the user has already worked through them and moving them would be hostile
    /// (task.md §5.4).
    pub fn submit_visual_sigs(&mut self, sigs: &[(PhotoId, crate::batch::VisualSig)]) -> usize {
        for (id, sig) in sigs {
            self.sigs.insert(*id, *sig);
        }
        let before = self.batches.len();
        self.rebatch();
        before - self.batches.len()
    }

    /// Tier counts, from the one mapped rule (REV-78), for the HUD and the Finish summary.
    pub fn count_by_tier(&self) -> HashMap<Tier, usize> {
        let mut counts: HashMap<Tier, usize> = Tier::ALL.into_iter().map(|t| (t, 0)).collect();
        for rating in self.ratings.values() {
            *counts.entry(rating.tier(self.rating_mode)).or_insert(0) += 1;
        }
        counts
    }

    /// Forces the pending XMP writes out. Called on batch change and on quit (task.md §6.3).
    pub fn flush(&mut self) {
        self.flush_xmp();
    }

    // ───────────────────────────────────────────────────────────────────────── internals

    fn load_state(&mut self) -> Result<(), SessionError> {
        // The store keys by raw u64; the rest of the core keys by PhotoId. Convert once, here,
        // so no caller ever has to remember which is which.
        self.ratings = records::ratings(&self.db)?
            .into_iter()
            .map(|(id, rating)| (PhotoId(id), rating))
            .collect();
        self.visited = records::visited_batch_ids(&self.db)?;
        Ok(())
    }

    /// Recomputes the batches, freezing the ones the user has visited.
    fn rebatch(&mut self) {
        let outcome = batch_with(
            &self.photos,
            &self.sigs,
            &self.frozen,
            BatchParams::default(),
        );
        self.batches = outcome
            .batches
            .iter()
            .map(|b| SessionBatch {
                id: b.id.0,
                index: b.index,
                photo_ids: b.photo_ids.clone(),
                visited: self.visited.contains(&b.id.0),
                provisional: b.provisional,
            })
            .collect();
        // A batch the user has been through becomes frozen for the rest of the session.
        for b in &self.batches {
            if b.visited
                && !self.frozen.iter().any(|f| f.id.0 == b.id)
                && let Some(full) =
                    outcome
                        .batches
                        .iter()
                        .find(|f| f.id.0 == b.id)
                        .map(|f| crate::batch::Batch {
                            provisional: false,
                            ..f.clone()
                        })
            {
                self.frozen.push(full);
            }
        }
    }

    /// Writes the DB rows for a rating to their sidecars, debounced.
    ///
    /// The DB is the source of truth and is already written; this is the interoperability copy
    /// (task.md §6, "XMP sidecars **and** app DB"). A failure here is reported, never fatal: the
    /// rating is safe in the DB and will be rewritten on the next change.
    fn flush_xmp(&mut self) {
        let Ok(pending) = records::pending_xmp_writes(&self.db) else {
            return;
        };
        for row in pending {
            let Some(photo) = self.photos.iter().find(|p| p.id.0 == row.photo_id) else {
                continue;
            };
            // **The sidecar path, never the RAW path.** `xmp::write_sidecar` writes to whatever
            // path it is given, so handing it the photograph would overwrite the original with XML
            // -- the single worst thing this program can do. `sidecar_path` appends `.xmp`.
            let path = self.folder.join(crate::xmp::sidecar_path(&photo.rel_path));
            // The mapping lives in `XmpMapping`, not here: it already knows a reject is -1 and a
            // keep is 5 stars, and a second copy of that rule is how a sidecar and the UI drift.
            let result = crate::xmp::write_rating(
                &path,
                row.rating,
                self.rating_mode,
                &crate::xmp::XmpMapping::default(),
            );
            if result.is_ok() {
                let _ = records::mark_xmp_written(&self.db, row.photo_id);
            } else if let Err(e) = result {
                self.warnings.push(format!("{}: {e}", photo.rel_path));
            }
        }
    }
}

/// Everything the app reads once per open. Borrows rather than clones: 1,500 `PhotoMeta` across
/// FFI is megabytes, and it is read once (REV-37).
pub struct SessionSnapshot<'a> {
    pub folder: String,
    pub photos: &'a [PhotoMeta],
    pub order: &'a [PhotoId],
    pub batches: &'a [SessionBatch],
    pub ratings: &'a HashMap<PhotoId, Rating>,
    pub visited: &'a std::collections::HashSet<u64>,
    pub rating_mode: RatingMode,
    pub warnings: &'a [String],
}

/// A photo's durable id: a hash of its fingerprint, so a rename keeps the same row.
///
/// The path is the documented fallback when a camera omitted the fields that make a fingerprint
/// possible (REV-15): an incomplete fingerprint hashed into an id would collide with another
/// photo's, which is worse than admitting the id is path-based.
pub fn photo_id_for(photo: &PhotoMeta) -> PhotoId {
    match photo.fingerprint() {
        Some(PhotoFingerprint {
            camera_serial,
            capture_unix_ms,
            shutter_count,
            file_size,
        }) => {
            let material = format!("{camera_serial}|{capture_unix_ms}|{shutter_count}|{file_size}");
            crate::batch::photo_id(&material)
        }
        None => crate::batch::photo_id(&photo.rel_path),
    }
}

fn to_photo_rows(photos: &[PhotoMeta], order: &[PhotoId]) -> Vec<records::PhotoRow> {
    let ordinal = |id: PhotoId| {
        order
            .iter()
            .position(|o| *o == id)
            .map_or(-1i64, |i| i as i64)
    };
    photos
        .iter()
        .map(|p| records::PhotoRow {
            id: p.id.0,
            rel_path: p.rel_path.clone(),
            group_key: p
                .rel_path
                .rsplit_once('.')
                .map_or(p.rel_path.clone(), |(s, _)| s.to_string()),
            companions: p.companions.clone(),
            file_size: p.file_size,
            mtime_ms: p
                .capture_time
                .and_then(|c| (c.source == TimeSource::FileModified).then_some(c.unix_ms)),
            device: None,
            ino: None,
            meta_json: None,
            ordinal: Some(ordinal(p.id)),
            first_seen_at_ms: now_ms(),
            last_seen_at_ms: now_ms(),
            present: true,
        })
        .collect()
}

/// `xmp:Rating`, which is -1 for a reject and 0..5 otherwise (task.md §6.3).
fn xmp_rating_value(rating: &Rating) -> i64 {
    use crate::store::rating::Flag;
    if rating.flag == Flag::Reject {
        -1
    } else {
        i64::from(rating.stars)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
