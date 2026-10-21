//! The culling screen under the headless harness — the plan's "one headless-UI snapshot"
//! (docs/v2-plan.md §6, M1): a real session opened on synthetic CR3s, pixels the test mints
//! itself (the synthetic CR3 carries no embedded JPEG by design, so the app's decode bank is
//! stood in for by this bank), keyboard events through the same input path a real window has,
//! and the same `dispatch` the other surfaces use.
//!
//! The PNGs live in `tests/snapshots/`; a stale image is regenerated with
//! `UPDATE_SNAPSHOTS=true cargo test -p firstcut`.
//!
//! §4.5's never-crash lints are a product-code rule; this is a tests-only crate, so the
//! panic paths tests are allowed to take (clippy.toml's `allow-*-in-tests`) are granted here
//! at file scope.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui;
use egui_kittest::kittest::Queryable;
use firstcut_engine::model::Model;
use firstcut_types::Histogram;
use firstcut_ui::{CullScreen, FrameImages};

/// Three synthetic CR3s 10 ms apart (one burst, so one batch) in a scratch shoot, opened.
fn shoot() -> (tempfile::TempDir, tempfile::TempDir, Model) {
    let sessions = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    // A fixed shoot name: the toolbar shows it, and the snapshots must not carry a random
    // temp-directory suffix.
    let folder = root.path().join("Game1JENKS");
    std::fs::create_dir(&folder).unwrap();
    for index in 1..=3u32 {
        let bytes = firstcut_meta::meta::cr3::SyntheticCr3::r8().at("2026:08:27 19:54:00").subsec(&format!("{index:02}")).build();
        std::fs::write(folder.join(format!("IMG_000{index}.CR3")), bytes).unwrap();
    }
    let mut model = Model::new();
    model.open_folder_in(&folder, sessions.path()).unwrap_or_else(|err| panic!("the test shoot opens: {err}"));
    (sessions, root, model)
}

/// Solid-colour 64×48 "photos" keyed by the model's real photo ids — no decoder involved;
/// the screen paints what the app supplies.
fn bank(model: &Model, colors: &[(u8, u8, u8)]) -> HashMap<u64, Arc<egui::ColorImage>> {
    let mut bank = HashMap::new();
    for (index, photo) in model.photos().iter().enumerate() {
        let (red, green, blue) = colors[index % colors.len()];
        let rgba: Vec<u8> = [red, green, blue, 255].repeat(64 * 48);
        bank.insert(photo.meta.id.0, Arc::new(egui::ColorImage::from_rgba_unmultiplied([64, 48], &rgba)));
    }
    bank
}

#[test]
fn the_culling_screen_shows_the_strip_and_rates_through_keys() {
    let (_sessions, _folder, model) = shoot();
    let bank = bank(&model, &[(160, 32, 32), (32, 96, 160), (32, 160, 96)]);
    // A deterministic hump for all four channels, so the Info panel's histogram is painted
    // from real data in the snapshot instead of the "histogram…" placeholder.
    let mut histogram = Histogram::empty();
    for index in 0..64 {
        let hump = |centre: f32| (1.0 - (index as f32 - centre).abs() / 64.0).max(0.02);
        histogram.luma[index] = hump(30.0);
        histogram.red[index] = hump(20.0);
        histogram.green[index] = hump(30.0);
        histogram.blue[index] = hump(40.0);
    }
    // The harness keeps the paint closure for its whole life, so the state it paints lives
    // behind cells: borrowed exclusively for the length of one frame, read in slices after.
    let state = std::cell::RefCell::new(model);
    let screen = std::cell::RefCell::new(CullScreen::new());

    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 760.0)).build_ui(|ui| {
        let mut model = state.borrow_mut();
        let mut screen = screen.borrow_mut();
        let current_id = model.current_photo_id();
        let current = current_id.and_then(|id| bank.get(&id.0).map(|pixels| (id, pixels.as_ref())));
        let images = FrameImages { current, thumbs: &bank, histogram: Some(&histogram), display: &bank };
        screen.show(ui, &mut model, &images);
    });

    harness.run();
    {
        let model = state.borrow();
        assert_eq!(model.phase(), firstcut_engine::model::Phase::Culling);
    }
    let first = state.borrow().current_photo_id().map(|id| id.0);
    harness.snapshot("culling");

    // The keyboard is the dispatch path: rates the photo under the cursor, then moves.
    harness.key_press(egui::Key::Num4);
    harness.key_press(egui::Key::ArrowRight);
    harness.run();

    {
        let model = state.borrow();
        assert_eq!(model.current_photo_index(), 1, "the arrow moved the cursor");
        let photo = model.photos().iter().find(|photo| Some(photo.meta.id.0) == first).unwrap();
        assert_eq!(photo.rating.stars, 4, "the 4 key rated the first photo");
    }
    harness.snapshot("culling-after-rating");

    // The Info panel is view state toggled from the keyboard (v1's I); the histogram rides in
    // from the app's decode frame, already orientation-safe.
    harness.key_press(egui::Key::I);
    harness.run();
    harness.snapshot("culling-info-panel");

    // The toolbar switches the mode through the registry; in Keep mode P means keep/not-keep,
    // not the pick flag.
    harness.get_by_label("Keep").click();
    harness.run();
    {
        let model = state.borrow();
        assert_eq!(model.settings().rating_mode, firstcut_types::RatingMode::KeepNotKeep);
        assert!(!model.current_photo().unwrap().rating.keep, "the current photo starts not kept");
    }
    harness.key_press(egui::Key::P);
    harness.run();
    {
        let model = state.borrow();
        assert!(model.current_photo().unwrap().rating.keep, "P kept the current photo in Keep mode");
    }
    harness.snapshot("culling-keep-mode");

    // G is the grid; a click in the contact sheet moves the cursor through the same registry
    // walk the filmstrip uses.
    harness.key_press(egui::Key::G);
    harness.run();
    harness.snapshot("culling-grid");
    harness.get_by_label("grid: IMG_0001.CR3").click();
    harness.run();
    {
        let model = state.borrow();
        assert_eq!(model.current_photo_index(), 0, "the grid click moved the cursor");
    }

    // Compare 2-up from the grid: the batch's frames side by side with shared zoom/pan.
    harness.key_press(egui::Key::C);
    harness.run();
    harness.snapshot("culling-compare");

    // The cheat sheet is generated from the registry; the File menu's Open dialog leaves a
    // request for the app rather than opening a session in the screen.
    harness.key_press(egui::Key::Questionmark);
    harness.run();
    harness.snapshot("culling-shortcuts");
    harness.key_press(egui::Key::Questionmark);
    harness.run();
}

/// The clipping overlay (J) is painted from the display pixels the loupe already has: a white
/// frame is entirely at the highlight threshold, so every sampled cell is marked. The thresholds
/// themselves come from the Viewer settings (250/255 and 5/255 by default).
#[test]
fn the_clipping_overlay_marks_highlight_clipped_pixels() {
    let (_sessions, _folder, model) = shoot();
    let bank = bank(&model, &[(255, 255, 255)]);
    let state = std::cell::RefCell::new(model);
    let screen = std::cell::RefCell::new(CullScreen::new());

    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 760.0)).build_ui(|ui| {
        let mut model = state.borrow_mut();
        let mut screen = screen.borrow_mut();
        let current_id = model.current_photo_id();
        let current = current_id.and_then(|id| bank.get(&id.0).map(|pixels| (id, pixels.as_ref())));
        let images = FrameImages { current, thumbs: &bank, histogram: None, display: &bank };
        screen.show(ui, &mut model, &images);
    });

    harness.run();
    harness.key_press(egui::Key::J);
    harness.run();
    harness.snapshot("culling-clipping");
}

/// File → Open Folder… is a typed path the app opens asynchronously. egui's TextEdit does not
/// expose a WidgetInfo label to the accessibility tree, so this one snapshot disables the
/// unnamed-input check; the dialog's buttons and the rest of the screen are still named.
#[test]
fn the_open_folder_dialog_takes_a_path() {
    let (_sessions, _folder, model) = shoot();
    let bank = bank(&model, &[(160, 32, 32)]);
    let state = std::cell::RefCell::new(model);
    let screen = std::cell::RefCell::new(CullScreen::new());

    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 760.0)).with_accessibility_check(false).build_ui(|ui| {
        let mut model = state.borrow_mut();
        let mut screen = screen.borrow_mut();
        let current_id = model.current_photo_id();
        let current = current_id.and_then(|id| bank.get(&id.0).map(|pixels| (id, pixels.as_ref())));
        let images = FrameImages { current, thumbs: &bank, histogram: None, display: &bank };
        screen.show(ui, &mut model, &images);
    });

    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::O);
    harness.run();
    harness.snapshot("open-folder-dialog");
}

/// The Welcome screen lists the shoots the sessions directory already knows and leaves an open
/// request when one is chosen; the engine owns the open, so the screen never touches a Session.
#[test]
fn the_welcome_screen_lists_recent_shoots_and_asks_to_resume() {
    let (sessions, _root, _opened) = shoot();
    let mut model = Model::new();
    model.attach_sessions_dir(sessions.path());
    assert_eq!(model.phase(), firstcut_engine::model::Phase::Welcome);
    assert_eq!(model.recents().len(), 1);
    let state = std::cell::RefCell::new(model);
    let screen = std::cell::RefCell::new(CullScreen::new());
    let empty = HashMap::new();

    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 760.0)).build_ui(|ui| {
        let mut model = state.borrow_mut();
        let mut screen = screen.borrow_mut();
        let images = FrameImages { current: None, thumbs: &empty, histogram: None, display: &empty };
        screen.show(ui, &mut model, &images);
    });

    harness.run();
    harness.snapshot("welcome-recents");
    harness.get_by_label("Game1JENKS   ·   3 photos").click();
    harness.run();
    let requested = screen.borrow_mut().take_open_request().expect("choosing a recent asks the app to open it");
    assert!(requested.ends_with("Game1JENKS"), "the request is the shoot folder, got {}", requested.display());
}

/// ⇧X asks before it flags a whole batch, and the confirmation dispatches the engine's single
/// command; the setting gates the key (off by default), so the dialog cannot appear unexpectedly.
#[test]
fn the_whole_batch_reject_asks_first_and_flags_every_frame() {
    let (_sessions, _root, mut model) = shoot();
    model.set_reject_whole_batch(true);
    let state = std::cell::RefCell::new(model);
    let screen = std::cell::RefCell::new(CullScreen::new());
    let bank = bank(&state.borrow(), &[(160, 32, 32), (32, 96, 160), (32, 160, 96)]);

    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 760.0)).build_ui(|ui| {
        let mut model = state.borrow_mut();
        let mut screen = screen.borrow_mut();
        let current_id = model.current_photo_id();
        let current = current_id.and_then(|id| bank.get(&id.0).map(|pixels| (id, pixels.as_ref())));
        let images = FrameImages { current, thumbs: &bank, histogram: None, display: &bank };
        screen.show(ui, &mut model, &images);
    });

    harness.run();
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::X);
    harness.run();
    harness.snapshot("reject-batch-dialog");
    assert!(state.borrow().photos().iter().all(|photo| photo.rating.flag != firstcut_types::Flag::Reject), "opening the dialog changes nothing");

    harness.get_by_label("Reject Batch").click();
    harness.run();
    assert!(state.borrow().photos().iter().all(|photo| photo.rating.flag == firstcut_types::Flag::Reject), "confirming flags the batch");
}
