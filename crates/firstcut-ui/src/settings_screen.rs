//! Settings → General / Keyboard / Viewer / Metadata / Performance (todo.md §9.7).
//!
//! A window over whichever phase is showing, opened with ⌘, or File → Settings…, the same place
//! v1 put it. Every control applies immediately and persists through the model's setters, which
//! write `settings.json` in one temp-file-and-rename; there is no OK/Cancel, exactly like v1's
//! live settings. The engine owns every value and every save — this module only draws the shapes
//! and calls the setter for the half it changed.
//!
//! Keyboard is the keymap editor: click a chord to record a new one, or import/export the file.
//! The map itself lives in `firstcut-engine::keymap`; this screen only asks it for what is bound
//! and hands the recorded chord back through the model.

use std::path::PathBuf;

use egui::{Color32, RichText};
use firstcut_engine::model::Model;
use firstcut_engine::registry::REGISTRY;
use firstcut_engine::settings::{InfoFieldJson, KeepMappingJson, MetadataJson, PerformanceJson, XmpWriteModeJson};
use firstcut_types::RatingMode;

use crate::cull::{VIEW_COMMANDS, chord_from_event, command_description};
use crate::finish_sheet::{KeptKind, UnkeptKind, kept_kind_of, kept_path, unkept_folder, unkept_kind_of};

const FOREGROUND: Color32 = Color32::from_rgb(232, 232, 236);
const ACCENT: Color32 = Color32::from_rgb(244, 200, 0);

/// The declaration order of `InfoFieldJson` (v1's Set is written back in this order, not a hash
/// order). The checkboxes follow it too, so the screen and the file agree.
const INFO_FIELDS: [InfoFieldJson; 20] = [
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
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SettingsTab {
    #[default]
    General,
    Keyboard,
    Viewer,
    Metadata,
    Performance,
}

impl SettingsTab {
    const ALL: [SettingsTab; 5] = [SettingsTab::General, SettingsTab::Keyboard, SettingsTab::Viewer, SettingsTab::Metadata, SettingsTab::Performance];

    fn title(self) -> &'static str {
        match self {
            SettingsTab::General => "General",
            SettingsTab::Keyboard => "Keyboard",
            SettingsTab::Viewer => "Viewer",
            SettingsTab::Metadata => "Metadata",
            SettingsTab::Performance => "Performance",
        }
    }
}

/// One command waiting for the next chord. `label` is what the banner names.
struct Recording {
    command: String,
    argument: Option<i64>,
    label: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeymapDialog {
    Import,
    Export,
}

/// The window's own state: visible, which tab, and the recorder/dialog in flight. Every value it
/// shows comes from the model each frame, so an external change (the CLI, a menu action) cannot
/// leave it stale.
pub(crate) struct SettingsScreen {
    pub(crate) visible: bool,
    tab: SettingsTab,
    recording: Option<Recording>,
    keymap_dialog: Option<KeymapDialog>,
    keymap_path: String,
    status: Option<String>,
}

impl Default for SettingsScreen {
    fn default() -> Self {
        Self { visible: false, tab: SettingsTab::General, recording: None, keymap_dialog: None, keymap_path: String::new(), status: None }
    }
}

impl SettingsScreen {
    /// True while a chord is being recorded: the window must not close on the Escape that
    /// cancels it, and the culling screen must not route the key that is being recorded.
    pub(crate) fn is_capturing(&self) -> bool {
        self.recording.is_some()
    }

    pub(crate) fn show(&mut self, ctx: &egui::Context, model: &mut Model) {
        if !self.visible {
            self.recording = None;
            self.keymap_dialog = None;
            return;
        }

        // The recorder reads the frame's key events before anything else; a key event cannot
        // also reach the culling screen (route_keys is gated on Settings being visible).
        if self.recording.is_some() {
            let pressed = ctx.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, modifiers)) = pressed {
                if key == egui::Key::Escape {
                    self.recording = None;
                    self.status = Some("Recording cancelled".to_string());
                } else if let Some(chord) = chord_from_event(key, modifiers)
                    && let Some(recording) = self.recording.take()
                {
                    self.status = Some(match model.rebind_keymap(&recording.command, recording.argument, &chord) {
                        Ok(()) => format!("{} is now {}", recording.label, chord.display()),
                        Err(err) => err,
                    });
                }
            }
        }

        let mut open = self.visible;
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(false)
            .default_width(660.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |window| {
                window.horizontal(|row| {
                    for tab in SettingsTab::ALL {
                        if row.selectable_label(self.tab == tab, tab.title()).clicked() {
                            self.tab = tab;
                        }
                    }
                });
                window.separator();
                egui::ScrollArea::vertical().max_height(440.0).auto_shrink([false, true]).show(window, |body| match self.tab {
                    SettingsTab::General => general_tab(body, model),
                    SettingsTab::Keyboard => self.keyboard_tab(body, model),
                    SettingsTab::Viewer => viewer_tab(body, model),
                    SettingsTab::Metadata => metadata_tab(body, model),
                    SettingsTab::Performance => performance_tab(body, model),
                });
            });
        if !open {
            self.recording = None;
            self.keymap_dialog = None;
        }
        self.visible = open;
        self.keymap_dialog_window(ctx, model);
    }

    /// The Keyboard tab: one row per live binding, plus a row for every command that has no
    /// key yet. Clicking the chord (or "Record") arms the recorder; the next non-modifier key
    /// becomes the new chord. Escape cancels without changing anything.
    fn keyboard_tab(&mut self, body: &mut egui::Ui, model: &mut Model) {
        body.horizontal(|row| {
            if row.button("Import…").clicked() {
                self.keymap_path = model.keymap_path().map(|path| path.display().to_string()).unwrap_or_default();
                self.keymap_dialog = Some(KeymapDialog::Import);
            }
            if row.button("Export…").clicked() {
                self.keymap_path =
                    model.keymap_path().map(|path| path.with_file_name("keymap-export.json").display().to_string()).unwrap_or_default();
                self.keymap_dialog = Some(KeymapDialog::Export);
            }
            if row.button("Reset All").clicked() {
                self.status = Some(match model.reset_keymap() {
                    Ok(()) => "Every shortcut is back to its default".to_string(),
                    Err(err) => err,
                });
            }
        });
        body.add_space(4.0);
        body.label(
            RichText::new("Click a shortcut to record a new one. Changes apply immediately and are written to your keymap.json; a key already in use is taken over.")
                .small()
                .weak(),
        );
        if let Some(recording) = &self.recording {
            body.add_space(4.0);
            body.label(RichText::new(format!("Press the new shortcut for {} — Escape cancels.", recording.label)).color(ACCENT));
        } else if let Some(status) = &self.status {
            body.add_space(4.0);
            body.label(RichText::new(status).small().weak());
        }
        body.separator();

        struct Row {
            command: String,
            argument: Option<i64>,
            label: String,
            chord: Option<String>,
        }

        let mut rows: Vec<Row> = Vec::new();
        let mut seen: Vec<(String, Option<i64>)> = Vec::new();
        for binding in model.keymap().bindings.clone() {
            let mut label = binding_label(&binding.command, binding.argument);
            if let Some(modes) = &binding.modes {
                let names = modes
                    .iter()
                    .map(|mode| match mode {
                        firstcut_engine::keymap::ModeName::Stars => "Stars",
                        firstcut_engine::keymap::ModeName::Keep => "Keep",
                    })
                    .collect::<Vec<_>>()
                    .join(" / ");
                label.push_str(&format!(" — {names} mode"));
            }
            seen.push((binding.command.clone(), binding.argument));
            let chord = Some(binding.chord().display());
            rows.push(Row { command: binding.command, argument: binding.argument, label, chord });
        }
        for (id, description) in REGISTRY.iter().map(|command| (command.id, command.description)).chain(VIEW_COMMANDS.iter().copied()) {
            if id.starts_with("engine.") || seen.iter().any(|(command, _)| command == id) {
                continue;
            }
            // A command that takes an argument cannot be offered without one; the bindings
            // above already carry every argument the shipped map uses.
            if REGISTRY.iter().any(|command| command.id == id && !command.params.is_empty()) {
                continue;
            }
            rows.push(Row { command: id.to_string(), argument: None, label: description.to_string(), chord: None });
        }

        let mut record: Option<(String, Option<i64>, String)> = None;
        for row in rows {
            body.horizontal_wrapped(|line| {
                let text = row.chord.clone().unwrap_or_else(|| "Record".to_string());
                let button = egui::Button::new(RichText::new(text).monospace().small().color(ACCENT));
                if line.add(button).on_hover_text("Click to record a new shortcut").clicked() {
                    record = Some((row.command.clone(), row.argument, row.label.clone()));
                }
                line.add(egui::Label::new(RichText::new(row.label.clone()).small().color(FOREGROUND)).wrap());
            });
            body.add_space(2.0);
        }
        if let Some((command, argument, label)) = record {
            self.recording = Some(Recording { command, argument, label });
            self.status = None;
        }
    }

    fn keymap_dialog_window(&mut self, ctx: &egui::Context, model: &mut Model) {
        let Some(dialog) = self.keymap_dialog else {
            return;
        };
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;
        let title = match dialog {
            KeymapDialog::Import => "Import Keymap",
            KeymapDialog::Export => "Export Keymap",
        };
        egui::Window::new(title).collapsible(false).resizable(false).open(&mut open).show(ctx, |window| {
            window.label(
                RichText::new(match dialog {
                    KeymapDialog::Import => "A keymap.json from another Firstcut, or an exported copy.",
                    KeymapDialog::Export => "Writes the shortcuts this app actually runs, defaults included.",
                })
                .small()
                .weak(),
            );
            window.add(egui::TextEdit::singleline(&mut self.keymap_path).desired_width(420.0));
            window.horizontal(|row| {
                if row
                    .button(match dialog {
                        KeymapDialog::Import => "Import",
                        KeymapDialog::Export => "Export",
                    })
                    .clicked()
                {
                    submit = true;
                }
                if row.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if submit {
            let path = PathBuf::from(self.keymap_path.trim());
            let outcome = match dialog {
                KeymapDialog::Import => model.import_keymap(&path),
                KeymapDialog::Export => model.export_keymap(&path),
            };
            match outcome {
                Ok(()) => {
                    let verb = match dialog {
                        KeymapDialog::Import => "Imported",
                        KeymapDialog::Export => "Exported",
                    };
                    self.status = Some(format!("{verb} {}", path.display()));
                    self.keymap_dialog = None;
                    return;
                }
                Err(err) => self.status = Some(err),
            }
        }
        if cancel || !open {
            self.keymap_dialog = None;
        }
    }
}

fn section(body: &mut egui::Ui, title: &str) {
    body.add_space(6.0);
    body.label(RichText::new(title).strong().color(ACCENT));
    body.add_space(2.0);
}

fn general_tab(body: &mut egui::Ui, model: &mut Model) {
    let mut general = model.settings().clone();
    let mut changed = false;

    section(body, "RATING");
    body.horizontal(|row| {
        changed |= row.radio_value(&mut general.rating_mode, RatingMode::Stars, "Stars").changed();
        changed |= row.radio_value(&mut general.rating_mode, RatingMode::KeepNotKeep, "Keep / Not keep").changed();
    });
    body.add_space(4.0);
    body.horizontal(|row| {
        row.label("A keep in stars mode is");
        changed |= row.radio_value(&mut general.keep_stars, 4u8, "4 stars or more").changed();
        changed |= row.radio_value(&mut general.keep_stars, 5u8, "5 stars").changed();
    });

    section(body, "NAVIGATION");
    changed |= body.checkbox(&mut general.auto_advance, "Auto-advance after every rating change").changed();
    body.horizontal(|row| {
        row.label("At the end of a batch");
        changed |= row
            .radio_value(&mut general.arrow_behavior, firstcut_engine::model::ArrowBehavior::ContinueIntoNextBatch, "Continue into the next")
            .changed();
        changed |= row.radio_value(&mut general.arrow_behavior, firstcut_engine::model::ArrowBehavior::Stop, "Stop").changed();
    });
    body.horizontal(|row| {
        row.label("Entering a batch");
        changed |=
            row.radio_value(&mut general.entering_behavior, firstcut_engine::model::EnteringBatchBehavior::FirstPhoto, "First photo").changed();
        changed |=
            row.radio_value(&mut general.entering_behavior, firstcut_engine::model::EnteringBatchBehavior::LastViewed, "Last viewed").changed();
    });

    section(body, "SAFETY");
    changed |= body.checkbox(&mut general.reject_whole_batch, "Allow rejecting a whole batch (Shift+X asks first)").changed();

    section(body, "FINISH CULL");
    changed |= body.checkbox(&mut general.confirm_before_finish, "Confirm before opening the Finish sheet").changed();
    changed |= body.checkbox(&mut general.confirm_permanent_delete, "Ask for the typed word when a Finish run deletes permanently").changed();
    body.add_space(4.0);

    // Defaults for the sheet's own choices. Changing one here never changes an open sheet; the
    // sheet seeds from these when it opens, exactly like v1's `showFinishOptions`.
    body.label(RichText::new("Not-kept photos").small().weak());
    let mut unkept_kind = unkept_kind_of(&general.finish_unkept);
    let mut folder = unkept_folder(&general.finish_unkept);
    for (kind, _) in
        [(UnkeptKind::MarkRejected, ""), (UnkeptKind::Subfolder, ""), (UnkeptKind::Trash, ""), (UnkeptKind::Delete, ""), (UnkeptKind::Nothing, "")]
    {
        changed |= body.radio_value(&mut unkept_kind, kind, kind.title()).changed();
    }
    if unkept_kind == UnkeptKind::Subfolder {
        body.horizontal(|row| {
            row.label("Subfolder");
            changed |= row.add(egui::TextEdit::singleline(&mut folder).hint_text("_Not kept").desired_width(220.0)).changed();
        });
    }
    if unkept_kind != unkept_kind_of(&general.finish_unkept) || folder != unkept_folder(&general.finish_unkept) {
        general.finish_unkept = unkept_kind.apply(&folder);
        changed = true;
    }

    body.add_space(4.0);
    body.label(RichText::new("Kept photos").small().weak());
    let mut kept_kind = kept_kind_of(&general.finish_kept);
    let mut path = kept_path(&general.finish_kept);
    for (kind, _) in [
        (KeptKind::None, ""),
        (KeptKind::Copy, ""),
        (KeptKind::Move, ""),
        (KeptKind::SplitTier, ""),
        (KeptKind::SplitStars, ""),
        (KeptKind::List, ""),
    ] {
        changed |= body.radio_value(&mut kept_kind, kind, kind.title()).changed();
    }
    if kept_kind != KeptKind::None {
        body.horizontal(|row| {
            row.label("Destination");
            changed |= row.add(egui::TextEdit::singleline(&mut path).hint_text("Destination path").desired_width(220.0)).changed();
        });
    }
    if kept_kind != kept_kind_of(&general.finish_kept) || path != kept_path(&general.finish_kept) {
        general.finish_kept = kept_kind.apply(&path);
        changed = true;
    }

    if changed {
        model.set_general_settings(general);
    }
}

fn viewer_tab(body: &mut egui::Ui, model: &mut Model) {
    let mut viewer = model.viewer_settings();
    let mut changed = false;

    section(body, "CANVAS");
    changed |= body.add(egui::Slider::new(&mut viewer.background_gray, 0.0..=0.6).text("Background gray")).changed();
    changed |= body.checkbox(&mut viewer.zoom_lock, "Zoom lock (keep the loupe's pixels while stepping)").changed();
    changed |= body.checkbox(&mut viewer.hud_visible, "Show the progress HUD on open").changed();

    section(body, "OVERLAYS");
    changed |= body.checkbox(&mut viewer.af_overlay, "Show the AF point by default").changed();
    changed |= body.add(egui::Slider::new(&mut viewer.clipping_shadow_threshold, 0.0..=0.2).text("Clipped-shadow threshold")).changed();
    changed |= body.add(egui::Slider::new(&mut viewer.clipping_highlight_threshold, 0.8..=1.0).text("Clipped-highlight threshold")).changed();
    body.label(RichText::new("v1's defaults: 5/255 for shadows, 250/255 for highlights.").small().weak());

    section(body, "INFO PANEL FIELDS");
    for field in INFO_FIELDS {
        let mut on = viewer.info_fields.contains(&field);
        if body.checkbox(&mut on, field_label(field)).changed() {
            if on && !viewer.info_fields.contains(&field) {
                viewer.info_fields.push(field);
            } else if !on {
                viewer.info_fields.retain(|existing| *existing != field);
            }
            // Keep the file in declaration order; a v1 Set comes back that way.
            viewer.info_fields.sort_by_key(|existing| INFO_FIELDS.iter().position(|candidate| candidate == existing).unwrap_or(usize::MAX));
            changed = true;
        }
    }

    section(body, "DEVELOP");
    changed |= body.checkbox(&mut viewer.exact_raw, "Exact RAW: develop the sensor data instead of the embedded preview").changed();

    if changed {
        model.set_viewer_settings(viewer);
    }
}

fn metadata_tab(body: &mut egui::Ui, model: &mut Model) {
    let mut metadata: MetadataJson = model.metadata_settings();
    let mut changed = false;

    section(body, "XMP");
    changed |= body.checkbox(&mut metadata.write_xmp, "Write ratings and labels into XMP sidecars").changed();
    changed |= body.checkbox(&mut metadata.write_sidecars_for_jpegs, "Also write sidecars next to JPEG/HEIF files").changed();

    body.horizontal(|row| {
        row.label("A keep is written as");
        let rating = matches!(metadata.keep_mapping, KeepMappingJson::Rating { .. });
        if row.radio(!rating, "A colour label").clicked() {
            metadata.keep_mapping = KeepMappingJson::ColorLabelVariant { label: firstcut_engine::settings::ColorLabelJson::Red };
            changed = true;
        }
        if row.radio(rating, "1 star").clicked() {
            metadata.keep_mapping = KeepMappingJson::Rating { stars: 1 };
            changed = true;
        }
    });

    body.horizontal(|row| {
        row.label("Existing sidecar");
        if row.radio(metadata.xmp_write_mode == XmpWriteModeJson::Merge, "Merge into it").clicked() {
            metadata.xmp_write_mode = XmpWriteModeJson::Merge;
            changed = true;
        }
        if row.radio(metadata.xmp_write_mode == XmpWriteModeJson::Overwrite, "Overwrite it").clicked() {
            metadata.xmp_write_mode = XmpWriteModeJson::Overwrite;
            changed = true;
        }
    });

    section(body, "DNG");
    changed |= body.checkbox(&mut metadata.write_ratings_into_dng, "Write ratings into DNG originals (modifies the original file)").changed();
    body.label(RichText::new("Off by default. Originals are otherwise never opened for writing.").small().weak());

    if changed {
        model.set_metadata_settings(metadata);
    }
}

fn performance_tab(body: &mut egui::Ui, model: &mut Model) {
    let mut performance: PerformanceJson = model.performance_settings();
    let mut changed = false;

    section(body, "MEMORY");
    changed |=
        body.add(egui::Slider::new(&mut performance.memory_budget_fraction, 0.1..=0.9).text("Decode budget (share of physical RAM)")).changed();

    section(body, "DECODE");
    changed |= body.add(egui::Slider::new(&mut performance.look_ahead_batches, 0..=6).text("Batches decoded ahead")).changed();
    changed |= body.add(egui::Slider::new(&mut performance.thumbnail_pixels, 128..=1024).step_by(64.0).text("Filmstrip thumbnail pixels")).changed();
    changed |= body.add(egui::Slider::new(&mut performance.decode_threads, 0..=16).text("Decode threads (0 = measure the machine)")).changed();
    changed |= body.checkbox(&mut performance.debug_hud, "Debug HUD (frame timings and memory)").changed();
    body.label(RichText::new("The decode bank takes a new budget the next time a shoot opens.").small().weak());

    if changed {
        model.set_performance_settings(performance);
    }
}

/// What a binding row says: the command's description, but arguments spelled out where the
/// registry's generic text (`Set the current photo's stars (0..=5)`) would be vague.
fn binding_label(command: &str, argument: Option<i64>) -> String {
    match (command, argument) {
        ("rate.stars", Some(0)) => "Clear the rating".to_string(),
        ("rate.stars", Some(stars)) => format!("Rate {stars} stars"),
        ("rate.starsAndAdvance", Some(stars)) => format!("Rate {stars} stars and advance"),
        ("label.set", Some(6)) => "Colour label: red".to_string(),
        ("label.set", Some(7)) => "Colour label: yellow".to_string(),
        ("label.set", Some(8)) => "Colour label: green".to_string(),
        ("label.set", Some(9)) => "Colour label: blue".to_string(),
        _ => match (command_description(command), argument) {
            (Some(description), Some(argument)) => format!("{description}  ({argument})"),
            (Some(description), None) => description.to_string(),
            (None, _) => format!("{command} (not in this build)"),
        },
    }
}

/// Human labels for Settings → Viewer; the enum's Debug name is not what v1 showed.
fn field_label(field: InfoFieldJson) -> &'static str {
    match field {
        InfoFieldJson::FileName => "File name",
        InfoFieldJson::CaptureTime => "Capture time",
        InfoFieldJson::Camera => "Camera",
        InfoFieldJson::Lens => "Lens",
        InfoFieldJson::FocalLength => "Focal length",
        InfoFieldJson::Shutter => "Shutter",
        InfoFieldJson::Aperture => "Aperture",
        InfoFieldJson::Iso => "ISO",
        InfoFieldJson::ExposureCompensation => "Exposure compensation",
        InfoFieldJson::Metering => "Metering",
        InfoFieldJson::AfMode => "AF mode",
        InfoFieldJson::DriveMode => "Drive mode",
        InfoFieldJson::ShutterMode => "Shutter mode",
        InfoFieldJson::ShutterCount => "Shutter count",
        InfoFieldJson::Dimensions => "Dimensions",
        InfoFieldJson::FileSize => "File size",
        InfoFieldJson::FolderPath => "Folder path",
        InfoFieldJson::Rating => "Rating",
        InfoFieldJson::Batch => "Batch",
        InfoFieldJson::Histogram => "Histogram",
    }
}
