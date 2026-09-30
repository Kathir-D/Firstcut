//! Owner: core-store. The `Session` object (docs/contracts/session-api.md).
//!
//! One object that ties the other three together: core-meta scans, core-batch orders and batches,
//! core-store persists, and `crate::xmp` mirrors every rating into a sidecar. It owns the session
//! database and is the only thing that writes SQL through it.
//!
//! The three rules that shape the design:
//!
//! * **A rating keystroke returns in under a millisecond** (docs/contracts/session-api.md). The
//!   rating goes into the WAL in one synchronous transaction; the sidecar is handed to
//!   [`XmpWriter`], which debounces it onto a background thread. Nothing on the caller's thread
//!   touches a photo file.
//! * **Originals are never modified.** No code path in this file opens a photo for writing. A
//!   rating can only reach a sidecar, which is a separate file Firstcut creates.
//! * **A renamed file keeps its rating** (REV-68). `PhotoId` is a hash of the path, so a rename
//!   would lose the rating; [`Session::reconcile`] reattaches rows by content fingerprint when it
//!   finds a row whose path is gone and a new path with the same bytes.
//!
//! The session is shared across threads — the UI calls it while the XMP writer runs — so every
//! mutation goes through the same `Mutex`. The store's `Db` has its own connection lock inside,
//! and `crate::store::records` keeps each of its statements to one lock acquisition so the two
//! never deadlock.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::batch::{Batch, BatchId, PhotoId, VisualSig, batch_with};
use crate::meta::{PhotoMeta, ScanResult, scan_folder};
use crate::store::rating::{Rating, RatingMode, Tier};
use crate::store::records::{BatchRow, HistoryEntry, PhotoRow, RatingRow, RatingWrite, VisitedRow};
use crate::store::{Db, MatchKind, StoreError, records};
use crate::xmp::writer::{ErrorSink, PendingWrite, XmpWriter};
use crate::xmp::{XmpMapping, read_sidecar, sidecar_path};

pub type Result<T> = std::result::Result<T, SessionError>;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("{0}")]
    Store(#[from] StoreError),

    #[error("no such folder: {0}")]
    FolderNotFound(PathBuf),

    #[error("not a folder: {0}")]
    NotAFolder(PathBuf),

    #[error("cannot scan {path}: {reason}")]
    Scan { path: PathBuf, reason: String },

    /// Undo Finish was asked for a run that cannot be reversed.
    #[error("{0}")]
    CannotUndo(String),

    /// The session database was written by a newer Firstcut. Refused rather than downgraded.
    #[error("session database is schema version {found}, this build only understands up to {max}")]
    NewerSchema { found: i64, max: i64 },
}

impl From<ScanErrorLike> for SessionError {
    fn from(err: ScanErrorLike) -> SessionError {
        match err {
            ScanErrorLike::FolderNotFound(path) => SessionError::FolderNotFound(path),
            ScanErrorLike::NotAFolder(path) => SessionError::NotAFolder(path),
            ScanErrorLike::Io { path, reason } => SessionError::Scan { path, reason },
        }
    }
}

/// The scanner's error, flattened so `Session` does not have to name the scan module's type in
/// its own error enum, which would leak it into the FFI surface.
#[derive(Debug)]
pub enum ScanErrorLike {
    FolderNotFound(PathBuf),
    NotAFolder(PathBuf),
    Io { path: PathBuf, reason: String },
}

impl From<crate::meta::ScanError> for ScanErrorLike {
    fn from(err: crate::meta::ScanError) -> Self {
        use crate::meta::ScanError;
        match err {
            ScanError::FolderNotFound(path) => ScanErrorLike::FolderNotFound(path),
            ScanError::NotAFolder(path) => ScanErrorLike::NotAFolder(path),
            ScanError::Io { path, source } => ScanErrorLike::Io {
                path,
                reason: source.to_string(),
            },
        }
    }
}

/// Where the user is: which batch, which photo inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub batch: BatchId,
    pub photo: PhotoId,
}

/// One rating change, with everything undo needs to put it back (task.md §6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    /// Monotonic across the session, so the app can order changes it has already seen.
    pub id: u64,
    pub photo: PhotoId,
    pub batch: BatchId,
    /// The batch position the change was made in, so `undo` can navigate back across batches.
    pub batch_index: u32,
    pub before: Rating,
    pub after: Rating,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionSnapshot {
    pub folder: String,
    /// In capture order, which is `ordinal`, not `rel_path`.
    pub photos: Vec<PhotoMeta>,
    pub batches: Vec<Batch>,
    pub ratings: HashMap<u64, Rating>,
    pub visited: HashSet<u64>,
    pub cursor: Option<Cursor>,
    pub last_photo_in_batch: HashMap<u64, u64>,
    /// Files found but not understood, with the reason (task.md §8).
    pub skipped: Vec<crate::meta::Skipped>,
}

/// What the session tells the app about (docs/contracts/session-api.md).
pub trait SessionListener: Send + Sync {
    /// Only unvisited batches can differ: a batch the user has been in is frozen.
    fn batches_changed(&self, batches: Vec<Batch>);
    /// FSEvents: files were added or removed.
    fn files_changed(&self);
    fn xmp_error(&self, photo: PhotoId, message: String);
    /// The folder moved since the last time this shoot was opened.
    fn session_moved(&self, from: String) {
        let _ = from;
    }
}

/// A listener that does nothing, for tests and for a session opened without one.
pub struct NoListener;

impl SessionListener for NoListener {
    fn batches_changed(&self, _batches: Vec<Batch>) {}
    fn files_changed(&self) {}
    fn xmp_error(&self, _photo: PhotoId, _message: String) {}
}

/// Whether and how ratings are mirrored to `.xmp` sidecars (Settings → Metadata, task.md §9.8).
///
/// Sidecars are the only thing Firstcut ever writes beside a photo, and never into the photo
/// itself, so these switches decide *whether* a sidecar appears, never whether an original changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XmpSettings {
    /// Off: ratings live only in the session database.
    pub write_sidecars: bool,
    /// JPEG/HEIF/PNG/TIFF photos get a sidecar too. Off leaves them without one.
    pub sidecars_for_non_raw: bool,
}

impl Default for XmpSettings {
    fn default() -> XmpSettings {
        XmpSettings {
            write_sidecars: true,
            sidecars_for_non_raw: true,
        }
    }
}

/// The state behind the `Mutex`. Split out so the lock is held for one operation at a time and the
/// public methods stay readable.
struct State {
    db: Db,
    /// The scan, by id, so `snapshot` and the batcher can look a photo up without re-reading.
    photos: HashMap<u64, PhotoMeta>,
    /// Capture order, as ids. The `photos` map is unordered, so this is what "in order" means.
    order: Vec<PhotoId>,
    /// Batch rows in index order, kept in step with the database.
    batches: Vec<Batch>,
    /// Signatures submitted by the pipeline so far. Never persisted: they are re-derived from
    /// the thumbnails on the next open.
    sigs: HashMap<PhotoId, VisualSig>,
    skipped: Vec<crate::meta::Skipped>,
    /// One `Change` per rating call, handed out in order so the app can detect a stale one.
    change_seq: u64,
    rating_mode: RatingMode,
    mapping: XmpMapping,
    xmp: XmpSettings,
}

pub struct Session {
    folder: PathBuf,
    state: Mutex<State>,
    listener: Arc<dyn SessionListener>,
    writer: Arc<XmpWriter>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("folder", &self.folder)
            .field("photos", &self.state().photos.len())
            .field("batches", &self.state().batches.len())
            .finish()
    }
}

/// Forwards writer failures to the listener, which is where `SessionListener::xmp_error` lives.
struct ListenerSink(Arc<dyn SessionListener>);

impl ErrorSink for ListenerSink {
    fn xmp_error(&self, photo_id: u64, message: String) {
        self.0.xmp_error(PhotoId(photo_id), message);
    }
}

impl Session {
    /// Opens a shoot: scan, order, provisional batches, restore the database.
    ///
    /// `sessions_dir` is where the database lives; the app passes
    /// `store::sessions_dir()`. Tests pass a temporary directory so they never touch the user's
    /// real sessions.
    pub fn open_in(
        folder: &Path,
        sessions_dir: &Path,
        listener: Arc<dyn SessionListener>,
    ) -> Result<Session> {
        let scan = scan_folder(folder).map_err(ScanErrorLike::from)?;
        Session::from_scan(scan, folder, sessions_dir, listener)
    }

    /// [`Session::open_in`] against the user's real sessions directory.
    pub fn open(folder: &Path, listener: Arc<dyn SessionListener>) -> Result<Session> {
        let sessions = crate::store::sessions_dir()?;
        Session::open_in(folder, &sessions, listener)
    }

    fn from_scan(
        scan: ScanResult,
        folder: &Path,
        sessions_dir: &Path,
        listener: Arc<dyn SessionListener>,
    ) -> Result<Session> {
        let db = Db::open_in(sessions_dir, folder)?;
        if let MatchKind::Moved { from } = db.matched() {
            listener.session_moved(from.display().to_string());
        }

        let writer = Arc::new(XmpWriter::new(Arc::new(ListenerSink(Arc::clone(
            &listener,
        )))));
        // A shoot rated in Lightroom and opened here for the first time has its ratings only in
        // sidecars, and starting from a blank database would silently discard them (task.md §11).
        let created = matches!(db.matched(), MatchKind::Created);
        let mut state = State {
            rating_mode: db.rating_mode()?,
            db,
            photos: HashMap::new(),
            order: Vec::new(),
            batches: Vec::new(),
            sigs: HashMap::new(),
            skipped: scan.skipped,
            change_seq: 0,
            mapping: XmpMapping::default(),
            xmp: XmpSettings::default(),
        };
        state.ingest(scan.photos)?;
        if created {
            let photos: Vec<PhotoMeta> = state.photos.values().cloned().collect();
            for (id, rating) in import_ratings_from_sidecars(folder, &photos) {
                records::write_rating(
                    &state.db,
                    &RatingWrite {
                        photo_id: id.0,
                        rating,
                        xmp_rating: None,
                        xmp_label: None,
                    },
                )?;
                // The sidecar already says this. Leaving the row pending would queue a write of
                // "no values", which reads as "remove the rating" and would wipe what was imported.
                records::mark_xmp_written(&state.db, id.0)?;
            }
        }

        // The session is shared across threads — the UI calls it while the XMP writer runs — so
        // every mutation goes through this one lock. A public method that needs the lock twice
        // must take it once and work from the guard, or it deadlocks against itself.
        let session = Session {
            folder: std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf()),
            state: Mutex::new(state),
            listener,
            writer,
        };
        // Provisional batches from metadata alone, restored against anything the previous session
        // already froze. Without this the app would open a shoot with no batches at all.
        {
            let mut state = session.state();
            state.batches = state.rebatch().unwrap_or_default();
        }
        // A sidecar write the last session was interrupted between is re-queued here, so at most
        // one debounce window of work is repeated and never lost.
        session.requeue_pending_writes()?;
        Ok(session)
    }

    /// The canonical shoot folder.
    #[must_use]
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// New, the same place, or re-matched after a move. Swift shows a note when it is `Moved`.
    #[must_use]
    pub fn matched(&self) -> MatchKind {
        self.state().db.matched().clone()
    }

    #[must_use]
    pub fn rating_mode(&self) -> RatingMode {
        self.state().rating_mode
    }

    /// Switches modes. Existing data is preserved and mapped, never rewritten
    /// (task.md §6): the stored rating keeps both the stars and the keep flag, and only the display
    /// changes. See `store::rating::display_rating`.
    pub fn set_rating_mode(&self, mode: RatingMode) -> Result<()> {
        {
            let mut state = self.state();
            state.db.set_rating_mode(mode)?;
            state.rating_mode = mode;
        }
        // Ratings written under the old mode may have sidecars with the old spelling, so they are
        // re-queued rather than left showing a stale value in Lightroom. This has to happen after
        // the lock is released, because re-queueing takes it again.
        self.requeue_pending_writes()?;
        Ok(())
    }

    /// How ratings are spelled in XMP (docs/contracts/session-api.md `XmpMapping`).
    pub fn set_xmp_mapping(&self, mapping: XmpMapping) {
        let changed = {
            let mut state = self.state();
            let changed = state.mapping != mapping;
            state.mapping = mapping;
            changed
        };
        // A different spelling of a Keep changes what the existing sidecars should say.
        if changed {
            let _ = self.write_all_sidecars();
        }
    }

    /// Whether sidecars are written at all, and for which kinds of file. Ratings already in the
    /// database are re-queued, so turning sidecars on writes the ones that were skipped.
    pub fn set_xmp_settings(&self, settings: XmpSettings) -> Result<()> {
        let changed = {
            let mut state = self.state();
            let changed = state.xmp != settings;
            state.xmp = settings;
            changed
        };
        if changed {
            self.write_all_sidecars()?;
        }
        Ok(())
    }

    /// Queues a sidecar for every photo that has a rating. Idempotent (a write merges), which is
    /// what makes it safe to run whenever a setting that could add sidecars changes: the pending
    /// flag is cleared when a write is *queued*, so a skipped one has to be rebuilt from the
    /// ratings themselves rather than from the queue.
    fn write_all_sidecars(&self) -> Result<()> {
        let (ratings, mode, mapping) = {
            let state = self.state();
            (
                records::ratings(&state.db)?,
                state.rating_mode,
                state.mapping.clone(),
            )
        };
        for (id, rating) in ratings {
            if rating.is_neutral() {
                continue;
            }
            self.queue_sidecar(PhotoId(id), &mapping.values_for(rating, mode));
        }
        Ok(())
    }

    /// Everything the UI draws from, in one value.
    pub fn snapshot(&self) -> SessionSnapshot {
        let state = self.state();
        let by_id: HashMap<u64, &PhotoMeta> =
            state.photos.iter().map(|(id, meta)| (*id, meta)).collect();
        let photos: Vec<PhotoMeta> = state
            .order
            .iter()
            .filter_map(|id| by_id.get(&id.0).map(|meta| (*meta).clone()))
            .collect();
        SessionSnapshot {
            folder: self.folder.display().to_string(),
            photos,
            batches: state.batches.clone(),
            ratings: records::ratings(&state.db).unwrap_or_default(),
            visited: records::visited_batch_ids(&state.db).unwrap_or_default(),
            cursor: read_cursor(&state.db).ok().flatten(),
            last_photo_in_batch: records::visited(&state.db)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|row: VisitedRow| row.last_photo_id.map(|id| (row.batch_id, id)))
                .collect(),
            skipped: state.skipped.clone(),
        }
    }

    /// The photos that could not be read, with the reason (task.md §8).
    #[must_use]
    pub fn skipped(&self) -> Vec<crate::meta::Skipped> {
        self.state().skipped.clone()
    }

    /// Submits visual signatures from the pipeline, which may re-batch the unvisited batches.
    ///
    /// The pipeline sends them in chunks as thumbnails finish, so this is called repeatedly and
    /// only fires the listener when the boundaries actually moved. Visited batches are frozen
    /// (task.md §5.4): a batch the user is in is never re-cut under them.
    pub fn submit_visual_sigs(&self, sigs: Vec<(PhotoId, VisualSig)>) {
        let mut state = self.state();
        for (id, sig) in sigs {
            state.sigs.insert(id, sig);
        }
        let Some(batches) = state.rebatch() else {
            return;
        };
        if batches == state.batches {
            return;
        }
        state.batches = batches;
        drop(state);
        self.listener.batches_changed(self.state().batches.clone());
    }

    /// Records a rating. Returns the change, which carries what undo needs.
    ///
    /// The database write is synchronous and the sidecar write is debounced, so this returns in
    /// the time one WAL transaction takes. A photo that is not in this session is still rated: the
    /// contract says the "only rate in the current batch" rule is enforced by app-logic, not here.
    pub fn set_rating(&self, photo: PhotoId, rating: Rating) -> Result<Change> {
        self.set_rating_with(photo, rating, None, None)
    }

    /// The `PhotoMeta`-shaped form the FFI layer uses, so Swift can set stars, flag, label and
    /// keep in one call.
    pub fn set_rating_with(
        &self,
        photo: PhotoId,
        rating: Rating,
        flag: Option<crate::store::rating::Flag>,
        label: Option<crate::store::rating::ColorLabel>,
    ) -> Result<Change> {
        let mut state = self.state();
        let mut rating = rating;
        if let Some(flag) = flag {
            rating.flag = flag;
        }
        if label.is_some() {
            rating.label = label;
        }

        let before =
            records::rating_of(&state.db, photo.0)?.map_or_else(Rating::neutral, |row| row.rating);
        let (batch, batch_index) = state.batch_of(photo);

        // The XMP values are decided here and stored with the rating, so the pending row already
        // says exactly what the sidecar should end up as. That is what makes a crash between the
        // commit and the write recoverable: the value is in the database, not in memory.
        // The sidecar is the interoperability copy, so it gets the rating *as displayed* in the
        // current mode: a keep with no stars is 5 stars to Lightroom (task.md §6). What is stored
        // stays as the user gave it — the display is a projection and writing it back would
        // persist a mode the user is not in (`store::rating::display_rating`).
        let shown = crate::store::rating::display_rating(&rating, state.rating_mode);
        let values = state.mapping.values_for(shown, state.rating_mode);
        let row = records::write_rating(
            &state.db,
            &RatingWrite {
                photo_id: photo.0,
                rating,
                xmp_rating: values.rating,
                xmp_label: values.label.clone(),
            },
        )?;

        state.change_seq += 1;
        let change = Change {
            id: state.change_seq,
            photo,
            batch,
            batch_index,
            before,
            after: rating,
        };
        records::push_history(
            &state.db,
            &HistoryEntry {
                seq: 0,
                photo_id: photo.0,
                batch_id: batch.0,
                batch_index: i64::from(batch_index),
                before,
                after: rating,
                at_ms: crate::store::now_ms(),
                undone: false,
            },
        )?;
        drop(state);

        // A rating that clears everything removes the sidecar's values rather than leaving a
        // stale rating behind, so the queued write still happens.
        self.queue_sidecar(photo, &values);
        let _ = row;
        Ok(change)
    }

    /// Reverts the newest change, or `None` when there is nothing to undo.
    pub fn undo(&self) -> Option<Change> {
        self.apply_history(crate::store::records::last_undoable, true)
    }

    /// Re-applies the newest undone change, or `None` when there is nothing to redo.
    pub fn redo(&self) -> Option<Change> {
        self.apply_history(crate::store::records::last_redoable, false)
    }

    fn apply_history(
        &self,
        pick: fn(&Db) -> crate::store::Result<Option<HistoryEntry>>,
        undoing: bool,
    ) -> Option<Change> {
        let mut state = self.state();
        let entry = pick(&state.db).ok().flatten()?;
        // Undo puts the "before" value back; redo puts the "after" value back. The change reports
        // what the rating was when this call found it, which is the other end of the same pair.
        let after = if undoing { entry.before } else { entry.after };
        let current = records::rating_of(&state.db, entry.photo_id)
            .ok()
            .flatten()
            .map_or_else(Rating::neutral, |row| row.rating);
        let shown = crate::store::rating::display_rating(&after, state.rating_mode);
        let values = state.mapping.values_for(shown, state.rating_mode);
        records::write_rating(
            &state.db,
            &RatingWrite {
                photo_id: entry.photo_id,
                rating: after,
                xmp_rating: values.rating,
                xmp_label: values.label.clone(),
            },
        )
        .ok()?;
        records::set_history_undone(&state.db, entry.seq, undoing).ok()?;
        state.change_seq += 1;
        let change = Change {
            id: state.change_seq,
            photo: PhotoId(entry.photo_id),
            batch: BatchId(entry.batch_id),
            batch_index: entry.batch_index.max(0) as u32,
            before: current,
            after,
        };
        drop(state);
        // The sidecar follows the database, so an undo also undoes what Lightroom would show.
        self.queue_sidecar(PhotoId(entry.photo_id), &values);
        Some(change)
    }

    /// Records where the user is, so reopening the shoot lands in the same place.
    pub fn set_cursor(&self, cursor: Cursor) -> Result<()> {
        let state = self.state();
        let batch_index = state
            .batches
            .iter()
            .find(|b| b.id == cursor.batch)
            .map_or(0, |b| b.index);
        records::set_cursor(
            &state.db,
            &records::CursorRow {
                batch_id: Some(cursor.batch.0),
                photo_id: Some(cursor.photo.0),
                batch_index: Some(i64::from(batch_index)),
                view: None,
                updated_at_ms: crate::store::now_ms(),
            },
        )?;
        // Leaving a batch is what freezes it: the user has seen it and has rated inside it.
        records::mark_visited(
            &state.db,
            cursor.batch.0,
            i64::from(batch_index),
            Some(cursor.photo.0),
        )?;
        Ok(())
    }

    /// Marks a batch as seen, which also freezes it against re-batching (task.md §5.4).
    ///
    /// Deliberately records no position: "seen" is not "the user was looking at the end of it",
    /// and writing the batch's last photo here would send them back to a frame they never
    /// reached. `set_cursor` is what records where they are.
    pub fn mark_visited(&self, batch: BatchId) -> Result<()> {
        let state = self.state();
        let index = state
            .batches
            .iter()
            .find(|b| b.id == batch)
            .map_or(0, |b| b.index);
        records::mark_visited(&state.db, batch.0, i64::from(index), None)?;
        Ok(())
    }

    /// Flushes the sidecar queue and checkpoints the WAL. Called on quit and on batch change.
    pub fn flush(&self) {
        self.writer.flush();
        let _ = self.state().db.checkpoint();
    }

    /// Re-reads the folder and reconciles by identity (REV-68).
    ///
    /// A `PhotoId` is a hash of the path, so a renamed file comes back from the scan as a brand new
    /// photo. Left alone its rating would be orphaned on the row for the old name, and the user
    /// would see their cull disappear because they renamed a file in Finder. The fix is to
    /// recognise that the file is the same one: `st_dev` + `st_ino` survive a rename within a
    /// volume, and the camera's `ShutterCount` plus size survive it anywhere. Whichever matches
    /// carries the rating, the history and the visited position over to the new id.
    pub fn rescan(&self) -> Result<ScanResult> {
        let scan = scan_folder(&self.folder).map_err(ScanErrorLike::from)?;
        let mut state = self.state();
        state.skipped = scan.skipped.clone();
        let photos = scan.photos.clone();
        let moved = state.reconcile(photos)?;
        state.batches = state.rebatch().unwrap_or_else(|| state.batches.clone());
        drop(state);
        if !moved.is_empty() {
            // A rename moved photos, so the batches that referenced the old ids are stale.
            self.listener.batches_changed(self.state().batches.clone());
        }
        self.listener.files_changed();
        Ok(scan)
    }

    /// Stops the writer, writing anything still pending. Called when the app quits.
    pub fn close(&self) {
        self.flush();
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Queues the sidecar write for one photo, and points the debouncer at its sidecar path.
    ///
    /// The rating itself is already committed; this only schedules the mirror, so a failure here
    /// is reported to the listener and nothing more.
    fn queue_sidecar(&self, photo: PhotoId, values: &crate::xmp::document::XmpValues) {
        let rel = {
            let state = self.state();
            let wanted = state.xmp.write_sidecars
                && state.photos.get(&photo.0).is_some_and(|meta| {
                    state.xmp.sidecars_for_non_raw
                        || matches!(meta.kind, crate::meta::FileKind::Raw(_))
                });
            if wanted {
                state.photos.get(&photo.0).map(|meta| meta.rel_path.clone())
            } else {
                None
            }
        };
        let Some(rel) = rel else {
            // A photo the session has never seen has no path to write next to. The rating is safe
            // in the database, which is the source of truth.
            return;
        };
        self.writer.submit(PendingWrite {
            photo_id: photo.0,
            sidecar: self.folder.join(sidecar_path(&rel)),
            values: values.clone(),
        });
    }

    /// Re-queues every rating the database still marks as unwritten.
    ///
    /// This is the crash-recovery path: a write that was pending when the app died is still in the
    /// database, so at most one debounce window of work is repeated and nothing is lost.
    fn requeue_pending_writes(&self) -> Result<()> {
        let pending = {
            let state = self.state();
            records::pending_xmp_writes(&state.db)?
        };
        for row in pending {
            self.queue_pending_row(row);
        }
        Ok(())
    }

    /// Rebuilds the queued write from a stored row, clearing the pending flag once it is queued.
    ///
    /// The row is marked written *before* the write happens, because the queue is in memory and
    /// the database is the thing that survives a crash. If the app dies between the two, the next
    /// open finds nothing pending and the sidecar is one rating behind — which the app repairs
    /// from the database when it next changes that photo. The reverse order would risk losing a
    /// rating entirely.
    fn queue_pending_row(&self, row: RatingRow) {
        let rating_mode = self.state().rating_mode;
        let mapping = self.state().mapping.clone();
        let values = crate::xmp::document::XmpValues {
            rating: row.xmp_rating,
            label: row.xmp_label.clone(),
        };
        // A row with no stored values means "remove the rating"; the mapping still decides what
        // that is for the current mode.
        let values = if row.xmp_rating.is_none() && row.xmp_label.is_none() {
            let stored = row.rating;
            if rating_mode == RatingMode::Stars || stored.is_neutral() {
                values
            } else {
                mapping.values_for(stored, rating_mode)
            }
        } else {
            values
        };
        self.queue_sidecar(PhotoId(row.photo_id), &values);
        let _ = records::mark_xmp_written(&self.state().db, row.photo_id);
    }
}

impl State {
    /// Folds a scan into the database, the id map and the ordinals.
    fn ingest(&mut self, photos: Vec<PhotoMeta>) -> crate::store::Result<()> {
        let now = crate::store::now_ms();
        let rows: Vec<PhotoRow> = photos.iter().map(|meta| photo_row(meta, now)).collect();
        records::upsert_photos(&self.db, &rows)?;
        self.absorb_missing(&photos);
        self.remember(photos);
        self.assign_ordinals();
        Ok(())
    }

    /// Marks the rows the scan did not see as absent, so a rating outlives a file that is
    /// temporarily off the card.
    fn absorb_missing(&mut self, photos: &[PhotoMeta]) {
        let present: Vec<u64> = photos.iter().map(|meta| meta.id.0).collect();
        let Ok(missing) = records::photos_not_in(&self.db, &present) else {
            return;
        };
        for row in missing {
            let _ = records::set_photo_present(&self.db, row.id, false);
        }
    }

    fn remember(&mut self, photos: Vec<PhotoMeta>) {
        for meta in &photos {
            self.photos.insert(meta.id.0, meta.clone());
        }
        self.order = crate::order::order(&photos);
    }

    fn assign_ordinals(&mut self) {
        let ordinals: Vec<(u64, i64)> = self
            .order
            .iter()
            .enumerate()
            .map(|(position, id)| (id.0, position as i64))
            .collect();
        let _ = records::set_photo_ordinals(&self.db, &ordinals);
    }

    /// Recognises renamed files and carries their state to the new id (REV-68).
    ///
    /// A file is "the same one" when it is the same inode on the same volume, which is what a
    /// rename in Finder produces. The shutter count and file size are the fallback for a file that
    /// came back from a card, because both are properties of the capture rather than of the name.
    /// Anything matched moves its rating, its history and its visited position; anything
    /// unmatched is simply a new photo, which is the common case and costs nothing.
    fn reconcile(&mut self, photos: Vec<PhotoMeta>) -> crate::store::Result<Vec<(u64, u64)>> {
        let present: Vec<u64> = photos.iter().map(|meta| meta.id.0).collect();
        // Only the rows the scan did not see are candidates for a rename. A row the scan *did*
        // see is the same file under the same name, whatever the previous scan thought.
        let vanished: Vec<PhotoRow> = records::photos_not_in(&self.db, &present)?;
        if vanished.is_empty() {
            self.ingest(photos)?;
            return Ok(Vec::new());
        }

        // Index the vanished rows by the two identities that survive a rename.
        let mut by_inode: HashMap<(i64, i64), Vec<PhotoRow>> = HashMap::new();
        let mut by_capture: HashMap<(Option<u64>, u64), Vec<PhotoRow>> = HashMap::new();
        for row in vanished {
            if let (Some(device), Some(ino)) = (row.device, row.ino) {
                by_inode.entry((device, ino)).or_default().push(row.clone());
            }
            let shutter = self.photos.get(&row.id).and_then(|meta| meta.shutter_count);
            by_capture
                .entry((shutter, row.file_size))
                .or_default()
                .push(row);
        }

        let mut moved = Vec::new();
        let mut renames: Vec<(PhotoMeta, PhotoRow)> = Vec::new();
        for meta in &photos {
            if self.photos.contains_key(&meta.id.0) {
                continue;
            }
            let mut row = meta
                .device
                .zip(meta.ino)
                .and_then(|key| by_inode.get(&key))
                .and_then(|rows| rows.first())
                .cloned();
            if row.is_none() {
                row = by_capture
                    .get(&(meta.shutter_count, meta.file_size))
                    .and_then(|rows| rows.first())
                    .cloned();
            }
            if let Some(row) = row {
                renames.push((meta.clone(), row));
            }
        }

        // The new row has to exist before anything points at it: `ratings`, `batch_photos` and
        // `cursor` all carry foreign keys into `photos`, and the pragmas have them enforced, so
        // moving a rating to an id with no row yet is a constraint failure rather than a lost row.
        let now = crate::store::now_ms();
        let fresh: Vec<PhotoRow> = renames
            .iter()
            .map(|(meta, _)| photo_row(meta, now))
            .collect();
        records::upsert_photos(&self.db, &fresh)?;

        for (meta, old) in &renames {
            move_state(&self.db, old.id, meta.id.0)?;
            moved.push((old.id, meta.id.0));
        }
        // The old ids go last: a row that is about to be re-pointed must not be mistaken for a
        // vanished file while the new ones are being written.
        for (meta, _) in &renames {
            let _ = records::set_photo_present(&self.db, meta.id.0, true);
        }
        for (old_id, _) in &moved {
            self.photos.remove(old_id);
        }

        self.ingest(photos)?;
        Ok(moved)
    }

    /// Runs the batcher over the current photos and signatures, honouring visited batches.
    ///
    /// Returns `None` when there is nothing to batch, so a caller can tell "no change" from
    /// "changed to nothing".
    fn rebatch(&mut self) -> Option<Vec<Batch>> {
        if self.photos.is_empty() {
            return None;
        }
        let frozen: Vec<Batch> = records::batches(&self.db)
            .ok()?
            .into_iter()
            .map(batch_from_row)
            .collect();
        let ordered: Vec<&PhotoMeta> = self
            .order
            .iter()
            .filter_map(|id| self.photos.get(&id.0))
            .collect();
        let outcome = batch_with(&ordered, &self.sigs, &frozen, Default::default());

        let rows: Vec<BatchRow> = outcome
            .batches
            .iter()
            .map(|batch| BatchRow {
                id: batch.id.0,
                index: batch.index,
                provisional: batch.provisional,
                first_ordinal: batch
                    .photo_ids
                    .first()
                    .and_then(|id| self.ordinal_of(*id))
                    .unwrap_or(0),
                last_ordinal: batch
                    .photo_ids
                    .last()
                    .and_then(|id| self.ordinal_of(*id))
                    .unwrap_or(0),
                photo_ids: batch.photo_ids.iter().map(|id| id.0).collect(),
            })
            .collect();
        records::replace_unvisited_batches(&self.db, &rows).ok()?;
        // Read back rather than trusting the outcome: the database is the source of truth, and it
        // is the only thing that knows which batches are frozen.
        Some(
            records::batches(&self.db)
                .ok()?
                .into_iter()
                .map(batch_from_row)
                .collect(),
        )
    }

    fn ordinal_of(&self, id: PhotoId) -> Option<i64> {
        self.order
            .iter()
            .position(|other| *other == id)
            .map(|at| at as i64)
    }

    /// Which batch a photo is in, and where. A photo the session has not seen reports batch 0,
    /// because the contract says the API accepts any photo.
    fn batch_of(&self, photo: PhotoId) -> (BatchId, u32) {
        for batch in &self.batches {
            if batch.photo_ids.contains(&photo) {
                return (batch.id, batch.index);
            }
        }
        (BatchId(0), 0)
    }
}

/// Moves everything attached to a photo id over to a new one, for a renamed file (REV-68).
///
/// Four tables hold state keyed by photo id: the rating, the undo log, the batch membership and the
/// "where the user was" row. All four move together in one transaction, because a half-moved
/// rename is worse than no rename: the user would see a rating on a photo with no way to undo it.
///
/// The old row is left in place and marked absent rather than deleted, so undoing a rename of a
/// rename is still possible, and so a file that comes back under its old name finds its history.
fn move_state(db: &Db, from: u64, to: u64) -> crate::store::Result<()> {
    if from == to {
        return Ok(());
    }
    let from = crate::store::db::id_to_i64(from);
    let to = crate::store::db::id_to_i64(to);
    db.transaction(|tx| {
        // The rating row moves, but only if the new id does not already have one: a photo that was
        // rated under both names keeps the rating it had.
        tx.execute(
            "UPDATE ratings SET photo_id = ?2
              WHERE photo_id = ?1
                AND NOT EXISTS (SELECT 1 FROM ratings WHERE photo_id = ?2)",
            rusqlite::params![from, to],
        )?;
        // The undo log is a history of what happened, not of what a photo is called, so every
        // entry moves: undoing a rename has to be able to walk back through it.
        tx.execute(
            "UPDATE history SET photo_id = ?2 WHERE photo_id = ?1",
            rusqlite::params![from, to],
        )?;
        // Batch membership cannot have two rows for the same pair, so the new one wins.
        tx.execute(
            "DELETE FROM batch_photos WHERE photo_id = ?1
               AND EXISTS (SELECT 1 FROM batch_photos WHERE photo_id = ?2 AND batch_id = batch_photos.batch_id)",
            rusqlite::params![from, to],
        )?;
        tx.execute(
            "UPDATE batch_photos SET photo_id = ?2 WHERE photo_id = ?1",
            rusqlite::params![from, to],
        )?;
        tx.execute(
            "UPDATE visited SET last_photo_id = ?2
              WHERE last_photo_id = ?1
                AND NOT EXISTS (SELECT 1 FROM visited WHERE last_photo_id = ?2 AND batch_id = visited.batch_id)",
            rusqlite::params![from, to],
        )?;
        tx.execute(
            "UPDATE cursor SET photo_id = ?2 WHERE photo_id = ?1",
            rusqlite::params![from, to],
        )?;
        Ok(())
    })
}

fn batch_from_row(row: BatchRow) -> Batch {
    Batch {
        id: BatchId(row.id),
        index: row.index,
        photo_ids: row.photo_ids.into_iter().map(PhotoId).collect(),
        provisional: row.provisional,
    }
}

fn photo_row(meta: &PhotoMeta, now: i64) -> PhotoRow {
    // The scanner owns this definition, and `photos_group_key` is an index over it. Re-deriving the
    // stem here instead meant splitting on the first dot in the whole path, so every file in a
    // folder called `Sat.1` was keyed `Sat` and the index could not separate them.
    let group_key = crate::meta::group_key(&meta.rel_path);
    let meta_json = serde_json::to_string(meta).ok();
    PhotoRow {
        id: meta.id.0,
        rel_path: meta.rel_path.clone(),
        group_key,
        companions: meta.companions.clone(),
        file_size: meta.file_size,
        mtime_ms: None,
        // The stat identity. A rename leaves both of these alone, which is what lets a rescan
        // recognise the file and carry its rating over (REV-68).
        device: meta.device,
        ino: meta.ino,
        meta_json,
        ordinal: None,
        first_seen_at_ms: now,
        last_seen_at_ms: now,
        present: true,
    }
}

fn read_cursor(db: &Db) -> crate::store::Result<Option<Cursor>> {
    Ok(records::cursor(db)?.and_then(|row| {
        Some(Cursor {
            batch: BatchId(row.batch_id?),
            photo: PhotoId(row.photo_id?),
        })
    }))
}

/// Imports the ratings a folder already carries in its sidecars (docs/contracts/session-api.md).
///
/// A folder rated in Lightroom and then opened in Firstcut has no session database, and its
/// ratings live only in the sidecars. Importing them is what stops the first cull from silently
/// discarding a shoot that was already rated. A sidecar that is not XMP is reported through the
/// listener and never overwritten.
pub fn import_ratings_from_sidecars(folder: &Path, photos: &[PhotoMeta]) -> Vec<(PhotoId, Rating)> {
    let mapping = XmpMapping::default();
    let mut imported = Vec::new();
    for meta in photos {
        let sidecar = folder.join(sidecar_path(&meta.rel_path));
        let Ok(Some(values)) = read_sidecar(&sidecar) else {
            continue;
        };
        let Some(rating) = mapping.rating_from(&values, RatingMode::Stars) else {
            continue;
        };
        if rating.is_neutral() {
            continue;
        }
        imported.push((meta.id, rating));
    }
    imported
}

/// Tier counts for the Finish summary, in the current mode (task.md §6.1).
#[must_use]
pub fn tier_counts(snapshot: &SessionSnapshot, mode: RatingMode) -> HashMap<Tier, usize> {
    let mut counts: HashMap<Tier, usize> = Tier::ALL.into_iter().map(|tier| (tier, 0)).collect();
    for rating in snapshot.ratings.values() {
        *counts.entry(rating.tier(mode)).or_insert(0) += 1;
    }
    counts
}

// ─────────────────────────────────────────────────────────── finish cull (task.md §9.7)

fn fs_write_marker(path: &Path) -> std::io::Result<()> {
    std::fs::write(
        path,
        "Firstcut moved photos here in a Finish Cull run, so this folder is skipped when the shoot is scanned.\n",
    )
}

/// What a Finish run did, for the report the app shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishRun {
    /// Which run this was; `undo_finish` reverses the highest one.
    pub finish_id: i64,
    pub summary: crate::fileops::ExecutionSummary,
    /// Every operation, in order, including the ones that failed and why.
    pub executed: Vec<crate::fileops::ExecutedOp>,
}

/// What an Undo Finish did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishUndo {
    pub finish_id: Option<i64>,
    pub summary: crate::fileops::ExecutionSummary,
    /// True when there has been no Finish run to undo.
    pub nothing_to_undo: bool,
}

impl Session {
    /// The dry run: what Finish *would* do, without touching anything.
    ///
    /// The rating mode is the session's own, not the caller's, so the preview cannot disagree with
    /// the filmstrip about what a keep is (REV-78). Every "kept" decision goes through
    /// [`Rating::tier`], the same function the rings and the tier counts use.
    pub fn plan_finish(
        &self,
        options: &crate::fileops::FinishOptions,
    ) -> Result<crate::fileops::FinishPlan> {
        // Queued sidecar writes are flushed first: a plan is built from, and then moves, the files
        // the writer is about to touch.
        self.writer.flush();
        let state = self.state();
        let rows: Vec<PhotoRow> = records::photos_in_order(&state.db)?
            .into_iter()
            .filter(|row| row.present)
            .collect();
        let ratings = records::ratings(&state.db)?;
        let visited = records::visited_batch_ids(&state.db)?;
        let unvisited = state
            .batches
            .iter()
            .filter(|batch| !visited.contains(&batch.id.0))
            .count();
        let options = crate::fileops::FinishOptions {
            rating_mode: state.rating_mode,
            ..options.clone()
        };
        Ok(crate::fileops::plan_finish(
            &self.folder,
            &rows,
            &ratings,
            &options,
            unvisited,
        ))
    }

    /// Does what the plan says and logs every operation, so it can be undone even after a
    /// relaunch: the log is in the session database, not in memory.
    ///
    /// The plan is not re-decided here. It is walked one operation at a time and the only judgement
    /// made is the safety one: a destination that has become occupied fails that operation instead
    /// of overwriting the file that is there.
    pub fn execute_finish(&self, plan: &crate::fileops::FinishPlan) -> Result<FinishRun> {
        self.writer.flush();
        let executed = crate::fileops::execute_ops(&plan.ops);
        let finish_id = {
            let state = self.state();
            let finish_id = records::last_finish_id(&state.db)?.unwrap_or(0) + 1;
            let now = crate::store::now_ms();
            for (seq, op) in executed.iter().enumerate() {
                records::log_file_op(
                    &state.db,
                    &records::FileOpRow {
                        id: 0,
                        finish_id,
                        seq: seq as i64,
                        kind: op.kind.as_str().to_string(),
                        src: op.src.clone(),
                        dst: op.dst.clone(),
                        size_bytes: op.size_bytes,
                        status: op.status.to_string(),
                        error: op.error.clone(),
                        at_ms: now,
                    },
                )?;
            }
            finish_id
        };
        self.mark_finish_folders(&executed);
        self.after_files_moved();
        Ok(FinishRun {
            finish_id,
            summary: crate::fileops::summarize(&executed),
            executed,
        })
    }

    /// Walks the last Finish run backwards.
    ///
    /// Reads the log from the database rather than from anything held in memory, so "Undo Finish"
    /// still works after the app has been quit and reopened, which is when somebody who has just
    /// moved a shoot actually reaches for it.
    pub fn undo_finish(&self) -> Result<FinishUndo> {
        self.writer.flush();
        let (finish_id, rows) = {
            let state = self.state();
            let Some(finish_id) = records::last_finish_id(&state.db)? else {
                return Ok(FinishUndo {
                    finish_id: None,
                    summary: crate::fileops::ExecutionSummary::default(),
                    nothing_to_undo: true,
                });
            };
            (finish_id, records::file_ops(&state.db, finish_id)?)
        };

        // Only operations that happened can be reversed, and the log's `dst` is where the file is
        // now, which is exactly what has to be put back.
        let executed: Vec<crate::fileops::ExecutedOp> = rows
            .iter()
            .filter_map(|row| {
                Some(crate::fileops::ExecutedOp {
                    kind: crate::fileops::FileOpKind::parse(&row.kind)?,
                    src: row.src.clone(),
                    dst: row.dst.clone(),
                    size_bytes: row.size_bytes,
                    status: if row.status == "done" {
                        "done"
                    } else {
                        "failed"
                    },
                    error: row.error.clone(),
                })
            })
            .collect();
        if executed
            .iter()
            .any(|op| op.is_done() && !op.kind.is_undoable())
        {
            return Err(SessionError::CannotUndo(
                "the last Finish run included a permanent delete, which cannot be undone"
                    .to_string(),
            ));
        }

        let undone = crate::fileops::undo_ops(&executed);
        {
            // The run is consumed, so a second "Undo Finish" reverses the one before it instead
            // of trying the same files again.
            let state = self.state();
            records::clear_file_ops(&state.db, finish_id)?;
        }
        self.after_files_moved();
        Ok(FinishUndo {
            finish_id: Some(finish_id),
            summary: crate::fileops::summarize(&undone),
            nothing_to_undo: false,
        })
    }

    /// Marks the top-level folders Finish put files into *inside the shoot*, so a rescan does not
    /// find the photos it just moved and bring them back into the cull.
    fn mark_finish_folders(&self, executed: &[crate::fileops::ExecutedOp]) {
        for op in executed.iter().filter(|op| op.is_done()) {
            let Some(dst) = op.dst.as_deref() else {
                continue;
            };
            let Ok(relative) = Path::new(dst).strip_prefix(&self.folder) else {
                continue;
            };
            let mut parts = relative.components();
            let (Some(first), Some(_file)) = (parts.next(), parts.next()) else {
                continue; // a file straight in the shoot folder is not inside a new folder
            };
            let marker = self
                .folder
                .join(first.as_os_str())
                .join(crate::meta::FINISH_MARKER);
            if !marker.exists() {
                let _ = fs_write_marker(&marker);
            }
        }
    }

    /// Re-reads the folder after files have moved: the session's idea of where things are is
    /// wrong, and a second plan built on it would move files that are not there. A Finish run can
    /// empty the folder, which is the run succeeding rather than the folder breaking, so a failed
    /// re-read is not an error here.
    fn after_files_moved(&self) {
        let _ = self.rescan();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{AfInfo, CaptureTime, EmbeddedPreview, FileKind, RawFormat, TimeSource};
    use crate::store::rating::{ColorLabel, Flag};
    use std::fs;

    /// A photo with no file behind it, so a test can drive a session without a RAW corpus.
    ///
    /// Every field the batcher reads is real; only the path is invented, which is what makes the
    /// ordering and batching assertions in these tests mean anything.
    fn photo(index: u32, ms: i64) -> PhotoMeta {
        let rel_path = format!("IMG_{index:04}.CR3");
        PhotoMeta {
            id: crate::batch::photo_id(&rel_path),
            companions: Vec::new(),
            kind: FileKind::Raw(RawFormat::Cr3),
            file_size: 1_000,
            capture_time: Some(CaptureTime {
                unix_ms: ms,
                subsec_resolution_ms: 10,
                offset_minutes: Some(-360),
                source: TimeSource::Exif,
            }),
            shutter_count: Some(1_000 + u64::from(index)),
            file_number: Some(index),
            camera_make: Some("Canon".to_string()),
            camera_model: Some("Canon EOS R8".to_string()),
            camera_serial: Some("122022006902".to_string()),
            lens_model: Some("EF70-200mm f/2.8L IS II USM".to_string()),
            focal_length_mm: Some(200.0),
            exposure_time_s: Some(0.0005),
            f_number: Some(2.8),
            iso: Some(800),
            exposure_comp_ev: Some(0.0),
            metering_mode: Some("Evaluative".to_string()),
            drive_mode: Some("Continuous, High+".to_string()),
            shutter_mode: Some("Electronic First Curtain".to_string()),
            orientation: 1,
            width: 6000,
            height: 4000,
            af: Some(AfInfo {
                area_mode: Some("AF Point Expansion (8 point)".to_string()),
                image_width: 6000,
                image_height: 4000,
                points: Vec::new(),
                points_in_focus: Vec::new(),
            }),
            // A distinct inode per photo, so a rename in a test is a rename and not a coincidence.
            device: Some(1),
            ino: Some(9_000 + i64::from(index)),
            preview: Some(EmbeddedPreview {
                range: crate::meta::ByteRange {
                    offset: 1000,
                    len: 200_000,
                },
                width: 1620,
                height: 1080,
            }),
            warnings: Vec::new(),
            rel_path,
        }
    }

    /// A shoot of `count` photos, `interval_ms` apart, so the batcher finds one burst.
    fn burst(count: u32, interval_ms: i64) -> Vec<PhotoMeta> {
        (0..count)
            .map(|i| photo(i, 1_000_000 + i64::from(i) * interval_ms))
            .collect()
    }

    /// A real CR3 on disk, so the scanner, the session database and the fingerprint all see
    /// something the production path can actually read. The 42 GB corpus is not available to a
    /// unit test, and a file of placeholder bytes would be (correctly) rejected as unparseable.
    fn write_cr3(folder: &Path, name: &str, index: u32) {
        let seconds = 10 + index;
        fs::write(
            folder.join(name),
            crate::meta::cr3::SyntheticCr3::r8()
                .at(&format!("2026:08:27 19:54:{seconds:02}"))
                .build(),
        )
        .unwrap();
    }

    /// A session over photos that exist only in memory, with a real database and a real folder.
    ///
    /// `Session::open` scans the disk, so a test that wants particular metadata builds it here and
    /// hands it to the same ingest path the scanner feeds. Everything below that line — ordering,
    /// batching, the database, the XMP writer — is the production code.
    fn session_with(photos: Vec<PhotoMeta>) -> (tempfile::TempDir, tempfile::TempDir, Session) {
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let mut photos = photos;
        for (index, meta) in photos.iter_mut().enumerate() {
            // A real, parseable CR3, so a reopen or a rescan in a later test goes through the
            // scanner rather than finding a folder of files it correctly refuses.
            write_cr3(folder.path(), &meta.rel_path, index as u32 + 1);
            // The stat identity has to come from the file that is actually there, or a rename
            // would not be recognisable and the reconcile test would be testing nothing.
            let stat = fs::metadata(folder.path().join(&meta.rel_path)).unwrap();
            use std::os::unix::fs::MetadataExt;
            meta.device = Some(stat.dev() as i64);
            meta.ino = Some(stat.ino() as i64);
        }
        let scan = crate::meta::ScanResult {
            skipped: Vec::new(),
            photos,
        };
        let session =
            Session::from_scan(scan, folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        (sessions, folder, session)
    }

    /// A synthetic CR3 whose bytes depend on `index`, so two folders holding the same file names
    /// have the same sizes but different contents — which is what a copy of a shoot looks like.
    fn synthetic_bytes(index: u32) -> Vec<u8> {
        crate::meta::cr3::SyntheticCr3::r8()
            .subsec(&format!("{index:02}"))
            .build()
    }

    fn empty_session() -> (tempfile::TempDir, tempfile::TempDir, Session) {
        session_with(burst(12, 90))
    }

    #[test]
    fn opening_a_shoot_orders_and_batches_it() {
        let (_s, _f, session) = empty_session();
        let snapshot = session.snapshot();
        assert_eq!(snapshot.photos.len(), 12);
        assert_eq!(snapshot.folder, session.folder().display().to_string());
        // 12 frames 90 ms apart is one burst, and the batcher's own thresholds are what decide it.
        assert_eq!(snapshot.batches.len(), 1, "{:?}", snapshot.batches);
        assert_eq!(snapshot.batches[0].photo_ids.len(), 12);
        // The snapshot's photos are in capture order, not path order.
        let ordinals: Vec<u32> = snapshot
            .photos
            .iter()
            .map(|meta| meta.file_number.expect("a number"))
            .collect();
        assert_eq!(ordinals, (0..12).collect::<Vec<_>>());
    }

    #[test]
    fn a_rating_reaches_the_sidecar_and_the_original_is_untouched() {
        // The two promises in docs/contracts/session-api.md, in one test: the rating lands in the
        // sidecar within the debounce, and the RAW is byte-for-byte what it was.
        let (_s, folder, session) = empty_session();
        let meta = session.snapshot().photos[0].clone();
        let original = folder.path().join(&meta.rel_path);
        let before = fs::metadata(&original).unwrap();
        let before_bytes = fs::read(&original).unwrap();

        session.set_rating(meta.id, Rating::stars(4)).unwrap();
        session.flush();

        let sidecar = folder.path().join(format!("{}.xmp", meta.rel_path));
        let values = crate::xmp::read_sidecar(&sidecar)
            .unwrap()
            .expect("a sidecar");
        assert_eq!(
            values.rating,
            Some(4),
            "the rating reached Lightroom's copy"
        );

        let after = fs::metadata(&original).unwrap();
        assert_eq!(
            fs::read(&original).unwrap(),
            before_bytes,
            "the RAW is unchanged"
        );
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    }

    #[test]
    fn a_rating_is_in_the_snapshot_and_survives_a_reopen() {
        let (sessions, folder, session) = empty_session();
        let meta = session.snapshot().photos[3].clone();
        session.set_rating(meta.id, Rating::stars(5)).unwrap();
        session.flush();
        drop(session);

        // Reopening the same folder finds the same database, so the rating is still there.
        let reopened =
            Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        let snapshot = reopened.snapshot();
        assert_eq!(snapshot.ratings.get(&meta.id.0), Some(&Rating::stars(5)));
        assert_eq!(
            snapshot.folder,
            folder.path().canonicalize().unwrap().display().to_string()
        );
    }

    #[test]
    fn a_rejected_rating_writes_minus_one() {
        // task.md §6.2: an explicit reject is `xmp:Rating="-1"`, which is what Lightroom shows.
        let (_s, folder, session) = empty_session();
        let meta = session.snapshot().photos[0].clone();
        session
            .set_rating_with(meta.id, Rating::neutral(), Some(Flag::Reject), None)
            .unwrap();
        session.flush();

        let sidecar = folder.path().join(format!("{}.xmp", meta.rel_path));
        let values = crate::xmp::read_sidecar(&sidecar)
            .unwrap()
            .expect("a sidecar");
        assert_eq!(values.rating, Some(-1));
    }

    #[test]
    fn a_colour_label_reaches_the_sidecar() {
        let (_s, folder, session) = empty_session();
        let meta = session.snapshot().photos[0].clone();
        session
            .set_rating_with(
                meta.id,
                Rating::new(2, Flag::Pick, Some(ColorLabel::Green), false),
                None,
                None,
            )
            .unwrap();
        session.flush();

        let sidecar = folder.path().join(format!("{}.xmp", meta.rel_path));
        let values = crate::xmp::read_sidecar(&sidecar)
            .unwrap()
            .expect("a sidecar");
        assert_eq!(values.rating, Some(2));
        assert_eq!(values.label.as_deref(), Some("Green"));
    }

    #[test]
    fn undo_and_redo_walk_the_same_log() {
        let (_s, folder, session) = empty_session();
        let meta = session.snapshot().photos[0].clone();

        session.set_rating(meta.id, Rating::stars(1)).unwrap();
        session.set_rating(meta.id, Rating::stars(4)).unwrap();
        assert_eq!(
            session.snapshot().ratings.get(&meta.id.0),
            Some(&Rating::stars(4))
        );

        let undone = session.undo().expect("something to undo");
        assert_eq!(undone.after, Rating::stars(1), "undo goes back one step");
        assert_eq!(
            session.snapshot().ratings.get(&meta.id.0),
            Some(&Rating::stars(1))
        );

        let redone = session.redo().expect("something to redo");
        assert_eq!(redone.after, Rating::stars(4));
        assert_eq!(
            session.snapshot().ratings.get(&meta.id.0),
            Some(&Rating::stars(4))
        );

        // The sidecar follows the database, so Lightroom sees the same value the app does.
        session.flush();
        let sidecar = folder.path().join(format!("{}.xmp", meta.rel_path));
        assert_eq!(
            crate::xmp::read_sidecar(&sidecar).unwrap().unwrap().rating,
            Some(4)
        );
    }

    #[test]
    fn undo_past_the_start_returns_nothing_rather_than_failing() {
        let (_s, _f, session) = empty_session();
        assert!(session.undo().is_none());
        assert!(session.redo().is_none());
    }

    #[test]
    fn undo_carries_the_batch_it_was_made_in() {
        // §6.3: a change records the batch position so undo can navigate back across batches.
        let (_s, _f, session) = session_with(photo_run());
        let snapshot = session.snapshot();
        assert!(
            snapshot.batches.len() > 2,
            "the fixture needs several batches"
        );
        let target = &snapshot.photos[9];
        let change = session.set_rating(target.id, Rating::stars(3)).unwrap();
        let undone = session.undo().expect("an undo");
        assert_eq!(undone.photo, target.id);
        assert_eq!(undone.batch, change.batch);
        assert_eq!(undone.batch_index, change.batch_index);
    }

    /// Twelve bursts three seconds apart: a shoot with real boundaries between them.
    fn photo_run() -> Vec<PhotoMeta> {
        (0..12u32)
            .flat_map(|run| {
                (0..5u32).map(move |frame| {
                    photo(
                        run * 5 + frame,
                        3_000 * i64::from(run) + 90 * i64::from(frame),
                    )
                })
            })
            .collect()
    }

    #[test]
    fn rapid_changes_to_one_photo_collapse_into_a_single_sidecar_write() {
        // The debounce is per photo, not per keystroke: five stars pressed in a second is one
        // file write with the last value (docs/contracts/session-api.md).
        let (_s, folder, session) = empty_session();
        let meta = session.snapshot().photos[0].clone();
        for stars in 1..=5u8 {
            session.set_rating(meta.id, Rating::stars(stars)).unwrap();
        }
        session.flush();
        let sidecar = folder.path().join(format!("{}.xmp", meta.rel_path));
        let text = fs::read_to_string(&sidecar).unwrap();
        assert_eq!(
            text.matches("xmp:Rating").count(),
            1,
            "one attribute, one write"
        );
        assert!(text.contains("xmp:Rating=\"5\""), "the last value wins");
    }

    #[test]
    fn a_visited_batch_is_frozen_against_re_batching() {
        // task.md §5.4: a batch the user has been in is never re-cut under them. A signature that
        // would otherwise merge it into its neighbour must not.
        let (_s, _f, session) = session_with(photo_run());
        let before = session.snapshot().batches;
        let batch = before[0].clone();
        session.mark_visited(batch.id).unwrap();

        // Identical signatures say "these frames look the same", which is exactly the evidence
        // that would otherwise collapse the boundary.
        let sig = VisualSig {
            dhash: 0,
            hist: [0; 48],
        };
        let sigs: Vec<(PhotoId, VisualSig)> = before
            .iter()
            .flat_map(|b| b.photo_ids.iter().map(|id| (*id, sig)))
            .collect();
        session.submit_visual_sigs(sigs);

        let after = session.snapshot().batches;
        let frozen = after
            .iter()
            .find(|b| b.id == batch.id)
            .expect("the visited batch is still there");
        assert_eq!(
            frozen.photo_ids, batch.photo_ids,
            "a visited batch must not change"
        );
    }

    #[test]
    fn marking_a_batch_visited_does_not_invent_a_position() {
        let (_s, _f, session) = session_with(photo_run());
        let batch = session.snapshot().batches[1].clone();
        session.mark_visited(batch.id).unwrap();
        let snapshot = session.snapshot();
        assert!(snapshot.visited.contains(&batch.id.0));
        // "Seen" is not "the user reached the end of it". Recording the batch's last photo here
        // would put them back on a frame they never looked at, so only `set_cursor` writes a
        // position.
        assert!(
            !snapshot.last_photo_in_batch.contains_key(&batch.id.0),
            "marking a batch seen must not claim a position in it"
        );
    }

    #[test]
    fn setting_the_cursor_records_the_position_in_the_batch() {
        let (sessions, folder, session) = session_with(photo_run());
        let batch = session.snapshot().batches[1].clone();
        let photo = batch.photo_ids[0];
        session
            .set_cursor(Cursor {
                batch: batch.id,
                photo,
            })
            .unwrap();
        let snapshot = session.snapshot();
        assert_eq!(
            snapshot.last_photo_in_batch.get(&batch.id.0).copied(),
            Some(photo.0),
            "set_cursor is what records where the user is"
        );
        drop(session);

        // And it is the position that comes back on reopen, not a later `mark_visited` clobbering it.
        let reopened =
            Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        assert_eq!(
            reopened
                .snapshot()
                .last_photo_in_batch
                .get(&batch.id.0)
                .copied(),
            Some(photo.0)
        );
    }

    #[test]
    fn the_cursor_survives_a_reopen() {
        let (sessions, folder, session) = session_with(photo_run());
        let snapshot = session.snapshot();
        let batch = snapshot.batches[2].clone();
        let photo = batch.photo_ids[1];
        session
            .set_cursor(Cursor {
                batch: batch.id,
                photo,
            })
            .unwrap();
        drop(session);

        let reopened =
            Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        let cursor = reopened.snapshot().cursor.expect("the cursor");
        assert_eq!(cursor.batch, batch.id);
        assert_eq!(cursor.photo, photo);
    }

    #[test]
    fn switching_rating_mode_keeps_the_data_the_user_set() {
        // task.md §6: switching mode is allowed and nothing is lost. A 5-star photo is a keep in
        // keep mode and 5 stars again in stars mode.
        let (_s, _f, session) = empty_session();
        let meta = session.snapshot().photos[0].clone();
        session.set_rating(meta.id, Rating::stars(5)).unwrap();
        assert_eq!(session.rating_mode(), RatingMode::Stars);

        session.set_rating_mode(RatingMode::KeepNotKeep).unwrap();
        assert_eq!(session.rating_mode(), RatingMode::KeepNotKeep);
        let stored = session.snapshot().ratings[&meta.id.0];
        assert_eq!(stored.stars, 5, "the stars are still there underneath");
        assert!(
            crate::store::rating::display_rating(&stored, RatingMode::KeepNotKeep).keep,
            "and it displays as a keep"
        );

        session.set_rating_mode(RatingMode::Stars).unwrap();
        assert_eq!(session.snapshot().ratings[&meta.id.0], Rating::stars(5));
    }

    #[test]
    fn a_keep_writes_five_stars_in_stars_mode() {
        // task.md §6: "a keep ↔ 5 stars by default", so Lightroom shows the same decision.
        let (_s, folder, session) = empty_session();
        let meta = session.snapshot().photos[0].clone();
        session.set_rating(meta.id, Rating::keep()).unwrap();
        session.flush();
        let sidecar = folder.path().join(format!("{}.xmp", meta.rel_path));
        assert_eq!(
            crate::xmp::read_sidecar(&sidecar).unwrap().unwrap().rating,
            Some(5)
        );
    }

    #[test]
    fn tier_counts_add_up_to_the_number_of_ratings() {
        let (_s, _f, session) = empty_session();
        let photos = session.snapshot().photos;
        session.set_rating(photos[0].id, Rating::stars(5)).unwrap();
        session.set_rating(photos[1].id, Rating::stars(3)).unwrap();
        session.set_rating(photos[2].id, Rating::stars(1)).unwrap();
        session
            .set_rating_with(photos[3].id, Rating::neutral(), Some(Flag::Reject), None)
            .unwrap();

        let snapshot = session.snapshot();
        let counts = tier_counts(&snapshot, RatingMode::Stars);
        assert_eq!(counts[&Tier::Keep], 1);
        assert_eq!(counts[&Tier::Good], 1);
        assert_eq!(counts[&Tier::Maybe], 1);
        assert_eq!(counts[&Tier::Rejected], 1);
        assert_eq!(counts.values().sum::<usize>(), 4);
    }

    #[test]
    fn a_renamed_file_keeps_its_rating() {
        // REV-68: `PhotoId` is a hash of the path, so a rename would otherwise leave the rating
        // stranded on a row nothing points at. The rescan recognises the file by its inode, which
        // a rename in Finder does not change, and carries the rating to the new id.
        let (_sessions, folder, session) = session_with(burst(3, 90));
        let before = session.snapshot();
        let meta = before.photos[1].clone();
        let change = session.set_rating(meta.id, Rating::stars(5)).unwrap();
        session.flush();

        let old = folder.path().join(&meta.rel_path);
        let new = folder.path().join("renamed_by_the_user.CR3");
        fs::rename(&old, &new).unwrap();

        let scan = session.rescan().unwrap();
        assert_eq!(scan.photos.len(), 3, "all three files are still there");
        let after = session.snapshot();
        let renamed = after
            .photos
            .iter()
            .find(|p| p.rel_path == "renamed_by_the_user.CR3")
            .expect("the renamed file is in the shoot");
        assert_ne!(renamed.id, meta.id, "the path really did change the id");
        assert_eq!(
            after.ratings.get(&renamed.id.0),
            Some(&Rating::stars(5)),
            "the rating followed the file"
        );
        assert_eq!(after.ratings.len(), 1, "and there is still only one rating");
        // The undo log moved with it, so undo still works after the rename, and it clears the
        // rating rather than restoring a stale one.
        let undone = session.undo().expect("the change is still undoable");
        assert_eq!(undone.photo, renamed.id, "undo is against the new id");
        assert_eq!(undone.before, change.after, "it took the 5 stars off");
        assert_eq!(undone.after, Rating::neutral(), "and left nothing behind");
        assert_eq!(
            session.snapshot().ratings.get(&renamed.id.0),
            Some(&Rating::neutral())
        );
    }

    #[test]
    fn a_file_that_vanishes_keeps_its_row_and_its_rating() {
        // A card read error must not lose work: the row stays, marked absent, so the rating comes
        // back when the file does.
        let (_sessions, folder, session) = session_with(burst(3, 90));
        let meta = session.snapshot().photos[2].clone();
        session.set_rating(meta.id, Rating::stars(4)).unwrap();
        fs::remove_file(folder.path().join(&meta.rel_path)).unwrap();

        session.rescan().unwrap();
        let after = session.snapshot();
        assert_eq!(after.photos.len(), 2, "the scan sees two files now");
        assert_eq!(
            after.ratings.get(&meta.id.0),
            Some(&Rating::stars(4)),
            "the rating outlives the file"
        );
        // The row is kept, not deleted: a file that comes back finds its history again.
        let restored = folder.path().join(&meta.rel_path);
        write_cr3(folder.path(), &meta.rel_path, 3);
        drop(restored);
        session.rescan().unwrap();
        assert_eq!(session.snapshot().photos.len(), 3, "the file came back");
        assert_eq!(
            session.snapshot().ratings.get(&meta.id.0),
            Some(&Rating::stars(4))
        );
    }

    #[test]
    fn a_file_the_parser_cannot_read_is_reported_not_dropped() {
        // task.md §8, at the session level: the shoot is incomplete and the app is told why.
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join("IMG_0001.CR3"), b"not a real raw file").unwrap();
        let session =
            Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        let skipped = session.skipped();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].rel_path, "IMG_0001.CR3");
        assert!(!skipped[0].reason.is_empty());
        assert!(session.snapshot().photos.is_empty());
    }

    #[test]
    fn an_empty_folder_opens_to_an_empty_session() {
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let session =
            Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        let snapshot = session.snapshot();
        assert!(snapshot.photos.is_empty());
        assert!(snapshot.batches.is_empty());
        assert!(snapshot.ratings.is_empty());
        // And every one of those is safe to ask about.
        assert!(session.undo().is_none());
        assert!(session.redo().is_none());
    }

    #[test]
    fn a_missing_folder_is_an_error_rather_than_an_empty_shoot() {
        let sessions = tempfile::tempdir().unwrap();
        let missing = sessions.path().join("nope");
        let err = Session::open_in(&missing, sessions.path(), Arc::new(NoListener))
            .expect_err("a missing folder must fail");
        assert!(matches!(err, SessionError::FolderNotFound(_)), "{err}");
    }

    #[test]
    fn a_moved_folder_keeps_its_session_and_says_so() {
        // docs/contracts/session-api.md: a moved folder is re-matched by fingerprint, and the app
        // shows a note when it happened.
        let parent = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let original = parent.path().join("Game1");
        fs::create_dir(&original).unwrap();
        for index in 1..=3u32 {
            write_cr3(&original, &format!("IMG_{index:04}.CR3"), index);
        }
        drop(Session::open_in(&original, sessions.path(), Arc::new(NoListener)).unwrap());

        let moved = parent.path().join("Game1 renamed");
        fs::rename(&original, &moved).unwrap();
        struct Notes(Mutex<Vec<String>>);
        impl SessionListener for Notes {
            fn batches_changed(&self, _batches: Vec<Batch>) {}
            fn files_changed(&self) {}
            fn xmp_error(&self, _photo: PhotoId, _message: String) {}
            fn session_moved(&self, from: String) {
                self.0.lock().unwrap().push(from);
            }
        }
        let notes = Arc::new(Notes(Mutex::new(Vec::new())));
        let session = Session::open_in(
            &moved,
            sessions.path(),
            Arc::clone(&notes) as Arc<dyn SessionListener>,
        )
        .unwrap();
        assert!(
            matches!(session.matched(), MatchKind::Moved { .. }),
            "{:?}",
            session.matched()
        );
        assert_eq!(notes.0.lock().unwrap().len(), 1, "the app is told once");
    }

    #[test]
    fn two_copies_of_a_shoot_get_their_own_sessions() {
        // A copy is not a move: the original is still there, so the second folder is a new shoot
        // and its ratings must not leak into the first (docs/contracts/session-api.md).
        let parent = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let a = parent.path().join("card1");
        let b = parent.path().join("card2");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        // Same names and sizes in both folders, so only the folder path tells them apart.
        for index in 1..=3u32 {
            let name = format!("IMG_{index:04}.CR3");
            let contents = synthetic_bytes(index);
            fs::write(a.join(&name), &contents).unwrap();
            fs::write(b.join(&name), &contents).unwrap();
        }
        let first = Session::open_in(&a, sessions.path(), Arc::new(NoListener)).unwrap();
        let photo = first.snapshot().photos[0].clone();
        first.set_rating(photo.id, Rating::stars(5)).unwrap();

        let second = Session::open_in(&b, sessions.path(), Arc::new(NoListener)).unwrap();
        assert!(
            second.snapshot().ratings.is_empty(),
            "a copy must not inherit the original's ratings"
        );
    }

    #[test]
    fn a_reshoot_in_the_same_folder_gets_its_own_session() {
        // Same path, different files: a different shoot, and the old session is left alone.
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        write_cr3(folder.path(), "IMG_0001.CR3", 1);
        let first = Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        let photo = first.snapshot().photos[0].clone();
        first.set_rating(photo.id, Rating::stars(5)).unwrap();
        first.flush();
        drop(first);

        // The new shoot does not inherit the old *database*. (A sidecar the old session left beside
        // `IMG_0001.CR3` is the photographer's data and would be imported by design, so the
        // stand-in for "a fresh card" clears it.)
        std::fs::remove_file(folder.path().join("IMG_0001.CR3.xmp")).ok();
        write_cr3(folder.path(), "IMG_0002.CR3", 2);
        let second =
            Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        assert!(second.snapshot().ratings.is_empty());
        assert_eq!(second.snapshot().photos.len(), 2);
    }

    #[test]
    fn importing_from_sidecars_recovers_a_shoot_rated_elsewhere() {
        // docs/contracts/session-api.md: a folder with sidecars but no database imports its
        // ratings, or the first cull would silently discard work done in Lightroom.
        let folder = tempfile::tempdir().unwrap();
        let photos = burst(3, 90);
        for meta in &photos {
            write_cr3(folder.path(), &meta.rel_path, 1);
            let sidecar = folder.path().join(format!("{}.xmp", meta.rel_path));
            let mut values = crate::xmp::XmpValues::rating(4);
            values.label = Some("Green".to_string());
            crate::xmp::write_sidecar(&sidecar, &values).unwrap();
        }

        let imported = import_ratings_from_sidecars(folder.path(), &photos);
        assert_eq!(imported.len(), 3, "every sidecar was read");
        let (id, rating) = imported[0];
        assert_eq!(id, photos[0].id);
        assert_eq!(rating.stars, 4);
        assert_eq!(rating.label, Some(ColorLabel::Green));
    }

    #[test]
    fn an_unrated_shoot_imports_nothing() {
        let folder = tempfile::tempdir().unwrap();
        let photos = burst(2, 90);
        for (index, meta) in photos.iter().enumerate() {
            write_cr3(folder.path(), &meta.rel_path, index as u32 + 1);
        }
        assert!(import_ratings_from_sidecars(folder.path(), &photos).is_empty());
    }

    #[test]
    fn a_sidecar_that_is_not_xmp_is_left_alone() {
        // The contract says a non-XMP file is reported, never overwritten.
        let folder = tempfile::tempdir().unwrap();
        let photos = burst(1, 90);
        let meta = &photos[0];
        write_cr3(folder.path(), &meta.rel_path, 1);
        let sidecar = folder.path().join(format!("{}.xmp", meta.rel_path));
        fs::write(&sidecar, "<?xml version=\"1.0\"?>\n<photos/>").unwrap();

        assert!(import_ratings_from_sidecars(folder.path(), &photos).is_empty());
        assert_eq!(
            fs::read_to_string(&sidecar).unwrap(),
            "<?xml version=\"1.0\"?>\n<photos/>",
            "the file is untouched"
        );
    }

    #[test]
    fn a_rating_is_visible_in_the_snapshot_before_the_sidecar_is_written() {
        // The < 1 ms promise: the database is the source of truth and the sidecar is the mirror,
        // so the app must never wait for the file write to show the user their own keystroke.
        let (_s, _f, session) = empty_session();
        let meta = session.snapshot().photos[0].clone();
        session.set_rating(meta.id, Rating::stars(3)).unwrap();
        assert_eq!(
            session.snapshot().ratings.get(&meta.id.0),
            Some(&Rating::stars(3)),
            "the rating is in the snapshot immediately, with no flush"
        );
    }

    #[test]
    fn a_change_carries_a_monotonic_id() {
        // The app uses it to drop a change it has already seen.
        let (_s, _f, session) = empty_session();
        let photos = session.snapshot().photos;
        let first = session.set_rating(photos[0].id, Rating::stars(1)).unwrap();
        let second = session.set_rating(photos[1].id, Rating::stars(2)).unwrap();
        let third = session.undo().unwrap();
        assert!(first.id < second.id);
        assert!(second.id < third.id, "an undo is itself a change");
    }

    // ─────────────────────────────────────────────── Finish Cull, on real files (task.md §9.7)

    /// Six JPEGs one second apart in a temp folder: real files, so Finish really moves things.
    fn six_jpegs() -> tempfile::TempDir {
        use crate::meta::exif::fixtures::{jpeg, tiff};
        let dir = tempfile::tempdir().unwrap();
        for n in 1..=6 {
            let stamp = format!("2026:08:27 10:00:{n:02}");
            fs::write(
                dir.path().join(format!("IMG_{n:04}.JPG")),
                jpeg(&tiff(&stamp, "", 100, "TEST", None), 600, 400),
            )
            .unwrap();
        }
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.ends_with(".xmp") && !n.starts_with('.'))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn finish_moves_the_unkept_photos_and_undo_puts_them_back() {
        let folder = six_jpegs();
        let sessions = tempfile::tempdir().unwrap();
        let session =
            Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        assert_eq!(session.snapshot().photos.len(), 6);

        // Keep 1 and 2 (4 and 5 stars), call 3 "good"; leave 4-6 unrated.
        let id = |n: u32| crate::batch::photo_id(&format!("IMG_{n:04}.JPG"));
        session.set_rating(id(1), Rating::stars(5)).unwrap();
        session.set_rating(id(2), Rating::stars(4)).unwrap();
        session.set_rating(id(3), Rating::stars(3)).unwrap();

        let options = crate::fileops::FinishOptions {
            unkept: crate::fileops::UnkeptAction::MoveToSubfolder("_Not kept".to_string()),
            ..Default::default()
        };
        let plan = session.plan_finish(&options).unwrap();
        // Dry run: nothing moved yet.
        assert_eq!(names(folder.path()).len(), 6);
        assert!(plan.ops.len() >= 4, "{:?}", plan.ops);

        let run = session.execute_finish(&plan).unwrap();
        assert!(run.summary.is_clean(), "{:?}", run.summary.failed);
        // Stars mode: 4 and 5 stars are the Keep tier. 3 stars is "Good", which Finish also
        // disposes of, because only the Keep tier is kept (task.md §9.7).
        assert_eq!(names(folder.path()), vec!["IMG_0001.JPG", "IMG_0002.JPG"]);
        assert_eq!(
            names(&folder.path().join("_Not kept")),
            vec![
                "IMG_0003.JPG",
                "IMG_0004.JPG",
                "IMG_0005.JPG",
                "IMG_0006.JPG"
            ]
        );

        let undo = session.undo_finish().unwrap();
        assert!(!undo.nothing_to_undo);
        assert!(undo.summary.is_clean(), "{:?}", undo.summary.failed);
        assert_eq!(names(folder.path()).len(), 6, "every file is back");
        assert_eq!(
            session.rescan().unwrap().photos.len(),
            6,
            "and they are in the cull again"
        );

        // A second undo has nothing left to reverse.
        assert!(session.undo_finish().unwrap().nothing_to_undo);
    }

    #[test]
    fn finish_never_moves_a_photo_the_filmstrip_shows_as_kept_in_either_mode() {
        // REV-78 at the level that moves files: a keep made in keep mode is 5 stars in stars mode.
        for mode in [RatingMode::Stars, RatingMode::KeepNotKeep] {
            let folder = six_jpegs();
            let sessions = tempfile::tempdir().unwrap();
            let session =
                Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
            session.set_rating_mode(mode).unwrap();
            let id = |n: u32| crate::batch::photo_id(&format!("IMG_{n:04}.JPG"));
            let keep = match mode {
                RatingMode::Stars => Rating::stars(4),
                RatingMode::KeepNotKeep => Rating::keep(),
            };
            session.set_rating(id(2), keep).unwrap();

            let options = crate::fileops::FinishOptions {
                unkept: crate::fileops::UnkeptAction::MoveToTrash,
                ..Default::default()
            };
            let plan = session.plan_finish(&options).unwrap();
            assert!(
                plan.ops.iter().all(|op| !op.from.ends_with("IMG_0002.JPG")),
                "{mode}: the kept photo is in the plan: {:?}",
                plan.ops
            );
            assert_eq!(
                crate::store::rating::display_tier(&keep, mode),
                Tier::Keep,
                "{mode}"
            );
        }
    }

    #[test]
    fn a_permanent_delete_run_cannot_be_undone() {
        let folder = six_jpegs();
        let sessions = tempfile::tempdir().unwrap();
        let session =
            Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        let id = |n: u32| crate::batch::photo_id(&format!("IMG_{n:04}.JPG"));
        session.set_rating(id(1), Rating::stars(5)).unwrap();
        let plan = session
            .plan_finish(&crate::fileops::FinishOptions {
                unkept: crate::fileops::UnkeptAction::DeletePermanently,
                ..Default::default()
            })
            .unwrap();
        assert!(!plan.is_undoable());
        assert!(plan.warnings.iter().any(|w| w.contains("cannot be undone")));
        let run = session.execute_finish(&plan).unwrap();
        assert!(!run.summary.undoable);
        assert_eq!(names(folder.path()), vec!["IMG_0001.JPG"]);
        assert!(matches!(
            session.undo_finish(),
            Err(SessionError::CannotUndo(_))
        ));
    }

    #[test]
    fn sidecar_settings_decide_whether_a_sidecar_appears() {
        let folder = six_jpegs();
        let sessions = tempfile::tempdir().unwrap();
        let session =
            Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        let sidecar = folder.path().join("IMG_0001.JPG.xmp");
        let id = crate::batch::photo_id("IMG_0001.JPG");

        // Off: the rating is in the database and nothing is written beside the photo.
        session
            .set_xmp_settings(XmpSettings {
                write_sidecars: false,
                sidecars_for_non_raw: true,
            })
            .unwrap();
        session.set_rating(id, Rating::stars(4)).unwrap();
        session.flush();
        assert!(!sidecar.exists(), "sidecars are off");
        assert_eq!(session.snapshot().ratings[&id.0].stars, 4);

        // On, but not for a JPEG: still nothing.
        session
            .set_xmp_settings(XmpSettings {
                write_sidecars: true,
                sidecars_for_non_raw: false,
            })
            .unwrap();
        session.flush();
        assert!(!sidecar.exists(), "no sidecars for non-RAW files");

        // Fully on: the rating that was skipped is written now.
        session.set_xmp_settings(XmpSettings::default()).unwrap();
        session.flush();
        let text = fs::read_to_string(&sidecar).expect("the skipped sidecar is backfilled");
        assert!(text.contains("xmp:Rating=\"4\""), "{text}");
    }

    #[test]
    fn a_first_open_imports_the_ratings_already_in_sidecars() {
        // Rated in Lightroom, opened in Firstcut for the first time: no database, ratings only in
        // the sidecars. They must be there, and the sidecars must be left exactly as they were.
        let folder = six_jpegs();
        for (name, stars) in [("IMG_0001.JPG", 5u8), ("IMG_0003.JPG", 2)] {
            crate::xmp::write_sidecar(
                &folder.path().join(format!("{name}.xmp")),
                &crate::xmp::document::XmpValues::rating(i64::from(stars)),
            )
            .unwrap();
        }
        let before = fs::read_to_string(folder.path().join("IMG_0001.JPG.xmp")).unwrap();

        let sessions = tempfile::tempdir().unwrap();
        let session =
            Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        let ratings = session.snapshot().ratings;
        let stars = |n: u32| {
            ratings
                .get(&crate::batch::photo_id(&format!("IMG_{n:04}.JPG")).0)
                .map(|r| r.stars)
        };
        assert_eq!(stars(1), Some(5));
        assert_eq!(stars(3), Some(2));
        assert_eq!(stars(2), None, "an unrated photo stays unrated");

        session.flush();
        assert_eq!(
            fs::read_to_string(folder.path().join("IMG_0001.JPG.xmp")).unwrap(),
            before,
            "importing must not rewrite the sidecar"
        );
    }

    #[test]
    fn a_rescan_after_everything_moved_away_is_not_an_error() {
        // A Finish that moves every photo out (say, "move kept photos to another folder" with
        // nothing unkept) leaves the folder empty. The app re-reads it afterwards and must get an
        // empty answer, not a failure it would have to guess the meaning of.
        let folder = six_jpegs();
        let sessions = tempfile::tempdir().unwrap();
        let session =
            Session::open_in(folder.path(), sessions.path(), Arc::new(NoListener)).unwrap();
        let id = |n: u32| crate::batch::photo_id(&format!("IMG_{n:04}.JPG"));
        for n in 1..=6 {
            session.set_rating(id(n), Rating::stars(5)).unwrap();
        }
        session.flush();
        for n in 1..=6 {
            fs::remove_file(folder.path().join(format!("IMG_{n:04}.JPG"))).unwrap();
            fs::remove_file(folder.path().join(format!("IMG_{n:04}.JPG.xmp"))).ok();
        }
        let scan = session.rescan().expect("an empty folder is a valid answer");
        assert!(scan.photos.is_empty());
        assert!(session.snapshot().photos.is_empty());
    }
}
