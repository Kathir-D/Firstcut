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
use serde_json::json;

mod control;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut smoke = false;
    let mut control: Option<u16> = None;
    let mut folder: Option<PathBuf> = None;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--smoke" => smoke = true,
            "--control" => match args.get(index + 1).and_then(|p| p.parse::<u16>().ok()) {
                Some(port) => {
                    control = Some(port);
                    index += 1; // the port is consumed as a value, not a folder
                }
                None => {
                    eprintln!("firstcut: `--control` needs a TCP port number (1..=65535)");
                    std::process::exit(2);
                }
            },
            non_flag if !non_flag.starts_with("--") => match folder {
                None => folder = Some(PathBuf::from(non_flag)),
                Some(_) => {
                    eprintln!("firstcut: unexpected argument `{non_flag}` (one shoot folder)");
                    std::process::exit(2);
                }
            },
            unknown => {
                eprintln!("firstcut: unknown option `{unknown}`");
                std::process::exit(2);
            }
        }
        index += 1;
    }
    if let Err(err) = run(smoke, folder, control) {
        eprintln!("firstcut: {err}");
        std::process::exit(1);
    }
}

fn run(smoke: bool, folder: Option<PathBuf>, control: Option<u16>) -> Result<(), String> {
    // The control channel starts before the window so a script can connect and drive
    // everything from the first frame; a bind failure is a startup failure, not silence.
    // `wake_slot` is filled in by the first frame: queued requests must wake the UI thread.
    let wake_slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let control_receiver = match control {
        Some(port) => Some(control::start(port, std::sync::Arc::clone(&wake_slot))?),
        None => None,
    };
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
        Box::new(move |_cc| Ok(Box::new(Shell::new(smoke, control_receiver, wake_slot, model)))),
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
    control_receiver: Option<mpsc::Receiver<control::Request>>,
    /// Filled in by the first frame; the control thread reads it to wake this one. It is set
    /// exactly once, so `Option` is not a state machine, it is "before the first paint".
    wake_slot: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>,
    wake_noticed: bool,
}

impl Shell {
    fn new(
        smoke: bool,
        control_receiver: Option<mpsc::Receiver<control::Request>>,
        wake_slot: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>,
        model: Model,
    ) -> Self {
        Self {
            model,
            screen: firstcut_ui::CullScreen::new(),
            bank: DecodeBank::new(),
            frames_painted: 0,
            smoke_logged: false,
            smoke,
            control_receiver,
            wake_slot,
            wake_noticed: false,
        }
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.wake_noticed {
            self.wake_noticed = true;
            if let Ok(mut wake) = self.wake_slot.lock() {
                *wake = Some(ui.ctx().clone());
            }
        }

        // Control clients first: their engine.execute runs inside this frame's pass, so an
        // agent's command and the result it reads back are the same frame's facts.
        if let Some(receiver) = self.control_receiver.take() {
            loop {
                match receiver.try_recv() {
                    Ok(request) => {
                        self.answer_control(request);
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        eprintln!("firstcut control: the listener is gone");
                        break;
                    }
                }
            }
            self.control_receiver = Some(receiver);
        }
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

// ---- the control channel's UI half --------------------------------------------------------

impl Shell {
    /// One queued request, answered against the live state inside this frame.
    fn answer_control(&mut self, request: control::Request) {
        match request {
            control::Request::Dispatch { id, params, reply } => {
                let _ = reply.send(firstcut_engine::registry::dispatch(&mut self.model, &id, &params));
            }
            control::Request::Widgets { reply } => {
                let _ = reply.send(self.ephemeral_ui_pass(None));
            }
            control::Request::Click { name, reply } => {
                let _ = reply.send(self.ephemeral_ui_pass(Some(name)));
            }
            control::Request::Screenshot { reply } => {
                let _ = reply.send(self.harness_screenshot());
            }
        }
    }

    /// Renders the live state through the same headless harness the snapshot tests use — the
    /// agents' `ui.widgets`/`ui.clickWidget` drive the real widget tree, not a shadow of it.
    /// `click` names the accessible name of the widget to click; `None` only lists.
    fn ephemeral_ui_pass(&mut self, click: Option<String>) -> Result<serde_json::Value, String> {
        use egui_kittest::kittest::{NodeT, Queryable};

        // Field-split borrows: the pass paints the model through a fresh screen (fresh texture
        // cache), the pixels come straight from the bank.
        let Shell { model, bank, .. } = self;
        let mut screen = firstcut_ui::CullScreen::new();
        let mut harness = egui_kittest::Harness::builder().with_size(control::HARNESS_SIZE).build_ui(|ui| {
            let current_id = model.current_photo_id();
            let current = current_id.and_then(|id| bank.image(id.0).map(|pixels| (id, pixels)));
            let images = firstcut_ui::FrameImages { current, thumbs: &bank.cache };
            screen.show(ui, model, &images);
        });
        harness.run();

        if let Some(name) = click {
            let found: Vec<_> = harness.query_all_by_label(&name).collect();
            let Some(target) = found.first() else {
                return Err(format!("no widget is named `{name}` — ui.widgets lists the names"));
            };
            target.click();
            drop(found); // the node borrows the harness; the click needs a frame to land
            harness.run();
        }

        // One entry per named node; rects so a script can also click at points.
        let mut widgets: Vec<serde_json::Value> = Vec::new();
        for node in harness.root().children_recursive() {
            let label = node.accesskit_node().label().or_else(|| node.accesskit_node().value());
            if let Some(label) = label {
                let rect = node.rect();
                widgets.push(json!({
                    "name": label,
                    "role": format!("{:?}", node.accesskit_node().role()),
                    "rect": [rect.min.x, rect.min.y, rect.width(), rect.height()],
                }));
            }
        }
        Ok(json!({ "widgets": widgets }))
    }

    /// `ui.screenshot`: the live state as a PNG, the path handed back for the caller to fetch.
    fn harness_screenshot(&mut self) -> Result<serde_json::Value, String> {
        let Shell { model, bank, .. } = self;
        let mut screen = firstcut_ui::CullScreen::new();
        let mut harness = egui_kittest::Harness::builder().with_size(control::HARNESS_SIZE).build_ui(|ui| {
            let current_id = model.current_photo_id();
            let current = current_id.and_then(|id| bank.image(id.0).map(|pixels| (id, pixels)));
            let images = firstcut_ui::FrameImages { current, thumbs: &bank.cache };
            screen.show(ui, model, &images);
        });
        harness.run();
        let image = harness.render()?;
        let path = std::env::temp_dir().join(format!(
            "firstcut-screenshot-{}.png",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default()
        ));
        image.save(&path).map_err(|e| format!("writing {}: {e}", path.display()))?;
        Ok(json!({ "path": path.display().to_string() }))
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
