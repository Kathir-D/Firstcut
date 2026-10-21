//! The v2 shell application (D1): wires the engine to the egui UI and owns the
//! process. At M1 it opens a shoot folder (`firstcut <folder>`), persists the
//! General settings next to the Sessions directory, and shows the culling
//! screen — filmstrip + loupe + the rating keys — through the registry. The
//! `--smoke` flag creates the window, paints a few frames and exits 0 — that is
//! the cross-OS proof the CI matrix runs (docs/v2-plan.md §6, M0/M1).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;

use eframe::egui;
use firstcut_decode::{DecodedImage, Histogram, RawDecode, decode_display, decode_display_at, decode_thumbnail, histogram};
use firstcut_engine::model::{Model, Phase};
use firstcut_engine::pipeline::{FALLBACK_DISPLAY_EDGE, Focus, Job, Pipeline, Source, THUMBNAIL_EDGE, Tier, TierBudgets};
use firstcut_engine::session::Session;
use firstcut_engine::settings::SettingsStore;
use firstcut_engine::watch::{FolderWatcher, watch};
use firstcut_types::PhotoId;
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
    eframe::run_native(
        "firstcut",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_title("Firstcut").with_inner_size([1200.0, 760.0]).with_min_inner_size([640.0, 380.0]),
            ..Default::default()
        },
        Box::new(move |_cc| Ok(Box::new(Shell::new(smoke, control_receiver, wake_slot, model, folder)))),
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

/// The user's keymap overrides live beside `settings.json` (v1's `KeymapStore.defaultDirectory`).
fn keymap_store() -> Option<firstcut_engine::keymap::KeymapStore> {
    let sessions = firstcut_catalog::sessions_dir().ok()?;
    let file = sessions.parent()?.join("keymap.json");
    Some(firstcut_engine::keymap::KeymapStore::at(&file))
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
    /// The live folder watcher and the folder it watches (started on the first frame the model
    /// has a folder, restarted if the folder changes).
    watcher: Option<(FolderWatcher, mpsc::Receiver<()>)>,
    watched_folder: Option<PathBuf>,
    /// The real sessions directory, kept so the scan thread can open without asking the catalog
    /// again (and so a missing one still takes `Session::open`'s default).
    sessions_dir: Option<PathBuf>,
    /// The first-photo fast path's background scan. The placeholder is already on screen; when
    /// this delivers, `finish_open` swaps the real catalog in.
    open_job: Option<mpsc::Receiver<Result<Session, String>>>,
}

impl Shell {
    fn new(
        smoke: bool,
        control_receiver: Option<mpsc::Receiver<control::Request>>,
        wake_slot: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>,
        model: Model,
        folder: Option<PathBuf>,
    ) -> Self {
        let performance = model.performance_settings();
        let fraction = if performance.memory_budget_fraction.is_finite() && performance.memory_budget_fraction > 0.0 {
            performance.memory_budget_fraction
        } else {
            0.4
        };
        // The fraction is of physical RAM (v1 applied it to a measured amount); until a settings
        // screen offers a byte count, apply it to the same four-gigabyte baseline the pipeline's
        // default uses.
        let total_budget = (4.0 * 1024.0 * 1024.0 * 1024.0 * fraction / 0.4) as usize;
        let threads = if performance.decode_threads == 0 {
            std::thread::available_parallelism().map_or(4, |count| count.get()).min(4)
        } else {
            performance.decode_threads.clamp(1, 8) as usize
        };
        let raw: Option<Arc<dyn RawDecode>> = firstcut_decode_libraw::backend();
        let mut shell = Self {
            model,
            screen: firstcut_ui::CullScreen::new(),
            bank: DecodeBank::new(TierBudgets::from_total(total_budget), threads, raw),
            frames_painted: 0,
            smoke_logged: false,
            smoke,
            control_receiver,
            wake_slot,
            wake_noticed: false,
            watcher: None,
            watched_folder: None,
            sessions_dir: firstcut_catalog::sessions_dir().ok(),
            open_job: None,
        };
        // The Welcome screen lists the shoots already in this sessions directory; a folder
        // argument still wins and opens immediately.
        if let Some(dir) = shell.sessions_dir.clone() {
            shell.model.attach_sessions_dir(&dir);
        }
        if let Some(store) = keymap_store() {
            shell.model.attach_keymap_store(store);
        }
        if let Some(folder) = folder {
            shell.start_open(folder);
        }
        shell
    }

    /// Reads the placeholder header and starts the full scan on its own thread. The window can
    /// paint the first photograph before the scan returns; `poll_open` installs the catalog.
    fn start_open(&mut self, folder: PathBuf) {
        if let Err(err) = self.model.begin_open(&folder) {
            eprintln!("firstcut: {err}");
            self.model.fail_open(err);
            return;
        }
        let listener = self.model.session_listener();
        let sessions = self.sessions_dir.clone();
        let (sender, receiver) = mpsc::channel();
        let label = folder.display().to_string();
        let spawned = std::thread::Builder::new().name("firstcut-open".to_string()).spawn(move || {
            let result = match sessions {
                Some(dir) => Session::open_in(&folder, &dir, listener),
                None => Session::open(&folder, listener),
            };
            let _ = sender.send(result.map_err(|err| err.to_string()));
        });
        match spawned {
            Ok(_) => self.open_job = Some(receiver),
            Err(err) => {
                let message = format!("starting the scan for {label}: {err}");
                eprintln!("firstcut: {message}");
                self.model.fail_open(message);
            }
        }
    }

    /// Takes the scan's result when it lands. Before that the placeholder stays on screen.
    fn poll_open(&mut self) {
        let received = match self.open_job.as_ref() {
            Some(receiver) => receiver.try_recv(),
            None => return,
        };
        match received {
            Ok(Ok(session)) => {
                self.open_job = None;
                self.model.finish_open(session);
            }
            Ok(Err(message)) => {
                self.open_job = None;
                eprintln!("firstcut: {message}");
                self.model.fail_open(message);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.open_job = None;
                let message = "the scan thread stopped without a result".to_string();
                eprintln!("firstcut: {message}");
                self.model.fail_open(message);
            }
        }
    }

    /// Starts or restarts the watcher when the open folder changes. A watcher that cannot start
    /// is a logged degradation, not a crash: the user can still reopen the folder by hand.
    fn sync_watcher(&mut self) {
        let wanted = self.model.folder().map(Path::to_path_buf);
        if wanted == self.watched_folder {
            return;
        }
        self.watcher = None;
        self.watched_folder = wanted.clone();
        if let Some(folder) = wanted {
            match watch(&folder) {
                Ok((watcher, events)) => self.watcher = Some((watcher, events)),
                Err(err) => eprintln!("firstcut: {err}"),
            }
        }
    }

    /// Drains the watcher's "something changed" token and reconciles. Events that arrive while
    /// the app is busy coalesce into one rescan, which is what FSEvents gives us anyway.
    fn poll_watcher(&mut self) {
        // During the fast path the catalog is one placeholder and the scan is already on its way;
        // leave the token in the bounded channel so the change is reconciled once the real
        // session is in rather than being scanned into nothing.
        if self.model.phase() == Phase::Loading {
            return;
        }
        let mut changed = false;
        if let Some((_, events)) = self.watcher.as_ref() {
            while events.try_recv().is_ok() {
                changed = true;
            }
        }
        if changed && let Err(err) = self.model.rescan_folder() {
            eprintln!("firstcut: rescan failed: {err}");
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
        self.poll_open();
        self.sync_watcher();
        self.poll_watcher();

        // Finished decodes land in the cache, then what the surfaces want next is queued — ahead
        // of repainting, so a decode has the whole frame's time to get back before the next try.
        let viewport_edge = self.screen.viewport_edge();
        let zoomed = self.screen.is_zoomed();
        if self.bank.pump(&mut self.model, viewport_edge, zoomed) {
            ui.ctx().request_repaint();
        }
        // Visual signatures may have re-cut unvisited batches; take the latest snapshot before the
        // screen paints so the strip never shows boundaries the session has already moved.
        self.model.drain_session_messages();

        let current_id = self.model.current_photo_id();
        let current_frame = current_id.and_then(|id| self.bank.frame(id));
        let images = firstcut_ui::FrameImages {
            current: current_id.zip(current_frame).map(|(id, frame)| (id, frame.image.as_ref())),
            thumbs: &self.bank.thumbnails,
            histogram: current_frame.map(|frame| &frame.histogram),
            display: &self.bank.displays,
        };
        self.screen.show(ui, &mut self.model, &images);
        // File → Open Folder… leaves a request; opening is the app's async job.
        if let Some(folder) = self.screen.take_open_request() {
            self.start_open(folder);
        }

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

    /// Flush the debounced sidecar queue and checkpoint the database on quit (docs/v2-plan.md
    /// §4.2: "flush on quit"). A rating in the last second before closing must not be lost.
    fn on_exit(&mut self) {
        self.model.close_session();
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
            let current_frame = current_id.and_then(|id| bank.frame(id));
            let images = firstcut_ui::FrameImages {
                current: current_id.zip(current_frame).map(|(id, frame)| (id, frame.image.as_ref())),
                thumbs: &bank.thumbnails,
                histogram: current_frame.map(|frame| &frame.histogram),
                display: &bank.displays,
            };
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
            let current_frame = current_id.and_then(|id| bank.frame(id));
            let images = firstcut_ui::FrameImages {
                current: current_id.zip(current_frame).map(|(id, frame)| (id, frame.image.as_ref())),
                thumbs: &bank.thumbnails,
                histogram: current_frame.map(|frame| &frame.histogram),
                display: &bank.displays,
            };
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

/// One decoded frame as the surfaces receive it: the pixels plus the histogram computed from
/// those exact (already oriented) pixels. Caching them together is what makes the Info panel's
/// data seam orientation-safe — there is no second decode to disagree with the loupe.
struct Frame {
    image: Arc<egui::ColorImage>,
    histogram: Histogram,
}

fn frame_bytes(frame: &Arc<Frame>) -> usize {
    frame.image.pixels.len().saturating_mul(std::mem::size_of::<egui::Color32>())
}

enum DecodeResult {
    Done(Job, DecodedImage, Box<Histogram>),
    Failed(Job, String),
}

/// The app-side half of the decode pipeline: threads and channels around the engine's policy
/// (`firstcut_engine::pipeline`). The policy owns the window, cancellation, tiers and budgets;
/// this type only turns jobs into pixels and hands the results back.
struct DecodeBank {
    pipeline: Pipeline<Arc<Frame>>,
    /// One job channel per worker. Separate channels are what let every worker block on recv;
    /// a shared `Receiver` would leave all but one blocked on the mutex.
    senders: Vec<mpsc::Sender<Job>>,
    next_worker: usize,
    results: mpsc::Receiver<DecodeResult>,
    /// The filmstrip's pixels, kept as the same `Arc` the pipeline caches, so eviction in the
    /// policy is the only accounting and this map never holds a private copy.
    thumbnails: HashMap<u64, Arc<egui::ColorImage>>,
    /// Display-tier pixels by photo id, kept in step with the pipeline's cache.
    displays: HashMap<u64, Arc<egui::ColorImage>>,
    catalog_revision: u64,
    inflight_limit: usize,
    /// Thumbnails already offered to the batcher. A re-decode after eviction must not submit the
    /// same signature again; the batcher would no-op, but the work is avoidable.
    signatures_sent: HashSet<u64>,
}

impl DecodeBank {
    fn new(budgets: TierBudgets, workers: usize, raw: Option<Arc<dyn RawDecode>>) -> Self {
        let (results, receiver) = mpsc::channel::<DecodeResult>();
        let mut senders = Vec::new();
        for index in 0..workers.max(1) {
            let (sender, jobs) = mpsc::channel::<Job>();
            let results = results.clone();
            let raw = raw.clone();
            let name = format!("firstcut-decode-{index}");
            if let Err(err) = std::thread::Builder::new().name(name).spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let result = match decode_job(&job, raw.as_deref()) {
                        Ok((image, histogram)) => DecodeResult::Done(job, image, Box::new(histogram)),
                        Err(message) => DecodeResult::Failed(job, message),
                    };
                    if results.send(result).is_err() {
                        break; // the window closed; the worker is done
                    }
                }
            }) {
                eprintln!("firstcut: the decode worker could not start: {err}");
            }
            senders.push(sender);
        }
        Self {
            pipeline: Pipeline::new(budgets, frame_bytes),
            senders,
            next_worker: 0,
            results: receiver,
            thumbnails: HashMap::new(),
            displays: HashMap::new(),
            catalog_revision: u64::MAX,
            inflight_limit: workers.max(1) * 2,
            signatures_sent: HashSet::new(),
        }
    }

    /// Drains finished decodes, reconciles the catalog when the model says it changed, reports
    /// the focus window, and fills the workers. `true` when pixels arrived.
    fn pump(&mut self, model: &mut Model, viewport_edge: u32, zoomed: bool) -> bool {
        let mut changed = false;
        loop {
            match self.results.try_recv() {
                Ok(DecodeResult::Done(job, decoded, histogram)) => {
                    let edge = decoded.longest_edge();
                    let image = Arc::new(egui::ColorImage::from_rgba_unmultiplied([decoded.width as usize, decoded.height as usize], &decoded.rgba));
                    let frame = Arc::new(Frame { image, histogram: *histogram });
                    if self.pipeline.complete(&job, Arc::clone(&frame), edge) {
                        if job.tier == Tier::Thumbnail {
                            self.thumbnails.insert(job.id.0, Arc::clone(&frame.image));
                            // One decode, two consumers: the filmstrip gets the pixels and the
                            // batcher gets the signature of those exact pixels.
                            if self.signatures_sent.insert(job.id.0) {
                                model.submit_thumbnail_pixels(job.id, &decoded.rgba, decoded.width, decoded.height);
                            }
                        }
                        if job.tier == Tier::Display {
                            // Compare shows frames other than the cursor's; keep the same Arcs the
                            // loupe already paints rather than decoding anything twice.
                            self.displays.insert(job.id.0, Arc::clone(&frame.image));
                        }
                        changed = true;
                    }
                }
                Ok(DecodeResult::Failed(job, message)) => {
                    eprintln!("firstcut: {message}");
                    self.pipeline.fail(&job);
                }
                Err(_) => break,
            }
        }

        let revision = model.catalog_revision();
        if revision != self.catalog_revision {
            self.catalog_revision = revision;
            let catalog = model
                .folder()
                .map(|folder| {
                    model.photos().iter().map(|photo| Source { path: folder.join(&photo.meta.rel_path), meta: photo.meta.clone() }).collect()
                })
                .unwrap_or_default();
            self.pipeline.set_catalog(catalog);
        }

        let focus = Focus {
            windows: model.focus_windows(),
            current: model.current_photo_id(),
            viewport_edge,
            zoomed,
            zoom_lock: model.zoom_lock(),
            exact_raw: model.exact_raw(),
        };
        self.pipeline.set_focus(focus);
        self.fill_workers();

        // Drop the strip's references to thumbnails the policy evicted, so the map is a view of
        // the cache rather than a second unbounded one.
        let keep: HashSet<u64> = self.thumbnails.keys().copied().filter(|id| self.pipeline.contains(PhotoId(*id), Tier::Thumbnail)).collect();
        self.thumbnails.retain(|id, _| keep.contains(id));
        // Display Arcs are views of the pipeline's cache too: drop the ones it evicted.
        self.displays.retain(|id, _| self.pipeline.contains(PhotoId(*id), Tier::Display));
        changed
    }

    fn fill_workers(&mut self) {
        while self.pipeline.in_flight() < self.inflight_limit {
            let Some(job) = self.pipeline.next_job() else { break };
            let Some(sender) = self.senders.get(self.next_worker % self.senders.len().max(1)) else { break };
            if sender.send(job).is_err() {
                // A worker that is gone must not wedge the queue: let the next frame retry.
                break;
            }
            self.next_worker = self.next_worker.wrapping_add(1);
        }
    }

    /// The display frame under the cursor, if its decode has landed.
    fn frame(&self, id: PhotoId) -> Option<&Arc<Frame>> {
        self.pipeline.peek(id, Tier::Display)
    }
}

/// Decodes one job. The tier decides the source: thumbnails use the small `PRVW` JPEG, display
/// uses the full-resolution embedded JPEG at the viewer's edge, and Exact RAW develops the
/// sensor data. Every path applies EXIF orientation in `firstcut_decode`/the backend exactly
/// once, and the histogram is computed from the result — never from a second decode.
fn decode_job(job: &Job, raw: Option<&dyn RawDecode>) -> Result<(DecodedImage, Histogram), String> {
    let image = match job.tier {
        Tier::Thumbnail => decode_thumbnail(&job.path, &job.meta, THUMBNAIL_EDGE),
        Tier::Display => decode_display_at(&job.path, &job.meta, if job.target_edge == 0 { FALLBACK_DISPLAY_EDGE } else { job.target_edge }),
        Tier::ExactRaw => match raw {
            Some(backend) => backend.decode_raw(&job.path, &job.meta),
            // No backend on this platform: fall back to the camera's own rendering rather than
            // blanking the loupe. The exact tier still stays a tier of its own.
            None => decode_display(&job.path, &job.meta),
        },
    }
    .map_err(|err| format!("{} ({:?}): {err}", job.path.display(), job.tier))?;
    let histogram = if job.tier == Tier::Thumbnail { Histogram::empty() } else { histogram(&image) };
    Ok((image, histogram))
}
