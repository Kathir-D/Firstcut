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

use firstcut_batching::batch::visual::VisualSig;
use firstcut_batching::batch::{Batch, BatchId, batch_id};
use firstcut_catalog::recents::RecentShoot;
use firstcut_meta::meta::PhotoMeta;
use firstcut_types::{ColorLabel, Flag, PhotoId, Rating, RatingMode, Tier};

use crate::keymap::{KeyChord, Keymap, KeymapStore};
use crate::session::{Cursor, Session, SessionListener, SessionSnapshot};

/// What phase the app is in (app-model.md). `Loading` is v1's first-photo fast path: the
/// placeholder header has been read and its preview can paint while the full scan runs in the
/// background. Actions wait until the real session is in (`can_act` is false), because ratings
/// and batch navigation need the catalog the scan builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// No folder is open; the welcome state.
    Welcome,
    /// One placeholder photograph is on screen; the full scan is still running.
    Loading,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneralSettings {
    pub rating_mode: RatingMode,
    pub auto_advance: bool,
    pub arrow_behavior: ArrowBehavior,
    pub entering_behavior: EnteringBatchBehavior,
    pub reject_whole_batch: bool,
    /// Stars that make a keep in stars mode (4 by default; the app offers 4..=5).
    pub keep_stars: u8,
    /// What Finish does with the photos that were not kept (v1's `finishUnkept`).
    pub finish_unkept: firstcut_fileops::UnkeptAction,
    /// What Finish does with the kept photos (v1's `finishKept`).
    pub finish_kept: firstcut_fileops::KeptAction,
    /// Ask before the Finish sheet opens at all (v1's `confirmBeforeFinish`).
    pub confirm_before_finish: bool,
    /// Require the typed word when a Finish run deletes permanently (v1's `confirmPermanentDelete`).
    pub confirm_permanent_delete: bool,
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
            // v1's `UnkeptAction.default`: not-kept photos move into `_Not kept`.
            finish_unkept: firstcut_fileops::UnkeptAction::MoveToSubfolder("_Not kept".to_string()),
            finish_kept: firstcut_fileops::KeptAction::None,
            confirm_before_finish: true,
            confirm_permanent_delete: true,
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
        Self { id: batch.id, index: batch.index, photo_ids: batch.photo_ids.clone(), visited, provisional: batch.provisional }
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
    /// Present only when a surface asked for persistence; the model then seeds its General
    /// settings from the loaded file and writes back on every general setter.
    settings_store: Option<crate::settings::SettingsStore>,
    /// Bumped whenever the photo catalog changes (open, rescan). The decode pipeline rebuilds
    /// its catalog on a change and keeps its cache otherwise, which is what stops a file landing
    /// in the folder from blanking the strip.
    catalog_revision: u64,
    /// Thumbnails that finished while the full scan was still running: there is no `Session` to
    /// submit them to yet, so they wait here and are handed over by [`Model::load_session`].
    pending_signatures: Vec<(PhotoId, VisualSig)>,
    /// Recent shoots for the Welcome screen, read from the session databases.
    recents: Vec<RecentShoot>,
    /// Where those databases live. The app attaches it at construction; `open_folder_in` records
    /// the directory it was handed, so the CLI and tests get the list without extra wiring.
    sessions_dir: Option<PathBuf>,
    /// The keyboard map the window routes through (defaults until a store attaches).
    keymap: Keymap,
    /// Defaults + the user's own chords, and the file that holds them.
    keymap_store: Option<KeymapStore>,
    /// The Finish sheet's stage; `Hidden` whenever the culling screen is the surface.
    finish: crate::finish::FinishStage,
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
            settings_store: None,
            catalog_revision: 0,
            pending_signatures: Vec::new(),
            recents: Vec::new(),
            sessions_dir: None,
            keymap: Keymap::defaults(),
            keymap_store: None,
            finish: crate::finish::FinishStage::Hidden,
        }
    }

    /// Attaches the user's keymap file and makes its layered result the map the window routes.
    pub fn attach_keymap_store(&mut self, store: KeymapStore) {
        self.keymap = store.effective();
        self.keymap_store = Some(store);
    }

    /// The effective map (defaults with the user's overrides on top).
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// The user's `keymap.json`, when one was attached.
    pub fn keymap_path(&self) -> Option<&Path> {
        self.keymap_store.as_ref().map(KeymapStore::path)
    }

    /// Binds `chord` to a command and saves the user's map. The command's existing modes carry
    /// over, so rebinding Stars-mode P does not make it fire in Keep mode. A command's old chords
    /// are replaced, which is v1's "write the command's complete chord set" rule.
    pub fn rebind_keymap(&mut self, command: &str, argument: Option<i64>, chord: &KeyChord) -> Result<(), String> {
        let mode = self.keymap.modes_for(command, argument).and_then(|modes| if modes.len() == 1 { modes.first().copied() } else { None });
        if let Some(store) = self.keymap_store.as_mut() {
            let mut overrides = store.overrides().clone();
            overrides.remove_command(command, argument);
            overrides.bind(chord, command, argument, mode);
            if let Err(err) = store.set_overrides(overrides) {
                let message = err.to_string();
                self.last_error = Some(message.clone());
                return Err(message);
            }
            self.keymap = store.effective();
        } else {
            self.keymap.remove_command(command, argument);
            self.keymap.bind(chord, command, argument, mode);
        }
        Ok(())
    }

    /// Throws the user's overrides away and returns to the shipped map.
    pub fn reset_keymap(&mut self) -> Result<(), String> {
        if let Some(store) = self.keymap_store.as_mut() {
            if let Err(err) = store.reset_to_defaults() {
                let message = err.to_string();
                self.last_error = Some(message.clone());
                return Err(message);
            }
            self.keymap = store.effective();
        } else {
            self.keymap = Keymap::defaults();
        }
        Ok(())
    }

    /// Replaces the user's map with an imported file (v1's `importKeymap`).
    pub fn import_keymap(&mut self, path: &Path) -> Result<(), String> {
        let outcome = if let Some(store) = self.keymap_store.as_mut() {
            match store.import(path) {
                Ok(()) => {
                    self.keymap = store.effective();
                    Ok(())
                }
                Err(err) => Err(err.to_string()),
            }
        } else {
            match std::fs::read_to_string(path) {
                Ok(text) => Keymap::from_json(&text).map(|map| self.keymap = map),
                Err(err) => Err(format!("reading {}: {err}", path.display())),
            }
        };
        if let Err(message) = &outcome {
            self.last_error = Some(message.clone());
        }
        outcome
    }

    /// Writes the effective map (defaults + overrides) to `path` (v1's `exportKeymap`).
    pub fn export_keymap(&mut self, path: &Path) -> Result<(), String> {
        let outcome = if let Some(store) = self.keymap_store.as_ref() {
            store.export(path).map_err(|err| err.to_string())
        } else {
            self.keymap
                .to_json()
                .map_err(|err| format!("writing {}: {err}", path.display()))
                .and_then(|json| std::fs::write(path, json).map_err(|err| format!("writing {}: {err}", path.display())))
        };
        if let Err(message) = &outcome {
            self.last_error = Some(message.clone());
        }
        outcome
    }

    /// Points the Welcome screen at the sessions directory and loads its recent shoots. Safe to
    /// call before any shoot is open; a missing directory just means an empty list.
    pub fn attach_sessions_dir(&mut self, sessions_dir: &Path) {
        self.sessions_dir = Some(sessions_dir.to_path_buf());
        self.refresh_recents();
    }

    /// The Welcome screen's list, newest first. `firstcut_catalog::recents::list` owns the rules
    /// (a vanished folder is not offered, an unreadable database is skipped).
    pub fn recents(&self) -> &[RecentShoot] {
        &self.recents
    }

    fn refresh_recents(&mut self) {
        if let Some(dir) = self.sessions_dir.as_deref() {
            self.recents = firstcut_catalog::recents::list(dir, 8);
        }
    }

    /// Hands the model a settings store (v1's `AppModel` construction order: load, then seed the
    /// General half from the file). A session already open — the UI's open-then-attach order —
    /// gets its keep stars re-synced the way `set_keep_stars` does it.
    pub fn attach_settings_store(&mut self, store: crate::settings::SettingsStore) {
        let general = store.general();
        self.settings = general;
        self.settings_store = Some(store);
        if let Some(session) = self.session.as_ref() {
            session.set_keep_stars(self.settings.keep_stars);
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

    /// Settings → Viewer → "Develop the sensor data, not the embedded preview" (T4).
    pub fn exact_raw(&self) -> bool {
        self.settings_store.as_ref().is_some_and(|store| store.current().viewer.exact_raw)
    }

    /// Settings → Viewer → zoom lock: keep the loupe's pixel size while stepping through photos.
    pub fn zoom_lock(&self) -> bool {
        self.settings_store.as_ref().is_some_and(|store| store.current().viewer.zoom_lock)
    }

    /// Settings → Performance, for the decode bank's budgets and worker count.
    pub fn performance_settings(&self) -> crate::settings::PerformanceJson {
        self.settings_store.as_ref().map(|store| store.current().performance.clone()).unwrap_or_default()
    }

    /// The viewer half of the settings: overlay defaults and their clipping thresholds. The app
    /// and the UI read them without reaching into the settings store.
    pub fn viewer_settings(&self) -> crate::settings::ViewerJson {
        self.settings_store.as_ref().map(|store| store.current().viewer.clone()).unwrap_or_default()
    }

    /// Offers a thumbnail decode's pixels to the batcher. The signature is computed from the same
    /// buffer the filmstrip is about to show — the pipeline never decodes a photo twice for it.
    /// Batches that move are queued on the session listener and arrive through
    /// [`Model::drain_session_messages`].
    ///
    /// During the first-photo fast path there is no session yet; the signature waits in
    /// `pending_signatures` and is submitted the moment the scan finishes, so the placeholder
    /// frame still contributes to its batch's boundary.
    pub fn submit_thumbnail_pixels(&mut self, id: PhotoId, rgba: &[u8], width: u32, height: u32) {
        let signature = firstcut_batching::batch::visual_sig(rgba, width, height);
        if let Some(session) = self.session.as_ref() {
            session.submit_visual_sigs(vec![(id, signature)]);
        } else {
            self.pending_signatures.push((id, signature));
        }
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// v1's `dismissError()`: the alert stays until the user says so.
    pub fn dismiss_error(&mut self) {
        self.last_error = None;
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

    /// The decode priority window (docs/v2-plan.md §4.2): the current batch, then the next
    /// batch, then the previous one — nearest-first with next before previous. Only the first
    /// three are guaranteed; the pipeline may prefetch more.
    pub fn focus_windows(&self) -> Vec<Vec<PhotoId>> {
        let mut windows = Vec::new();
        for index in [Some(self.current_batch_index), self.current_batch_index.checked_add(1), self.current_batch_index.checked_sub(1)] {
            let Some(index) = index else { continue };
            if let Some(batch) = self.batches.get(index)
                && !batch.photo_ids.is_empty()
            {
                windows.push(batch.photo_ids.clone());
            }
        }
        windows
    }

    /// Changes when the photo catalog does; the app's decode bank watches it.
    pub fn catalog_revision(&self) -> u64 {
        self.catalog_revision
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

    // ---- finish cull (todo.md §9.7) ---------------------------------------------------------

    /// The sheet's current stage. The UI draws this and dispatches `finish.*` commands; it never
    /// advances the stage itself (docs/v2-plan.md §4.3).
    pub fn finish_stage(&self) -> &crate::finish::FinishStage {
        &self.finish
    }

    /// Totals for the summary screen, from the same tier rule the filmstrip and HUD use.
    pub fn finish_summary(&self) -> crate::finish::FinishSummary {
        crate::finish::FinishSummary {
            counts: self.counts_by_tier(),
            unvisited_batches: self.batches.iter().filter(|batch| !batch.visited).count(),
            batch_count: self.batches.len(),
            total_photos: self.photos.len(),
        }
    }

    /// Finish Cull: the confirm question or the summary, depending on the setting. Nothing is
    /// planned or touched here; this only opens the sheet.
    pub fn start_finish(&mut self) -> Result<(), String> {
        if !self.can_act() || self.photos.is_empty() || self.batches.is_empty() {
            return Err("cull.finish: the culling screen is not showing a shoot".to_string());
        }
        if self.finish.is_visible() {
            // A second press while the sheet is up must not throw away the options or dry run.
            return Ok(());
        }
        self.finish = if self.settings.confirm_before_finish {
            crate::finish::FinishStage::Confirm
        } else {
            crate::finish::FinishStage::Summary(self.finish_summary())
        };
        Ok(())
    }

    /// The answer to the confirm stage.
    pub fn confirm_finish(&mut self) -> Result<(), String> {
        if !matches!(self.finish, crate::finish::FinishStage::Confirm) {
            return Err("finish.confirm: the Finish sheet is not asking to confirm".to_string());
        }
        self.finish = crate::finish::FinishStage::Summary(self.finish_summary());
        Ok(())
    }

    /// Summary → options, seeded from Settings → General.
    pub fn show_finish_options(&mut self) -> Result<(), String> {
        let crate::finish::FinishStage::Summary(summary) = self.finish.clone() else {
            return Err("finish.options: the Finish sheet is not showing the summary".to_string());
        };
        let options = firstcut_fileops::FinishOptions {
            unkept: self.settings.finish_unkept.clone(),
            kept: self.settings.finish_kept.clone(),
            rating_mode: self.settings.rating_mode,
            keep_stars: self.settings.keep_stars,
        };
        self.finish = crate::finish::FinishStage::Options { summary, options };
        Ok(())
    }

    /// One options change. The rating mode and keep threshold are the session's and cannot be
    /// faked by a caller, exactly as v1's `updateFinishSettings` pinned `ratingMode`.
    pub fn set_finish_options(
        &mut self,
        unkept: Option<firstcut_fileops::UnkeptAction>,
        kept: Option<firstcut_fileops::KeptAction>,
    ) -> Result<(), String> {
        let crate::finish::FinishStage::Options { summary, mut options } = self.finish.clone() else {
            return Err("finish.unkept/finish.kept: the Finish sheet is not choosing options".to_string());
        };
        if let Some(unkept) = unkept {
            options.unkept = unkept;
        }
        if let Some(kept) = kept {
            options.kept = kept;
        }
        options.rating_mode = self.settings.rating_mode;
        options.keep_stars = self.settings.keep_stars;
        self.finish = crate::finish::FinishStage::Options { summary, options };
        Ok(())
    }

    /// The dry run: everything that would happen, before anything does. The typed-delete gate is
    /// decided here, where the plan, the options and the setting are all in hand.
    pub fn run_finish_dry_run(&mut self) -> Result<(), String> {
        let crate::finish::FinishStage::Options { summary, options } = self.finish.clone() else {
            return Err("finish.preview: the Finish sheet is not choosing options".to_string());
        };
        if let firstcut_fileops::UnkeptAction::MoveToSubfolder(folder) = &options.unkept
            && folder.trim().is_empty()
        {
            return Err("finish.preview: give the not-kept subfolder a name".to_string());
        }
        if crate::finish::kept_needs_path(&options.kept) {
            let path = match &options.kept {
                firstcut_fileops::KeptAction::CopyTo(path)
                | firstcut_fileops::KeptAction::MoveTo(path)
                | firstcut_fileops::KeptAction::SplitByTier(path)
                | firstcut_fileops::KeptAction::SplitByStars(path)
                | firstcut_fileops::KeptAction::WriteList(path) => path,
                firstcut_fileops::KeptAction::None => "",
            };
            if path.trim().is_empty() {
                return Err("finish.preview: give the kept destination a name".to_string());
            }
        }
        let Some(session) = self.session.as_ref() else {
            return Err("finish.preview: no shoot is open".to_string());
        };
        match session.plan_finish(&options) {
            Ok(plan) => {
                let requires_typed_confirmation = self.settings.confirm_permanent_delete && options.unkept.is_destructive() && !plan.is_empty();
                self.finish = crate::finish::FinishStage::DryRun { summary, options, plan, requires_typed_confirmation };
                Ok(())
            }
            Err(err) => {
                let message = err.to_string();
                self.last_error = Some(message.clone());
                self.finish = crate::finish::FinishStage::Failed { summary, message: message.clone() };
                Err(message)
            }
        }
    }

    /// Walks the dry run. `confirmation` is the sheet's typed word; it is checked here as well as
    /// in the view, because a command can arrive from the CLI or the control channel.
    pub fn execute_finish(&mut self, confirmation: Option<&str>) -> Result<(), String> {
        let crate::finish::FinishStage::DryRun { summary, plan, requires_typed_confirmation, .. } = self.finish.clone() else {
            return Err("finish.execute: the Finish sheet is not showing a dry run".to_string());
        };
        if requires_typed_confirmation && confirmation != Some(crate::finish::TYPED_CONFIRMATION) {
            return Err(format!("finish.execute: type {} to confirm a permanent delete", crate::finish::TYPED_CONFIRMATION));
        }
        let Some(session) = self.session.as_ref() else {
            return Err("finish.execute: no shoot is open".to_string());
        };
        let run = match session.execute_finish(&plan) {
            Ok(run) => run,
            Err(err) => {
                let message = err.to_string();
                self.last_error = Some(message.clone());
                self.finish = crate::finish::FinishStage::Failed { summary, message: message.clone() };
                return Err(message);
            }
        };
        let report =
            crate::finish::FinishReport { done: run.summary.done, failed: run.summary.failed, undoable: run.summary.undoable, was_undo: false };
        self.finish = crate::finish::FinishStage::Report { summary, report };
        // The files moved, so the catalog is re-read before the sheet's Done goes back to it.
        self.sync_from_session();
        Ok(())
    }

    /// Reverses the last Finish run. One press per run: the session consumes the log, so a second
    /// press would reverse an earlier run, which is why `was_undo` exists on the report.
    pub fn undo_finish(&mut self) -> Result<(), String> {
        let crate::finish::FinishStage::Report { summary, report } = self.finish.clone() else {
            return Err("finish.undo: the Finish sheet is not showing a report".to_string());
        };
        if report.was_undo || !report.undoable {
            return Err("finish.undo: this run cannot be undone".to_string());
        }
        let Some(session) = self.session.as_ref() else {
            return Err("finish.undo: no shoot is open".to_string());
        };
        let undone = match session.undo_finish() {
            Ok(undone) => undone,
            Err(err) => {
                let message = err.to_string();
                self.last_error = Some(message.clone());
                return Err(message);
            }
        };
        let report = crate::finish::FinishReport { done: undone.summary.done, failed: undone.summary.failed, undoable: false, was_undo: true };
        self.finish = crate::finish::FinishStage::Report { summary, report };
        self.sync_from_session();
        Ok(())
    }

    /// One free step back: dry run → options → summary. Nothing has touched the disk in any of
    /// these stages.
    pub fn back_finish(&mut self) -> Result<(), String> {
        match self.finish.clone() {
            crate::finish::FinishStage::Options { summary, .. } => {
                self.finish = crate::finish::FinishStage::Summary(summary);
                Ok(())
            }
            crate::finish::FinishStage::DryRun { summary, options, .. } => {
                self.finish = crate::finish::FinishStage::Options { summary, options };
                Ok(())
            }
            _ => Err("finish.back: there is no previous Finish screen".to_string()),
        }
    }

    /// Closes the sheet. A Finish that moved every photo out leaves nothing to cull, so the model
    /// goes back to Welcome instead of an empty viewer (v1's `cancelFinish`).
    pub fn cancel_finish(&mut self) {
        self.finish = crate::finish::FinishStage::Hidden;
        if self.phase == Phase::Culling && self.photos.is_empty() {
            self.close_session();
            self.session = None;
            self.phase = Phase::Welcome;
            self.folder = None;
            self.folder_name.clear();
            self.photos.clear();
            self.photo_index.clear();
            self.batches.clear();
            self.current_batch_index = 0;
            self.current_photo_index = 0;
            self.refresh_recents();
        }
    }

    /// Re-reads the open session's snapshot into the model without another disk pass. The Finish
    /// run already rescanned the folder after moving files; this is what makes the strip agree.
    fn sync_from_session(&mut self) {
        let Some(snapshot) = self.session.as_ref().map(Session::snapshot) else {
            return;
        };
        self.adopt_snapshot(snapshot);
    }

    // ---- open ------------------------------------------------------------------------------

    /// Opens a folder, then restores the state of the shoot from the database. On failure the
    /// model stays Welcome with `last_error` set (app-model.md); on success it drops the
    /// previous session and lands in Culling, seeded from the restored cursor and keep stars.
    pub fn open_folder(&mut self, folder: &Path) -> Result<(), String> {
        // The standard location (firstcut_catalog::sessions_dir) is inside; tests and `bench`
        // need an isolated sessions directory, hence the `open_folder_in` seam.
        let sessions = firstcut_catalog::sessions_dir().map_err(|err| {
            // A failed open reports the same way no matter which leg failed: the phase stays
            // welcome and `last_error` is set (app-model.md). A `HOME` that is gone is just as
            // much "the folder did not open" as a missing shoot folder is.
            self.last_error = Some(err.to_string());
            err.to_string()
        })?;
        self.open_folder_in(folder, &sessions)
    }

    /// `open_folder` with the sessions directory chosen (tests, `bench --folder`).
    pub fn open_folder_in(&mut self, folder: &Path, sessions_dir: &Path) -> Result<(), String> {
        let listener = SessionForwarder { errors: self.pending_errors.clone(), batches: self.pending_batches.clone() };
        let session = Session::open_in(folder, sessions_dir, Arc::new(listener)).map_err(|err| {
            self.last_error = Some(err.to_string());
            self.phase = Phase::Welcome;
            err.to_string()
        })?;
        self.settings.keep_stars = clamp_keep_stars(session.keep_stars());
        self.folder = Some(folder.to_path_buf());
        self.folder_name = folder.file_name().map_or_else(|| folder.to_string_lossy().to_string(), |name| name.to_string_lossy().to_string());
        self.sessions_dir = Some(sessions_dir.to_path_buf());
        self.load_session(session);
        self.drain_session_messages();
        self.refresh_recents();
        Ok(())
    }

    /// Starts the first-photo fast path: reads the placeholder header (one file), puts its
    /// preview on screen, and leaves the full scan to the caller. `finish_open` installs the real
    /// session; `fail_open` returns to Welcome with the error.
    ///
    /// A folder with no photograph returns the error before the previous session is touched.
    pub fn begin_open(&mut self, folder: &Path) -> Result<(), String> {
        let name = firstcut_meta::meta::first_photo_name(folder).ok_or_else(|| format!("no photographs in {}", folder.display()))?;
        let meta = firstcut_meta::meta::read_photo(folder, &name)
            .ok_or_else(|| format!("could not read the first photograph ({name}) in {}", folder.display()))?;
        // Commit only after the placeholder is in hand: a bad folder must not close a good shoot.
        self.close_session();
        self.session = None;
        let id = meta.id;
        self.folder = Some(folder.to_path_buf());
        self.folder_name = folder.file_name().map_or_else(|| folder.to_string_lossy().to_string(), |name| name.to_string_lossy().to_string());
        self.photos = vec![PhotoVm { meta, rating: Rating::default() }];
        self.photo_index = HashMap::from([(id, 0)]);
        self.batches = vec![BatchVm { id: batch_id(id), index: 0, photo_ids: vec![id], visited: false, provisional: true }];
        self.current_batch_index = 0;
        self.current_photo_index = 0;
        self.phase = Phase::Loading;
        self.last_error = None;
        self.catalog_revision = self.catalog_revision.saturating_add(1);
        Ok(())
    }

    /// Installs the finished scan behind the placeholder. `Session::open_in` runs on the app's
    /// thread; the model only swaps the catalog when it arrives.
    pub fn finish_open(&mut self, session: Session) {
        self.load_session(session);
        self.refresh_recents();
    }

    /// The scan failed after the placeholder painted: back to Welcome, with the reason kept.
    pub fn fail_open(&mut self, message: String) {
        self.session = None;
        self.folder = None;
        self.folder_name.clear();
        self.photos.clear();
        self.photo_index.clear();
        self.batches.clear();
        self.current_batch_index = 0;
        self.current_photo_index = 0;
        self.pending_signatures.clear();
        self.last_error = Some(message);
        self.phase = Phase::Welcome;
        self.catalog_revision = self.catalog_revision.saturating_add(1);
    }

    /// A listener the app's scan thread can hand to `Session::open_in`, so re-batches and XMP
    /// failures queue on the same channels [`Model::drain_session_messages`] already drains.
    pub fn session_listener(&self) -> Arc<dyn SessionListener> {
        Arc::new(SessionForwarder { errors: Arc::clone(&self.pending_errors), batches: Arc::clone(&self.pending_batches) })
    }

    /// Seeds the model from a session (the split point `bench` also uses: scan → model).
    pub fn load_session(&mut self, session: Session) {
        let snapshot = session.snapshot();
        self.photo_index = snapshot.photos.iter().enumerate().map(|(position, meta)| (meta.id, position)).collect();
        self.photos =
            snapshot.photos.into_iter().map(|meta| PhotoVm { rating: snapshot.ratings.get(&meta.id.0).copied().unwrap_or_default(), meta }).collect();
        self.batches = snapshot.batches.iter().map(|batch| BatchVm::of(batch, snapshot.visited.contains(&batch.id.0))).collect();
        self.current_batch_index = 0;
        self.current_photo_index = 0;
        // Restore the cursor the database kept: the batch it points into, if re-batching has not
        // eaten it, and the photo it pointed at within that batch.
        if let Some(cursor) = snapshot.cursor
            && let Some(position) = self.batches.iter().position(|b| b.id == cursor.batch)
        {
            self.current_batch_index = position;
            if let Some(within) = self.batches.get(position).and_then(|batch| batch_position_of(batch, cursor.photo)) {
                self.current_photo_index = within;
            }
        }
        self.session = Some(session);
        self.phase = Phase::Culling;
        self.finish = crate::finish::FinishStage::Hidden;
        self.last_error = None;
        self.catalog_revision = self.catalog_revision.saturating_add(1);
        // Thumbnails decoded against the placeholder catalog still count; hand their signatures
        // to the session now that one exists.
        if !self.pending_signatures.is_empty()
            && let Some(session) = self.session.as_ref()
        {
            session.submit_visual_sigs(std::mem::take(&mut self.pending_signatures));
        }
    }

    /// Reconciles the open folder with the disk (the watcher calls this, and `file.rescan` does).
    /// Photos that stayed keep their cache entries; the cursor follows its photo by id.
    pub fn rescan_folder(&mut self) -> Result<(), String> {
        let rescanned = match self.session.as_ref() {
            Some(session) => session.rescan().map(|_| session.snapshot()),
            None => return Ok(()),
        };
        let snapshot = match rescanned {
            Ok(snapshot) => snapshot,
            Err(err) => {
                let message = err.to_string();
                self.last_error = Some(message.clone());
                return Err(message);
            }
        };
        self.adopt_snapshot(snapshot);
        Ok(())
    }

    /// Installs a snapshot the session already produced. The cursor follows its photo by id, so a
    /// rescan or a Finish run moves the view with the files rather than jumping to the top.
    fn adopt_snapshot(&mut self, snapshot: SessionSnapshot) {
        let current = self.current_photo_id();
        self.photo_index = snapshot.photos.iter().enumerate().map(|(position, meta)| (meta.id, position)).collect();
        self.photos = snapshot
            .photos
            .into_iter()
            .map(|meta| {
                let rating = snapshot.ratings.get(&meta.id.0).copied().unwrap_or_default();
                PhotoVm { meta, rating }
            })
            .collect();
        self.batches = snapshot.batches.iter().map(|batch| BatchVm::of(batch, snapshot.visited.contains(&batch.id.0))).collect();
        if let Some(photo) = current
            && let Some(position) = self.batches.iter().position(|batch| batch.photo_ids.contains(&photo))
        {
            self.current_batch_index = position;
            self.current_photo_index = self.batches.get(position).and_then(|batch| batch_position_of(batch, photo)).unwrap_or(0);
        }
        self.current_batch_index = self.current_batch_index.min(self.batches.len().saturating_sub(1));
        self.current_photo_index = self.current_photo_index.min(self.current_batch().map_or(0, BatchVm::count).saturating_sub(1));
        self.catalog_revision = self.catalog_revision.saturating_add(1);
        self.last_error = None;
        self.drain_session_messages();
    }

    /// Flushes the sidecar writer and checkpoints the database. Called on quit: the queue is
    /// debounced, so a window closing within a second of a rating would otherwise lose it.
    pub fn close_session(&self) {
        if let Some(session) = self.session.as_ref() {
            session.close();
        }
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
            if by < 0 && self.move_batch_selecting(-1, Selecting::Last) {}
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
        let Some(photo) = session.snapshot().last_photo_in_batch.get(&batch.id.0).copied().and_then(|id| self.photo(PhotoId(id))) else {
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
        if let Some(session) = self.session.as_ref()
            && let Err(err) = session.set_rating(photo_id, updated)
        {
            self.last_error = Some(err.to_string());
        }
        self.apply_rating_locally(photo_id, updated);
        if advance_after && self.auto_advance() {
            self.advance();
        }
        Some(updated)
    }

    /// A change that is already persisted gets only mirrored locally (undo, reject batch).
    fn apply_rating_locally(&mut self, photo_id: PhotoId, rating: Rating) {
        if let Some(position) = self.photo_index.get(&photo_id)
            && let Some(photo) = self.photos.get_mut(*position)
        {
            photo.rating = rating;
        }
    }

    /// Switching mode mid-session is allowed and loses nothing: stars and keep are kept
    /// consistent, and the visible tier is derived at read time (todo.md §6).
    pub fn set_rating_mode(&mut self, mode: RatingMode) {
        self.settings.rating_mode = mode;
        self.persist_settings();
    }

    pub fn set_auto_advance(&mut self, on: bool) {
        self.settings.auto_advance = on;
        self.auto_advance_override = None;
        self.persist_settings();
    }

    /// Caps-Lock's session-scoped override: deliberately NOT persisted (v1's behaviour — it is
    /// the session's say, not the app's).
    pub fn toggle_auto_advance(&mut self) {
        self.auto_advance_override = Some(!self.auto_advance());
    }

    pub fn set_keep_stars(&mut self, stars: u8) {
        // 4..=5 are the two meanings the app offers (`clamp_keep_stars`).
        self.settings.keep_stars = clamp_keep_stars(stars);
        if let Some(session) = self.session.as_ref() {
            session.set_keep_stars(self.settings.keep_stars);
        }
        self.persist_settings();
    }

    /// The metadata half of the settings (sidecars, keep spelling, DNG writes). Settings →
    /// Metadata edits this shape; only the two sidecar switches are wired into the live session
    /// today, the rest reach the writer through the Finish/XMP plumbing.
    pub fn metadata_settings(&self) -> crate::settings::MetadataJson {
        self.settings_store.as_ref().map(|store| store.current().metadata.clone()).unwrap_or_default()
    }

    pub fn set_viewer_settings(&mut self, viewer: crate::settings::ViewerJson) {
        if let Some(store) = self.settings_store.as_mut()
            && let Err(err) = store.set_viewer(viewer)
        {
            self.last_error = Some(err.to_string());
        }
    }

    pub fn set_metadata_settings(&mut self, metadata: crate::settings::MetadataJson) {
        if let Some(session) = self.session.as_ref() {
            let xmp = crate::session::XmpSettings { write_sidecars: metadata.write_xmp, sidecars_for_non_raw: metadata.write_sidecars_for_jpegs };
            if let Err(err) = session.set_xmp_settings(xmp) {
                self.last_error = Some(err.to_string());
            }
        }
        if let Some(store) = self.settings_store.as_mut()
            && let Err(err) = store.set_metadata(metadata)
        {
            self.last_error = Some(err.to_string());
        }
    }

    pub fn set_performance_settings(&mut self, performance: crate::settings::PerformanceJson) {
        if let Some(store) = self.settings_store.as_mut()
            && let Err(err) = store.set_performance(performance)
        {
            self.last_error = Some(err.to_string());
        }
    }

    /// Settings → General applies as one change: one write, and the fields with side effects
    /// (keep stars reaching the session, a new auto-advance value clearing Caps Lock's override)
    /// are honoured exactly as their individual setters would.
    pub fn set_general_settings(&mut self, general: GeneralSettings) {
        let auto_advance_changed = self.settings.auto_advance != general.auto_advance;
        self.settings = general;
        if auto_advance_changed {
            self.auto_advance_override = None;
        }
        if let Some(session) = self.session.as_ref() {
            session.set_keep_stars(self.settings.keep_stars);
        }
        self.persist_settings();
    }

    /// Settings → General → "Reject Whole Batch" (off by default; v1's `rejectWholeBatch`).
    pub fn set_reject_whole_batch(&mut self, on: bool) {
        self.settings.reject_whole_batch = on;
        self.persist_settings();
    }

    pub fn set_arrow_behavior(&mut self, behavior: ArrowBehavior) {
        self.settings.arrow_behavior = behavior;
        self.persist_settings();
    }

    pub fn set_entering_behavior(&mut self, behavior: EnteringBatchBehavior) {
        self.settings.entering_behavior = behavior;
        self.persist_settings();
    }

    /// Writes the general half back through the attached store, when a surface gave the model
    /// one. Failures land in `last_error` like every other side effect the window shows; without
    /// a store (tests, `bench`) persistence is a no-op.
    fn persist_settings(&mut self) {
        if let Some(store) = self.settings_store.as_mut()
            && let Err(err) = store.set_general_from(self.settings.clone())
        {
            self.last_error = Some(err.to_string());
        }
    }

    // ---- rating primitives (the one place a field changes) ---------------------------------

    pub fn rate_stars(&mut self, stars: u8) -> Option<Rating> {
        let stars = stars.min(5);
        self.rate_current(|rating| Rating { stars, ..rating })
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
        self.rate_current(|rating| Rating { flag: if rating.flag == Flag::Pick { Flag::None } else { Flag::Pick }, ..rating })
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
        self.rate_current(|rating| Rating { keep: !rating.keep, ..rating })
    }

    pub fn reject_flag(&mut self) -> Option<Rating> {
        self.rate_current(|rating| Rating { flag: Flag::Reject, ..rating })
    }

    pub fn unflag(&mut self) -> Option<Rating> {
        self.rate_current(|rating| Rating { flag: Flag::None, ..rating })
    }

    pub fn toggle_flag(&mut self) -> Option<Rating> {
        self.rate_current(|rating| Rating { flag: if rating.flag == Flag::None { Flag::Pick } else { Flag::None }, ..rating })
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
            // v1's rule (`rejectCurrentBatch`): only photos nobody has rated yet. A batch that is
            // half culled must not have its Good or 3-star calls flattened to Rejected.
            .filter(|(_, rating)| rating.is_neutral())
            .map(|(id, rating)| (id, Rating { flag: Flag::Reject, ..rating }))
            .filter(|(id, updated)| self.photo(*id).is_some_and(|photo| photo.rating != *updated))
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
            // `apply_history_entry` already wrote the restored value to the database: whatever
            // was found rides `before`, the restored value rides `after`, for BOTH undo and redo
            // (session.rs documents the pairing). Mirror the restored value.
            self.apply_rating_locally(change.photo, change.after);
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
        let latest: Vec<Batch> = self.pending_batches.lock().map(|mut queue| queue.drain(..).next_back().unwrap_or_default()).unwrap_or_default();
        if latest.is_empty() {
            // No re-batch pending: the model's batches are already the real ones, and draining
            // them with an empty list must not blank them out.
            return;
        }
        // The latest re-batch is authoritative; visited is per-batch and a batch the user was in
        // is frozen (only unvisited ones can differ).
        self.batches = latest
            .iter()
            .map(|batch| {
                let visited = self.batches.iter().find(|old| old.id == batch.id).is_some_and(|old| old.visited);
                BatchVm::of(batch, visited)
            })
            .collect();
        // The current photo may have moved batch; follow it by id, or reset to the first photo.
        match self.current_photo_id() {
            Some(photo) => {
                let position = self.batches.iter().position(|batch| batch.photo_ids.contains(&photo));
                self.current_batch_index = position.unwrap_or(0);
                self.current_photo_index = position.and_then(|p| self.batches.get(p)).and_then(|batch| batch_position_of(batch, photo)).unwrap_or(0);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn synthetic(index: u32) -> Vec<u8> {
        firstcut_meta::meta::cr3::SyntheticCr3::r8().at("2026:08:27 19:54:00").subsec(&format!("{index:02}")).build()
    }

    #[test]
    fn the_fast_path_shows_one_photo_before_the_scan_installs_the_catalog() {
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        for index in 1..=4u32 {
            std::fs::write(folder.path().join(format!("IMG_000{index}.CR3")), synthetic(index)).unwrap();
        }
        let mut model = Model::new();
        model.begin_open(folder.path()).expect("the placeholder header reads");
        assert_eq!(model.phase(), Phase::Loading);
        assert_eq!(model.photos().len(), 1, "one placeholder frame, not the shoot");
        let placeholder = model.current_photo_id().expect("the placeholder is the cursor");
        assert!(!model.can_act(), "actions wait for the real catalog");

        let session = Session::open_in(folder.path(), sessions.path(), Arc::new(crate::session::NoListener)).expect("the scan opens");
        model.finish_open(session);
        assert_eq!(model.phase(), Phase::Culling);
        assert_eq!(model.photos().len(), 4, "the full scan replaced the placeholder");
        assert!(model.photos().iter().any(|photo| photo.meta.id == placeholder), "the frame on screen is still in the shoot");
        assert!(model.can_act());
    }

    #[test]
    fn a_failed_open_returns_to_welcome_with_the_reason() {
        let mut model = Model::new();
        model.begin_open(Path::new("/definitely/not/a/folder")).expect_err("a missing folder has no first photo");
        model.fail_open("boom".to_string());
        assert_eq!(model.phase(), Phase::Welcome);
        assert!(model.photos().is_empty());
        assert_eq!(model.last_error(), Some("boom"));
    }

    #[test]
    fn a_rebound_key_reaches_the_effective_map_and_the_user_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keymap.json");
        let mut model = Model::new();
        model.attach_keymap_store(crate::keymap::KeymapStore::at(&path));
        let chord = crate::keymap::KeyChord::new(crate::keymap::KeyName::N, vec![crate::keymap::Modifier::Command]);
        model.rebind_keymap("photo.next", None, &chord).unwrap();

        let mode = RatingMode::Stars;
        assert_eq!(model.keymap().binding(&chord, mode).map(|binding| binding.command.as_str()), Some("photo.next"));
        assert_eq!(model.keymap().chords_for("photo.next", None).len(), 1, "the default arrow is replaced");

        let reread = crate::keymap::KeymapStore::at(&path);
        assert_eq!(reread.effective().binding(&chord, mode).map(|binding| binding.command.as_str()), Some("photo.next"));
        assert_eq!(reread.overrides().bindings.len(), 1, "only the command the user changed is written");
    }

    #[test]
    fn rebinding_a_mode_specific_command_keeps_its_mode() {
        let directory = tempfile::tempdir().unwrap();
        let mut model = Model::new();
        model.attach_keymap_store(crate::keymap::KeymapStore::at(&directory.path().join("keymap.json")));
        let chord = crate::keymap::KeyChord::new(crate::keymap::KeyName::Q, vec![]);
        model.rebind_keymap("flag.pick", None, &chord).unwrap();
        assert_eq!(model.keymap().binding(&chord, RatingMode::Stars).map(|binding| binding.command.as_str()), Some("flag.pick"));
        assert!(model.keymap().binding(&chord, RatingMode::KeepNotKeep).is_none(), "Stars-only P stays Stars-only");
    }

    #[test]
    fn the_settings_screens_write_viewer_metadata_and_performance_back() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("settings.json");
        let mut model = Model::new();
        model.attach_settings_store(crate::settings::SettingsStore::at(&file));

        let mut viewer = model.viewer_settings();
        viewer.zoom_lock = true;
        viewer.background_gray = 0.2;
        viewer.info_fields.retain(|field| *field != crate::settings::InfoFieldJson::Lens);
        model.set_viewer_settings(viewer.clone());

        let mut metadata = model.metadata_settings();
        metadata.write_xmp = false;
        metadata.write_sidecars_for_jpegs = false;
        metadata.xmp_write_mode = crate::settings::XmpWriteModeJson::Overwrite;
        model.set_metadata_settings(metadata.clone());

        let mut performance = model.performance_settings();
        performance.decode_threads = 6;
        performance.look_ahead_batches = 4;
        model.set_performance_settings(performance.clone());

        let reread = crate::settings::SettingsStore::at(&file);
        // JSON floats round-trip within an ulp (250/255 is not exactly representable), so the
        // thresholds compare with a tolerance; the switches and field list are exact.
        assert_eq!(reread.current().viewer.zoom_lock, viewer.zoom_lock);
        assert_eq!(reread.current().viewer.background_gray, viewer.background_gray);
        assert_eq!(reread.current().viewer.info_fields, viewer.info_fields);
        assert!((reread.current().viewer.clipping_highlight_threshold - viewer.clipping_highlight_threshold).abs() < 1e-12);
        assert_eq!(reread.current().metadata, metadata);
        assert_eq!(reread.current().performance, performance);
        assert!(!reread.current().viewer.info_fields.contains(&crate::settings::InfoFieldJson::Lens));
        assert_eq!(model.metadata_settings().xmp_write_mode, crate::settings::XmpWriteModeJson::Overwrite);
    }

    #[test]
    fn a_whole_batch_reject_is_off_by_default_and_undoes_as_one_step() {
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        for index in 1..=3u32 {
            std::fs::write(folder.path().join(format!("IMG_000{index}.CR3")), synthetic(index)).unwrap();
        }
        let mut model = Model::new();
        model.open_folder_in(folder.path(), sessions.path()).unwrap();

        assert!(!model.reject_current_batch(), "the setting is off by default");
        assert!(model.photos().iter().all(|photo| photo.rating.flag != Flag::Reject), "nothing changed");

        model.set_reject_whole_batch(true);
        assert!(model.reject_current_batch(), "the enabled command rejects the batch");
        assert_eq!(model.photos().iter().filter(|photo| photo.rating.flag == Flag::Reject).count(), 3, "every frame in the batch");

        assert!(model.undo(), "the whole batch is one step");
        assert!(model.photos().iter().all(|photo| photo.rating.flag != Flag::Reject), "one undo takes the batch back");
    }

    /// v1's rule: Reject Whole Batch only touches frames nobody has rated yet. A half-culled
    /// batch must keep its Good calls and star ratings.
    #[test]
    fn a_whole_batch_reject_leaves_photos_that_were_already_rated_alone() {
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        for index in 1..=3u32 {
            std::fs::write(folder.path().join(format!("IMG_000{index}.CR3")), synthetic(index)).unwrap();
        }
        let mut model = Model::new();
        model.open_folder_in(folder.path(), sessions.path()).unwrap();
        model.set_reject_whole_batch(true);
        model.rate_stars(3);
        assert!(model.photos().iter().any(|photo| photo.rating.stars == 3));

        assert!(model.reject_current_batch());
        assert_eq!(model.photos().iter().filter(|photo| photo.rating.flag == Flag::Reject).count(), 2);
        let rated = model.photos().iter().find(|photo| photo.rating.stars == 3).expect("the rated frame stays");
        assert_eq!(rated.rating.flag, Flag::None, "an already-rated frame is not rejected");

        assert!(model.undo(), "the two flags undo as one step");
        assert!(model.photos().iter().all(|photo| photo.rating.flag == Flag::None));
    }

    #[test]
    fn the_finish_sheet_walks_summary_options_dry_run_report_and_undo() {
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        for index in 1..=3u32 {
            std::fs::write(folder.path().join(format!("IMG_000{index}.CR3")), synthetic(index)).unwrap();
        }
        let mut model = Model::new();
        model.open_folder_in(folder.path(), sessions.path()).unwrap();
        model.rate_stars(5);
        assert_eq!(model.finish_stage().name(), "hidden");

        model.start_finish().unwrap();
        assert_eq!(model.finish_stage().name(), "confirm", "Confirm before Finish is on by default");
        model.confirm_finish().unwrap();
        assert_eq!(model.finish_stage().name(), "summary");
        let summary = model.finish_stage().summary().cloned().unwrap();
        assert_eq!(summary.total_photos, 3);
        assert_eq!(summary.kept_count(), 1);

        model.show_finish_options().unwrap();
        assert_eq!(model.finish_stage().name(), "options");
        model.set_finish_options(Some(firstcut_fileops::UnkeptAction::MoveToSubfolder("_Not kept".to_string())), None).unwrap();
        model.run_finish_dry_run().unwrap();
        assert_eq!(model.finish_stage().name(), "dryRun");
        let crate::finish::FinishStage::DryRun { plan, requires_typed_confirmation, .. } = model.finish_stage() else {
            panic!("the dry run is showing")
        };
        assert!(!plan.is_empty());
        assert!(!requires_typed_confirmation, "a move into a subfolder is undoable");

        model.execute_finish(None).unwrap();
        assert_eq!(model.finish_stage().name(), "report");
        let report = match model.finish_stage() {
            crate::finish::FinishStage::Report { report, .. } => report.clone(),
            other => panic!("expected a report, got {}", other.name()),
        };
        assert!(report.is_clean());
        assert!(report.undoable);
        assert_eq!(model.photos().len(), 1, "the filmstrip drops the photos that left the shoot");

        model.undo_finish().unwrap();
        let report = match model.finish_stage() {
            crate::finish::FinishStage::Report { report, .. } => report.clone(),
            other => panic!("expected a report, got {}", other.name()),
        };
        assert!(report.was_undo);
        assert_eq!(model.photos().len(), 3, "undo brings them back into the cull");
    }

    #[test]
    fn a_permanent_delete_needs_the_typed_word_even_through_the_registry() {
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        for index in 1..=2u32 {
            std::fs::write(folder.path().join(format!("IMG_000{index}.CR3")), synthetic(index)).unwrap();
        }
        let mut model = Model::new();
        model.open_folder_in(folder.path(), sessions.path()).unwrap();
        crate::registry::dispatch(&mut model, "cull.finish", &json!({})).unwrap();
        crate::registry::dispatch(&mut model, "finish.confirm", &json!({})).unwrap();
        crate::registry::dispatch(&mut model, "finish.options", &json!({})).unwrap();
        crate::registry::dispatch(&mut model, "finish.unkept", &json!({ "action": "delete" })).unwrap();
        crate::registry::dispatch(&mut model, "finish.preview", &json!({})).unwrap();
        let state = crate::registry::dispatch(&mut model, "finish.state", &json!({})).unwrap();
        assert_eq!(state["finish"]["requiresTypedConfirmation"], true);

        let refused = crate::registry::dispatch(&mut model, "finish.execute", &json!({}));
        assert!(refused.is_err(), "the model, not just the view, checks the word");
        assert_eq!(model.finish_stage().name(), "dryRun", "a refused run stays on the preview");
        assert!(folder.path().join("IMG_0001.CR3").exists());

        crate::registry::dispatch(&mut model, "finish.execute", &json!({ "confirm": "DELETE" })).unwrap();
        assert_eq!(model.finish_stage().name(), "report");
        assert!(!folder.path().join("IMG_0001.CR3").exists());
    }

    #[test]
    fn an_opened_shoot_is_a_recent_and_reopening_it_resumes_the_cursor() {
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        for index in 1..=3u32 {
            std::fs::write(folder.path().join(format!("IMG_000{index}.CR3")), synthetic(index)).unwrap();
        }
        let mut model = Model::new();
        model.open_folder_in(folder.path(), sessions.path()).unwrap();
        model.go_to_photo(1);
        let cursor = model.current_photo_id().expect("the shoot has photographs");

        let recents = model.recents();
        assert_eq!(recents.len(), 1, "the opened shoot is the one recent");
        assert_eq!(recents.first().map(|shoot| shoot.name.as_str()), Some(folder.path().file_name().unwrap().to_str().unwrap()));
        assert_eq!(recents.first().map(|shoot| shoot.photo_count), Some(3));
        assert_eq!(
            std::fs::canonicalize(&recents[0].folder).unwrap_or_else(|_| recents[0].folder.clone()),
            std::fs::canonicalize(folder.path()).unwrap()
        );

        // A fresh process (a new model) sees the list from the same sessions directory, and
        // opening the folder is all "resume" has to do: the database restores the cursor.
        let mut reopened = Model::new();
        reopened.attach_sessions_dir(sessions.path());
        assert_eq!(reopened.recents().len(), 1);
        reopened.open_folder_in(&reopened.recents()[0].folder.clone(), sessions.path()).unwrap();
        assert_eq!(reopened.current_photo_id(), Some(cursor), "the cursor comes back with the shoot");
    }

    #[test]
    fn a_rescan_keeps_the_cursor_and_bumps_the_catalog_revision() {
        let sessions = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        for index in 1..=3u32 {
            std::fs::write(folder.path().join(format!("IMG_000{index}.CR3")), synthetic(index)).unwrap();
        }
        let mut model = Model::new();
        model.open_folder_in(folder.path(), sessions.path()).unwrap();
        model.go_to_photo(1);
        let cursor = model.current_photo_id().expect("the shoot has photographs");
        let revision = model.catalog_revision();

        std::fs::write(folder.path().join("IMG_0004.CR3"), synthetic(4)).unwrap();
        model.rescan_folder().expect("the rescan reads the new file");

        assert_eq!(model.photos().len(), 4, "the new photograph is part of the catalog");
        assert_eq!(model.current_photo_id(), Some(cursor), "the cursor follows its photograph by id");
        assert!(model.catalog_revision() > revision, "the decode bank must see a catalog change");
    }
}
