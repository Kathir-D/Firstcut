//! Settings → General / Keyboard / Viewer / Metadata / Performance (todo.md §9.7).
//!
//! A window over whichever phase is showing, opened with ⌘, or File → Settings…, the same place
//! v1 put it. Every control applies immediately and persists through the model's setters, which
//! write `settings.json` in one temp-file-and-rename; there is no OK/Cancel, exactly like v1's
//! live settings. The engine owns every value and every save — this module only draws the shapes
//! and calls the setter for the half it changed.
//!
//! Keyboard is a read-only list until the keymap engine lands; the recorder, import and export
//! replace it there, not here.

use egui::{Color32, RichText, vec2};
use firstcut_engine::model::Model;
use firstcut_engine::registry::REGISTRY;
use firstcut_engine::settings::{InfoFieldJson, KeepMappingJson, MetadataJson, PerformanceJson, XmpWriteModeJson};
use firstcut_types::RatingMode;

use crate::cull::shortcut_for;

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

/// The window's own state: visible, which tab, nothing else. Every value it shows comes from the
/// model each frame, so an external change (the CLI, a menu action) cannot leave it stale.
pub(crate) struct SettingsScreen {
    pub(crate) visible: bool,
    tab: SettingsTab,
}

impl Default for SettingsScreen {
    fn default() -> Self {
        Self { visible: false, tab: SettingsTab::General }
    }
}

impl SettingsScreen {
    pub(crate) fn show(&mut self, ctx: &egui::Context, model: &mut Model) {
        if !self.visible {
            return;
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
                    SettingsTab::Keyboard => keyboard_tab(body),
                    SettingsTab::Viewer => viewer_tab(body, model),
                    SettingsTab::Metadata => metadata_tab(body, model),
                    SettingsTab::Performance => performance_tab(body, model),
                });
            });
        self.visible = open;
    }
}

fn section(body: &mut egui::Ui, title: &str) {
    body.add_space(6.0);
    body.label(RichText::new(title).strong().color(ACCENT));
    body.add_space(2.0);
}

fn general_tab(body: &mut egui::Ui, model: &mut Model) {
    let mut general = *model.settings();
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

fn keyboard_tab(body: &mut egui::Ui) {
    body.label(RichText::new("Every command in the registry and its bound key.").weak());
    body.add_space(4.0);
    for command in REGISTRY {
        body.horizontal_wrapped(|row| {
            row.add_sized(vec2(120.0, 16.0), egui::Label::new(RichText::new(shortcut_for(command.id)).monospace().small().color(ACCENT)));
            row.label(RichText::new(command.description).small().color(FOREGROUND));
        });
        body.add_space(2.0);
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
