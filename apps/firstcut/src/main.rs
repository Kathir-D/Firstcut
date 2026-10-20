//! The v2 shell application (D1): wires the engine to the egui UI and owns the
//! process. At M1 it opens a shoot folder (`firstcut <folder>`), persists the
//! General settings next to the Sessions directory, and shows the culling
//! screen — filmstrip + loupe + the rating keys — through the registry. The
//! `--smoke` flag creates the window, paints a few frames and exits 0 — that is
//! the cross-OS proof the CI matrix runs (docs/v2-plan.md §6, M0/M1).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use eframe::egui;
use firstcut_engine::model::Model;
use firstcut_engine::settings::SettingsStore;
use firstcut_meta::meta::PhotoMeta;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let smoke = args.iter().any(|a| a == "--smoke");
    let folder = args.iter().filter(|a| a.as_str() != "--smoke").map(PathBuf::from).next();
    if let Err(err) = run(smoke, folder) {
        eprintln!("firstcut: {err}");
        std::process::exit(1);
    }
}

fn run(smoke: bool, folder: Option<PathBuf>) -> Result<(), String> {
    let mut model = Model::new();
    // v1's AppModel construction order: load the settings file, seed the General
    // half from it, then open. A file that will not read is v1's defaults.
    if let Some(store) = settings_store() {
        model.attach_settings_store(store);
    }
    if let Some(folder) = folder.as_deref() {
        // Errors stay on the model (phase welcome, last_error set); the console
        // line is for the terminal that launched the window.
        if let Err(err) = model.open_folder(folder) {
            eprintln!("firstcut: {err}");
        }
    }
    eframe::run_native(
        "firstcut",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_title("Firstcut").with_inner_size([1200.0, 760.0]).with_min_inner_size([640.0, 380.0]),
            ..Default::default()
        },
        Box::new(move |_cc| Ok(Box::new(Shell::new(smoke, model)))),
    )
    .map_err(|e| format!("eframe failed to create the window: {e}"))
}

/// The settings file lives next to the Sessions directory (the `Sessions`
/// level's parent), the same place v1's `SettingsModel` wrote `settings.json`.
fn settings_store() -> Option<SettingsStore> {
    let sessions = firstcut_catalog::sessions_dir().ok()?;
    let file = sessions.parent()?.join("settings.json");
    Some(SettingsStore::at(&file))
}

struct Shell {
    model: Model,
    screen: firstcut_ui::CullScreen,
    bank: DecodeBank,
    frames_painted: u32,
    smoke_logged: bool,
    smoke: bool,
}

impl Shell {
    fn new(smoke: bool, model: Model) -> Self {
        Self { model, screen: firstcut_ui::CullScreen::new(), bank: DecodeBank::new(), frames_painted: 0, smoke_logged: false, smoke }
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Finished decodes land in the cache, then what the surfaces want next
        // is queued — ahead of repainting, so a decode has the whole frame's
        // time to get back before the next try.
        if self.bank.pump(&self.model) {
            ui.ctx().request_repaint();
        }

        let current_id = self.model.current_photo_id();
        let current = current_id.and_then(|id| self.bank.image(id.0).map(|pixels| (id, pixels)));
        let images = firstcut_ui::FrameImages { current, thumbs: &self.bank.cache };
        self.screen.show(ui, &mut self.model, &images);

        self.frames_painted += 1;
        // `--smoke`: prove the window exists and paints, then close cleanly.
        if self.smoke && self.frames_painted >= 5 {
            if !self.smoke_logged {
                self.smoke_logged = true;
                eprintln!("smoke: painting frame {}", self.frames_painted);
            }
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

// ---- decoding -----------------------------------------------------------------------------

/// One decode the UI asked for. The address and the metadata travel together so
/// the worker never needs the model.
struct Job {
    id: u64,
    path: PathBuf,
    meta: PhotoMeta,
}

/// The app-side half of the decode pipeline for the slice: one worker thread
/// and a small cache of the shots the surfaces are looking at right now. The
/// priority window and tiers that replace this are M2's work.
struct DecodeBank {
    sender: mpsc::Sender<Job>,
    receiver: mpsc::Receiver<Result<(u64, egui::ColorImage), (u64, String)>>,
    /// Decoded pixels by photo id; `order` keeps the insertion order for eviction.
    cache: HashMap<u64, egui::ColorImage>,
    order: VecDeque<u64>,
    inflight: HashSet<u64>,
    /// Decodes that failed are never requested again, so a corrupt file costs
    /// one attempt, not one attempt per frame.
    failed: HashSet<u64>,
}

/// How many decoded shots the bank holds. The filmstrip plus the loupe need a
/// screenful; more only keeps pixels nobody can see.
const CACHE_LIMIT: usize = 64;
/// Decodes happening at once. A real burst arrive ~50 ms apart on a card; four
/// keeps the strip filling while the loupe's subject jumps the queue.
const INFLIGHT_LIMIT: usize = 4;

impl DecodeBank {
    /// The worker is the only way pixels arrive; a thread that could not start
    /// is not a crash — the bank just answers "not decoded yet" forever, and
    /// later sends fail through the closure-dropped `jobs` channel.
    fn new() -> Self {
        let (sender, jobs) = mpsc::channel::<Job>();
        let (results, receiver) = mpsc::channel();
        if let Err(err) = std::thread::Builder::new().name("firstcut-decode".to_string()).spawn(move || {
            while let Ok(job) = jobs.recv() {
                let decoded = firstcut_decode::decode_display(&job.path, &job.meta);
                let result = decoded
                    .map(|raw| (job.id, egui::ColorImage::from_rgba_unmultiplied([raw.width as usize, raw.height as usize], &raw.rgba)))
                    .map_err(|err| (job.id, err.to_string()));
                if results.send(result).is_err() {
                    break; // the window closed; the worker is done
                }
            }
        }) {
            eprintln!("firstcut: the decode worker could not start: {err}");
        }
        Self { sender, receiver, cache: HashMap::new(), order: VecDeque::new(), inflight: HashSet::new(), failed: HashSet::new() }
    }

    /// Drains finished decodes into the cache, then queues what the surfaces
    /// want next — the loupe's photo first, then the strip's, evicting the
    /// coldest entries when the cache is over its limit. `true` when pixels
    /// arrived, so the window repaints while the user is idle.
    fn pump(&mut self, model: &Model) -> bool {
        let mut changed = false;
        loop {
            match self.receiver.try_recv() {
                Ok(Ok((id, image))) => {
                    self.cache.insert(id, image);
                    self.order.push_back(id);
                    self.inflight.remove(&id);
                    changed = true;
                }
                Ok(Err((id, message))) => {
                    eprintln!("firstcut: {message}");
                    self.failed.insert(id);
                    self.inflight.remove(&id);
                }
                Err(_) => break,
            }
        }

        if let Some(folder) = model.folder() {
            self.queue_requests(model, folder);
        }
        self.evict(model.current_photo_id().map(|id| id.0));
        changed
    }

    /// One decode queued for the worker per not-yet-decoded photo, current
    /// photo first. Queues, never blocks: a worker that is gone is reported
    /// once and the rest of the frame carries on.
    fn queue_requests(&mut self, model: &Model, folder: &Path) {
        let mut wanted: Vec<u64> = Vec::new();
        if let Some(id) = model.current_photo_id() {
            wanted.push(id.0);
        }
        for batch in model.batches() {
            wanted.extend(batch.photo_ids.iter().map(|id| id.0));
        }

        let mut slots = INFLIGHT_LIMIT.saturating_sub(self.inflight.len());
        for id in wanted {
            if slots == 0 {
                return;
            }
            if self.cache.contains_key(&id) || self.inflight.contains(&id) || self.failed.contains(&id) {
                continue;
            }
            let Some(photo) = model.photos().iter().find(|photo| photo.meta.id.0 == id) else { continue };
            let job = Job { id, path: folder.join(&photo.meta.rel_path), meta: photo.meta.clone() };
            if self.sender.send(job).is_err() {
                eprintln!("firstcut: the decode worker is not running");
                return;
            }
            self.inflight.insert(id);
            slots -= 1;
        }
    }

    /// When the cache is over its limit, the coldest entries go first — but
    /// never the photo the loupe is showing, or it would decode in a loop.
    fn evict(&mut self, current: Option<u64>) {
        while self.cache.len() > CACHE_LIMIT {
            let Some(oldest) = self.order.pop_front() else { break };
            if current == Some(oldest) {
                self.order.push_back(oldest);
                break;
            }
            self.cache.remove(&oldest);
        }
    }

    fn image(&self, id: u64) -> Option<&egui::ColorImage> {
        self.cache.get(&id)
    }
}
