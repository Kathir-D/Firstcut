//! The model the surfaces bind to: the v1 `AppModel` port (docs/contracts/app-model.md).
//!
//! Ported from `App/Sources/Session/AppModel.swift` at M1, narrowed to what the vertical slice
//! needs — navigation, rating and undo rules are verbatim semantics; the pixel pipeline taps
//! (`updatePipelineFocus`, frame-latency spans, stand-ins) arrive with M2's decode work and are
//! not in this port. The UI never mutates engine state directly: it dispatches registry
//! commands (§4.3), which are the only callers of [`Model`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use firstcut_batching::batch::BatchId;
use firstcut_meta::meta::PhotoMeta;
use firstcut_types::{ColorLabel, Flag, PhotoId, Rating, RatingMode, Tier};

use crate::session::{Batch, Cursor, Session, SessionListener, SessionSnapshot};

/// What phase the app is in (app-model.md). The `loading` phase and its progress stream exist in
/// v1 because the first-photo fast path can have a folder ratable before the scan finishes; M2
/// brings that back with the decode priority window. Until then `open_folder` is a blocking call,
/// so `Loading` has nothing to say and is not a state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// No folder is open; the welcome state.
    Welcome,
    /// Photos, batches and ratings are all loaded; the user can act.
    Culling,
}

/// The batch-end arrow rule (app-model.md): stop, or keep swimming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowBehavior {
    Stop,
    ContinueIntoNextBatch,
}

/// The explicit-batch-navigation rule (‹ ›): always the first photo, or where the user last was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnteringBatchBehavior {
    FirstPhoto,
    LastViewed,
}

/// The General half of v1's settings the model reads. Defaults are v1's (SettingsModel.swift).
/// Persistence lands with the settings store (§4.3); the model only holds what was set.
#[derive(Clone, Copy, Debug)]
pub struct GeneralSettings {
    pub rating_mode: RatingMode,
    pub auto_advance: bool,
    pub arrow_behavior: ArrowBehavior,
    pub entering_behavior: EnteringBatchBehavior,
    pub reject_whole_batch: bool,
    /// Stars that make a keep in stars mode (4 by default; the app offers 4..=5).
    pub keep_stars: u8,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            rating_mode: RatingMode::Stars,
            auto_advance: false,
            arrow_behavior: ArrowBehavior::ContinueIntoNextBatch,
            entering_behavior: EnteringBatchBehavior::FirstPhoto,
            reject_whole_batch: false,
            keep_stars: 4,
        }
    }
}

/// One photo as the surfaces see it: capture-order metadata plus the live rating.
#[derive(Clone, Debug)]
pub struct PhotoVm {
    pub meta: PhotoMeta,
    pub rating: Rating,
}

/// One batch as the surfaces see it: the v1 `BatchVM` fields.
#[derive(Clone, Debug)]
pub struct BatchVm {
    pub id: BatchId,
    /// 0-based position in the shoot.
    pub index: u32,
    /// Photo ids in capture order.
    pub photo_ids: Vec<PhotoId>,
    pub visited: bool,
    /// True until visual signatures settle the batch's boundaries (M2 fills the sigs in).
    pub provisional: bool,
}

impl BatchVm {
    fn of(batch: &Batch, visited: bool) -> Self {
        Self {
            id: batch.id,
            index: batch.index,
            photo_ids: batch.photo_ids.clone(),
            visited,
            provisional: batch.provisional,
        }
    }

    pub fn count(&self) -> usize {
        self.photo_ids.len()
    }
}

/// Where in a batch to land when entering it (v1 `Selecting`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selecting {
    First,
    Last,
    LastViewed,
}

/// The model state the surfaces bind to. One at a time; the surfaces drive it through the
/// registry. Interior persistence belongs to the `Session`; everything here is the cursor and
/// view state.
pub struct Model {
    phase: Phase,
    session: Option<Session>,
    folder_name: String,
    folder: Option<PathBuf>,
    /// In capture order (app-model.md).
    photos: Vec<PhotoVm>,
    /// photo id → position in `photos`, so a rating update is O(1).
    photo_index: HashMap<PhotoId, usize>,
    batches: Vec<BatchVm>,
    current_batch_index: usize,
    current_photo_index: usize,
    settings: GeneralSettings,
    /// Set by `toggle_auto_advance`; `None` means the setting applies.
    auto_advance_override: Option<bool>,
    last_error: Option<String>,
    /// XMP writer failures arriving from the session between dispatches.
    pending_errors: Arc<Mutex<Vec<String>>>,
    /// Re-batches the settle pass pushed while the dispatch loop was not looking.
    pending_batches: Arc<Mutex<Vec<Vec<Batch>>>>,
}

impl Model {
    pub fn new() -> Self {
        Self {
            phase: Phase::Welcome,
            session: None,
            folder_name: String::new(),
            folder: None,
            photos: Vec::new(),
            photo_index: HashMap::new(),
            batches: Vec::new(),
            current_batch_index: 0,
            current_photo_index: 0,
            settings: GeneralSettings::default(),
            auto_advance_override: None,
            last_error: None,
            pending_errors: Arc::new(Mutex::new(Vec::new())),
            pending_batches: Arc::new(Mutex::new(Vec::new())),
        }
    }

    // ---- state the surfaces read -----------------------------------------------------------

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn folder_name(&self) -> &str {
        &self.folder_name
    }

    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    pub fn photos(&self) -> &[PhotoVm] {
        &self.photos
    }

    pub fn batches(&self) -> &[BatchVm] {
        &self.batches
    }

    pub fn current_batch_index(&self) -> usize {
        self.current_batch_index
    }

    pub fn current_photo_index(&self) -> usize {
        self.current_photo_index
    }

    pub fn settings(&self) -> &GeneralSettings {
        &self.settings
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Photos of a batch in capture order (app-model.md `photos(inBatch:)`).
    pub fn photos_in_batch(&self, index: usize) -> Vec<&PhotoVm> {
        match self.batches.get(index) {
            Some(batch) => batch.photo_ids.iter().filter_map(|id| self.photo(*id)).collect(),
            None => Vec::new(),
        }
    }

    pub fn current_batch(&self) -> Option<&BatchVm> {
        self.batches.get(self.current_batch_index)
    }

    /// The photo the cursor sits on; `None` outside every batch (welcome phase, empty shoot).
    pub fn current_photo(&self) -> Option<&PhotoVm> {
        let batch = self.current_batch()?;
        self.photo_before(batch)
    }

    /// The image the loupe shows next (M2's priority target).
    pub fn current_photo_id(&self) -> Option<PhotoId> {
        self.current_photo().map(|photo| photo.meta.id)
    }

    fn photo_before(&self, batch: &BatchVm) -> Option<&PhotoVm> {
        let id = *batch.photo_ids.get(self.current_photo_index)?;
        self.photo(id)
    }

    pub fn photo(&self, id: PhotoId) -> Option<&PhotoVm> {
        let index = self.photo_index.get(&id)?;
        self.photos.get(*index)
    }

    /// `canAct` in v1: only in the culling phase, and never mid-swap. M2 adds the first-photo
    /// fast path's provisional state; until then phase == culling is the whole rule.
    pub fn can_act(&self) -> bool {
        self.phase == Phase::Culling
    }

    /// Effective auto-advance: the on-the-fly toggle, then the setting.
    pub fn auto_advance(&self) -> bool {
        self.auto_advance_override.unwrap_or(self.settings.auto_advance)
    }

    /// Tier counts over the shoot, computed on read (v1 maintained counters; the slice recomputes).
    pub fn counts_by_tier(&self) -> HashMap<Tier, usize> {
        let mut counts = HashMap::new();
        for photo in self.photos() {
            let tier = photo.rating.tier_at(self.settings.rating_mode, self.settings.keep_stars);
            *counts.entry(tier).or_insert(0) += 1;
        }
        counts
    }

    /// Everything Finish considers reviewed.
    pub fn rated_count(&self) -> usize {
        self.photos().iter().filter(|p| is_reviewed(&p.rating)).count()
    }

    // ---- open ------------------------------------------------------------------------------

    /// Opens a folder, then restores the state of the shoot from the database. On failure the
    /// model stays Welcome with `last_error` set (app-model.md); on success it drops the
    /// previous session and lands in Culling, seeded from the restored cursor and keep stars.
    pub fn open_folder(&mut self, folder: &Path) -> Result<(), String> {
        let listener = SessionForwarder {
            errors: self.pending_errors.clone(),
            batches: self.pending_batches.clone(),
        };
        let session = Session::open(folder, Arc::new(listener)).map_err(|err| {
            self.last_error = Some(err.to_string());
            self.phase = Phase::Welcome;
            err.to_string()
        })?;
        self.settings.keep_stars = clamp_keep_stars(session.keep_stars());
        self.folder = Some(folder.to_path_buf());
        self.folder_name = folder
            .file_name()
            .map_or_else(|| folder.to_string_lossy().to_string(), |name| name.to_string_lossy().to_string());
        self.load_session(session);
        self.drain_session_messages();
        Ok(())
    }

    /// Seeds the model from a session (the split point `bench` also uses: scan → model).
    pub fn load_session(&mut self, session: Session) {
        let snapshot = session.snapshot();
        self.photo_index = snapshot
            .photos
            .iter()
            .enumerate()
            .map(|(position, meta)| (meta.id, position))
            .collect();
        self.photos = snapshot
            .photos
            .into_iter()
            .map(|meta| PhotoVm {
                rating: snapshot.ratings.get(&meta.id.0).copied().unwrap_or_default(),
                meta,
            })
            .collect();
        self.batches = snapshot
            .batches
            .iter()
            .map(|batch| BatchVm::of(batch, snapshot.visited.contains(&batch.id.0)))
            .collect();
        self.current_batch_index = 0;
        self.current_photo_index = 0;
        // Restore the cursor the database kept: the batch it points into, if re-batching has not
        // eaten it, and the photo it pointed at within that batch.
        if let Some(cursor) = snapshot.cursor {
            if let Some(position) = self.batches.iter().position(|b| b.id == cursor.batch) {
                self.current_batch_index = position;
                if let Some(within) = self
                    .batches
                    .get(position)
                    .and_then(|batch| batch_position_of(batch, cursor.photo))
                {
                    self.current_photo_index = within;
                }
            }
        }
        self.session = Some(session);
        self.phase = Phase::Culling;
        self.last_error = None;
    }

    // ---- navigation ------------------------------------------------------------------------

    /// A cursor moves only through these, so "only photos in the current batch can be rated" and
    /// the visited/cursor bookkeeping hold no matter who drives (UI, CLI, remote).
    pub fn move_photo(&mut self, by: i32) {
        if !self.can_act() {
            return;
        }
        let Some(batch) = self.current_batch().cloned() else { return };
        if batch.count() == 0 {
            return;
        }
        let target = self.current_photo_index as i64 + by as i64;
        if target >= 0 && (target as usize) < batch.count() {
            self.set_current_photo(target as usize);
            return;
        }
        if self.settings.arrow_behavior == ArrowBehavior::ContinueIntoNextBatch {
            // Leaving forward lands on the next batch's first photo, backward on the previous
            // batch's last — EnteringBatchBehavior is about explicit batch navigation, not
            // rolling over an edge (app-model.md `movePhoto`).
            if by > 0 && self.move_batch_selecting(1, Selecting::First) {
                return;
            }
            if by < 0 && self.move_batch_selecting(-1, Selecting::Last) {
                return;
            }
        }
    }

    pub fn move_batch(&mut self, by: i32) {
        let selecting = match self.settings.entering_behavior {
            EnteringBatchBehavior::LastViewed => Selecting::LastViewed,
            EnteringBatchBehavior::FirstPhoto => Selecting::First,
        };
        self.move_batch_selecting(by, selecting);
    }

    /// The ‹ › of it: forward goes to the next batch's first frame, back to the previous batch's
    /// last — whichever way the user entered the batch they are leaving. The
    /// `enteringBatchBehavior` setting does not apply (app-model.md `jumpBatch`).
    pub fn jump_batch(&mut self, forward: bool) {
        if forward {
            self.move_batch_selecting(1, Selecting::First);
        } else {
            self.move_batch_selecting(-1, Selecting::Last);
        }
    }

    /// `index` is within the current batch.
    pub fn go_to_photo(&mut self, index: usize) {
        if !self.can_act() {
            return;
        }
        self.set_current_photo(index);
    }

    /// Steps past the current photo whatever the arrows-at-edges setting says. Auto-advance and
    /// ⇧1…⇧5 both exist so you never have to stop mid-burst, and every burst ends at the edge, so
    /// crossing into the next batch is the useful behaviour (app-model.md `advance`).
    pub fn advance(&mut self) {
        let Some(batch) = self.current_batch().cloned() else { return };
        if self.current_photo_index + 1 < batch.count() {
            self.set_current_photo(self.current_photo_index + 1);
            return;
        }
        self.move_batch_selecting(1, Selecting::First);
    }

    fn set_current_photo(&mut self, index: usize) {
        let Some(batch) = self.current_batch().cloned() else { return };
        if index >= batch.count() {
            return;
        }
        self.current_photo_index = index;
        self.mark_current_batch_visited();
        self.push_cursor(&batch);
    }

    fn move_batch_selecting(&mut self, by: i32, selecting: Selecting) -> bool {
        let target = self.current_batch_index as i64 + by as i64;
        if target < 0 || target as usize >= self.batches.len() {
            return false;
        }
        let position = target as usize;
        let Some(batch) = self.batches.get(position).cloned() else { return false };
        self.current_batch_index = position;
        self.current_photo_index = match selecting {
            Selecting::First => 0,
            Selecting::Last => batch.count().saturating_sub(1),
            Selecting::LastViewed => self.position_of_last_viewed(&batch),
        };
        self.mark_current_batch_visited();
        self.push_cursor(&batch);
        // Leaving a batch flushes the debounced XMP queue (task.md §6.3; session-api.md).
        if let Some(session) = self.session.as_ref() {
            session.flush();
        }
        true
    }

    fn position_of_last_viewed(&self, batch: &BatchVm) -> usize {
        let Some(session) = self.session.as_ref() else { return 0 };
        let Some(photo) = session
            .snapshot()
            .last_photo_in_batch
            .get(&batch.id.0)
            .copied()
            .and_then(|id| self.photo(PhotoId(id)))
        else {
            return 0;
        };
        batch_position_of(batch, photo.meta.id).unwrap_or(0)
    }

    fn mark_current_batch_visited(&mut self) {
        let Some(session) = self.session.as_ref() else { return };
        let Some(batch) = self.batches.get_mut(self.current_batch_index) else { return };
        if !batch.visited {
            batch.visited = true;
            let _ = session.mark_visited(batch.id);
        }
    }

    fn push_cursor(&mut self, batch: &BatchVm) {
        let Some(session) = self.session.as_ref() else { return };
        if let Some(photo) = self.current_photo() {
            let _ = session.set_cursor(Cursor { batch: batch.id, photo: photo.meta.id });
        }
    }

    // ---- rating ----------------------------------------------------------------------------

    /// Every way of rating a photo funnels through `rate_photo`. The session owns persistence
    /// (DB now, XMP debounced ≤ 1 s); undo works because `set_rating` hands back the before/after
    /// pair, recorded before the local state moves (app-model.md `rateCurrent`).
    pub fn rate_current(&mut self, apply: impl FnOnce(Rating) -> Rating) -> Option<Rating> {
        let photo_id = self.current_photo_id()?;
        self.rate_photo(photo_id, apply, true)
    }

    /// The current photo, then honour auto-advance.
    fn rate_photo(&mut self, photo_id: PhotoId, apply: impl FnOnce(Rating) -> Rating, advance_after: bool) -> Option<Rating> {
        if !self.can_act() {
            return None;
        }
        let photo = self.photo(photo_id)?;
        let updated = apply(photo.rating);
        if updated == photo.rating {
            return None;
        }
        // The session owns persistence; a writer failure surfaces as `last_error`, the local
        // rating still moves — the same listener backstop v1 had.
        if let Some(session) = self.session.as_ref() {
            if let Err(err) = session.set_rating(photo_id, updated) {
                self.last_error = Some(err.to_string());
            }
        }
        self.apply_rating_locally(photo_id, updated);
        if advance_after && self.auto_advance() {
            self.advance();
        }
        Some(updated)
    }

    /// A change that is already persisted gets only mirrored locally (undo, reject batch).
    fn apply_rating_locally(&mut self, photo_id: PhotoId, rating: Rating) {
        if let Some(photo) = self.photos.get_mut(*self.photo_index.get(&photo_id)? as usize) {
            photo.rating = rating;
        }
    }

    /// Switching mode mid-session is allowed and loses nothing: stars and keep are kept
    /// consistent, and the visible tier is derived at read time (todo.md §6).
    pub fn set_rating_mode(&mut self, mode: RatingMode) {
        self.settings.rating_mode = mode;
    }

    pub fn set_auto_advance(&mut self, on: bool) {
        self.settings.auto_advance = on;
        self.auto_advance_override = None;
    }

    pub fn toggle_auto_advance(&mut self) {
        self.auto_advance_override = Some(!self.auto_advance());
    }

    pub fn set_keep_stars(&mut self, stars: u8) {
        // 4..=5 are the two meanings the app offers (`clamp_keep_stars`).
        self.settings.keep_stars = clamp_keep_stars(stars);
        if let Some(session) = self.session.as_ref() {
            session.set_keep_stars(self.settings.keep_stars);
        }
    }

    // ---- rating primitives (the one place a field changes) ---------------------------------

    pub fn rate_stars(&mut self, stars: u8) -> Option<Rating> {
        let stars = stars.min(5);
        self.rate_current(|rating| Rating {
            stars,
            ..rating
        })
    }

    pub fn rate_stars_and_advance(&mut self, stars: u8) -> Option<Rating> {
        // ⇧1…⇧5 always advances, auto-advance or not (app-model.md `setStarsAndAdvance`): rate
        // without the auto step so the cursor moves once, then advance.
        let stars = stars.clamp(1, 5);
        let photo_id = self.current_photo_id()?;
        let updated = self.rate_photo(photo_id, |rating| Rating { stars, ..rating }, false);
        if updated.is_some() {
            self.advance();
        }
        updated
    }

    pub fn toggle_pick_flag(&mut self) -> Option<Rating> {
        self.rate_current(|rating| Rating {
            flag: if rating.flag == Flag::Pick { Flag::None } else { Flag::Pick },
            ..rating
        })
    }

    pub fn set_keep(&mut self) -> Option<Rating> {
        // The two buttons, not a toggle: idempotent. A "Keep" button that toggled would make a
        // second click mean "not keep", which is not what the button says (app-model.md).
        self.rate_current(|rating| Rating { keep: true, ..rating })
    }

    pub fn set_not_keep(&mut self) -> Option<Rating> {
        self.rate_current(|rating| Rating { keep: false, ..rating })
    }

    pub fn toggle_keep(&mut self) -> Option<Rating> {
        self.rate_current(|rating| Rating {
            keep: !rating.keep,
            ..rating
        })
    }

    pub fn reject_flag(&mut self) -> Option<Rating> {
        self.rate_current(|rating| Rating {
            flag: Flag::Reject,
            ..rating
        })
    }

    pub fn unflag(&mut self) -> Option<Rating> {
        self.rate_current(|rating| Rating { flag: Flag::None, ..rating })
    }

    pub fn toggle_flag(&mut self) -> Option<Rating> {
        self.rate_current(|rating| Rating {
            flag: if rating.flag == Flag::None { Flag::Pick } else { Flag::None },
            ..rating
        })
    }

    pub fn set_label(&mut self, label: Option<ColorLabel>) -> Option<Rating> {
        self.rate_current(|rating| Rating { label, ..rating })
    }

    /// Settings → General → "Reject Whole Batch", off by default: X for every frame of the
    /// current batch, as one step that one ⌘Z takes back, then on to the next batch. It only
    /// flags: nothing leaves the disk until Finish (app-model.md `rejectCurrentBatch`).
    pub fn reject_current_batch(&mut self) -> bool {
        if !self.settings.reject_whole_batch || !self.can_act() {
            return false;
        }
        let Some(batch) = self.current_batch().cloned() else { return false };
        let Some(session) = self.session.as_ref() else { return false };
        let updates: Vec<(PhotoId, Rating)> = batch
            .photo_ids
            .iter()
            .filter_map(|id| self.photo(*id).map(|photo| (*id, photo.rating)))
            .map(|(id, rating)| (id, Rating { flag: Flag::Reject, ..rating }))
            .filter(|(id, updated)| self.photo(*id).map_or(false, |photo| photo.rating != *updated))
            .collect();
        if !updates.is_empty() {
            // `set_ratings` records the batch as one undo step (session-api.md).
            if let Ok(changes) = session.set_ratings(&updates) {
                for change in changes {
                    self.apply_rating_locally(change.photo, change.after);
                }
            }
        }
        self.move_batch(1);
        true
    }

    // ---- undo ------------------------------------------------------------------------------

    /// One undo is one *step*: a single rating, or every frame of a whole-batch reject. Undo
    /// navigates to the batch the step was made in, then reverts it (app-model.md `undo`), so
    /// the only-rate-here rule still holds visibly. A step lands on the last photo it put back.
    pub fn undo(&mut self) -> bool {
        self.apply_undo(UndoSide::Before)
    }

    pub fn redo(&mut self) -> bool {
        self.apply_undo(UndoSide::After)
    }

    fn apply_undo(&mut self, side: UndoSide) -> bool {
        if !self.can_act() {
            return false;
        }
        let Some(session) = self.session.as_ref() else { return false };
        let changes = match side {
            UndoSide::Before => session.undo_step(),
            UndoSide::After => session.redo_step(),
        };
        if changes.is_empty() {
            return false;
        }
        for change in &changes {
            self.navigate_to(change.photo, change.batch);
            self.apply_rating_locally(change.photo, match side {
                UndoSide::Before => change.before,
                UndoSide::After => change.after,
            });
        }
        true
    }

    /// Moves the cursor to a change's batch and photo if both still exist (a change whose batch
    /// was re-batched away is still reverted; the cursor simply stays put).
    fn navigate_to(&mut self, photo: PhotoId, batch: BatchId) {
        let Some(position) = self.batches.iter().position(|b| b.id == batch) else { return };
        let Some(batch_vm) = self.batches.get(position).cloned() else { return };
        let Some(within) = batch_position_of(&batch_vm, photo) else { return };
        let batch_changed = position != self.current_batch_index;
        self.current_batch_index = position;
        self.current_photo_index = within;
        if batch_changed {
            self.mark_current_batch_visited();
            self.push_cursor(&batch_vm);
            if let Some(session) = self.session.as_ref() {
                session.flush();
            }
        }
    }

    // ---- session messages ------------------------------------------------------------------

    /// Pulls what the session queued between dispatches: re-batches from the settle pass, and
    /// XMP writer failures (→ `last_error`).
    pub fn drain_session_messages(&mut self) {
        if let Ok(mut errors) = self.pending_errors.lock() {
            if let Some(latest) = errors.pop() {
                self.last_error = Some(latest);
            }
            errors.clear();
        }
        let fresh: Vec<Vec<Batch>> = self
            .pending_batches
            .lock()
            .map(|mut queue| queue.drain(..).flatten().collect())
            .unwrap_or_default();
        let Some(latest) = fresh.last() else { return };
        // The latest re-batch is authoritative; visited is per-batch and a batch the user was in
        // is frozen (only unvisited ones can differ).
        self.batches = latest
            .iter()
            .map(|batch| {
                let visited = self.batches.iter().find(|old| old.id == batch.id).map_or(false, |old| old.visited);
                BatchVm::of(batch, visited)
            })
            .collect();
        // The current photo may have moved batch; follow it by id, or reset to the first photo.
        match self.current_photo_id() {
            Some(photo) => {
                let position = self.batches.iter().position(|batch| batch.photo_ids.contains(&photo));
                self.current_batch_index = position.unwrap_or(0);
                self.current_photo_index = position
                    .and_then(|p| self.batches.get(p))
                    .and_then(|batch| batch_position_of(batch, photo))
                    .unwrap_or(0);
                if position.is_none() {
                    self.current_photo_index = 0;
                }
            }
            None => {
                self.current_batch_index = 0;
                self.current_photo_index = 0;
            }
        }
        self.current_batch_index = self.current_batch_index.min(self.batches.len().saturating_sub(1));
    }

    pub fn session_root(&self) -> Option<&Session> {
        self.session.as_ref()
    }
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
enum UndoSide {
    Before,
    After,
}

fn clamp_keep_stars(stars: u8) -> u8 {
    stars.clamp(4, 5)
}

fn batch_position_of(batch: &BatchVm, photo: PhotoId) -> Option<usize> {
    batch.photo_ids.iter().position(|id| *id == photo)
}

/// `isReviewed`: every field that says "this photo has been looked at" (v1 `RatingRules`).
fn is_reviewed(rating: &Rating) -> bool {
    rating.stars > 0 || rating.flag != Flag::None || rating.label.is_some() || rating.keep
}

/// The listener that bridges session callbacks into the model's drain queue. v1's AppModel got
/// Swift-lane callbacks; the registry's dispatch loop drains this instead.
struct SessionForwarder {
    errors: Arc<Mutex<Vec<String>>>,
    batches: Arc<Mutex<Vec<Vec<Batch>>>>,
}

impl SessionListener for SessionForwarder {
    fn batches_changed(&self, batches: Vec<Batch>) {
        if let Ok(mut queue) = self.batches.lock() {
            queue.push(batches);
        }
    }

    fn files_changed(&self) {
        // The reconcile pass for FSEvents lands with M2; until then a changed folder shows up on
        // the next open, exactly as an app restart would report it.
    }

    fn xmp_error(&self, _photo: PhotoId, message: String) {
        if let Ok(mut queue) = self.errors.lock() {
            queue.push(message);
        }
    }
}
