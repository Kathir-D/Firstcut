//! The culling screen — the M1 vertical slice's surface: filmstrip + loupe + the keyboard.
//!
//! Every action here goes through [`dispatch`]: the keyboard and the filmstrip are just another
//! pair of command origins, indistinguishable from the CLI and the control channel as far as the
//! model is concerned (app-model.md). The screen reads the model freely — ids, cursor, ratings —
//! and never writes a field itself.
//!
//! Pixels are not this crate's business: [`FrameImages`] delivers decoded photos to paint, and
//! the app decides what decodes and when. What the screen owns is the texture cache (a pass
//! through `ctx.load_texture`, so an unchanged photo costs nothing) and the zoom/pan the user is
//! in the middle of.

use std::collections::HashMap;

use egui::{Color32, FontId, Image, Key, RichText, Sense, Stroke, TextureHandle, Vec2, pos2, vec2};
use firstcut_engine::model::{Model, Phase};
use firstcut_engine::registry::dispatch;
use firstcut_types::{Flag, Histogram, Rating, RatingMode, Tier};
use serde_json::{Value as Json, json};

use crate::FrameImages;

/// v1's viewer background (Settings → Viewer → backgroundGray): 0.12 on the grey ramp.
const BACKGROUND: Color32 = Color32::from_rgb(31, 31, 31);
/// The strip sits a step darker than the loupe's area, the way v1 layered it.
const STRIP: Color32 = Color32::from_rgb(21, 21, 21);
const FOREGROUND: Color32 = Color32::from_rgb(232, 232, 236);
/// The cursor's outline on the filmstrip (v1's selection accent).
const ACCENT: Color32 = Color32::from_rgb(244, 200, 0);
const THUMB_WIDTH: f32 = 128.0;
/// Each filmstrip frame's image area; the file name and stars go underneath it.
const THUMB_IMAGE_HEIGHT: f32 = 78.0;
/// One grid cell's outer width; the image inside keeps a 3:2 box.
const GRID_WIDTH: f32 = 168.0;

/// What the area above the filmstrip shows. v1's `ViewMode` narrowed to the two screens that
/// exist today; compare joins with its own row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScreenView {
    Loupe,
    Grid,
    /// 2-, 3- or 4-up compare of the current batch.
    Compare(usize),
}

pub struct CullScreen {
    /// Decoded pixels → GPU textures, keyed by photo id, so an unchanged frame reuses them all.
    /// The pixel size is kept beside the handle: zooming asks the decode pipeline for a bigger
    /// copy of the same photo, and a size change must rebuild the texture.
    textures: HashMap<u64, (usize, usize, TextureHandle)>,
    /// The loupe's zoom, relative to the fit-to-pane size (1.0 = the whole photo shown).
    zoom: f32,
    /// The loupe's pan while zoomed in; the pane centres the photo at rest.
    pan: Vec2,
    /// The viewer's backing size from the last frame, in physical pixels — the display tier's
    /// decode target. Zero until the pane has been laid out.
    viewport_pixels: Vec2,
    /// The progress HUD's visibility (v1's `hudVisible`, toggled with H).
    hud_visible: bool,
    /// The Info panel's visibility (v1's `infoPanelVisible`, toggled with I).
    info_visible: bool,
    /// The photo the loupe framed last, so a move can reset zoom unless zoom lock is on.
    last_photo: Option<u64>,
    /// Loupe or grid (G toggles; Escape returns to the loupe).
    view: ScreenView,
    /// The AF overlay's session toggle; `None` means "use the Viewer setting".
    af_overlay: Option<bool>,
    /// The clipping overlay's session toggle (J), off by default like v1.
    clipping_overlay: bool,
}

impl Default for CullScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl CullScreen {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            zoom: 1.0,
            pan: Vec2::ZERO,
            viewport_pixels: Vec2::ZERO,
            hud_visible: true,
            info_visible: false,
            last_photo: None,
            view: ScreenView::Loupe,
            af_overlay: None,
            clipping_overlay: false,
        }
    }

    /// The viewer's long edge in backing pixels, or 0 before the first layout. The app turns
    /// this into a display-tier decode target.
    pub fn viewport_edge(&self) -> u32 {
        let edge = self.viewport_pixels.x.max(self.viewport_pixels.y);
        if edge.is_finite() && edge > 0.0 { edge.round().min(u32::MAX as f32) as u32 } else { 0 }
    }

    /// Whether the loupe is showing pixels beyond fit-to-pane (the 1:1 tier).
    pub fn is_zoomed(&self) -> bool {
        self.zoom > 1.01
    }

    /// Paints one frame. Takes the model mutable because dispatching is this surface's only way
    /// to act, and the keyboard is dispatching.
    pub fn show(&mut self, ui: &mut egui::Ui, model: &mut Model, images: &FrameImages<'_>) {
        // Photos that left the shoot no longer need their GPU memory; a rescan can drop hundreds.
        let known: Vec<u64> = model.photos().iter().map(|photo| photo.meta.id.0).collect();
        self.textures.retain(|id, _| known.contains(id));

        match model.phase() {
            Phase::Welcome => self.show_welcome(ui),
            // The fast path paints the placeholder through the same culling screen; dispatch
            // refuses actions until the scan finishes (`canAct` is false).
            Phase::Loading | Phase::Culling => self.show_culling(ui, model, images),
        }
    }

    fn show_welcome(&self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BACKGROUND)).show(ui, |panel| {
            panel.vertical_centered(|panel| {
                panel.add_space(panel.available_height() / 2.0 - 40.0);
                panel.heading(RichText::new("Firstcut").color(FOREGROUND));
                panel.add_space(8.0);
                panel.label(RichText::new("Open a shoot folder to start culling").weak());
            });
        });
    }

    fn show_culling(&mut self, ui: &mut egui::Ui, model: &mut Model, images: &FrameImages<'_>) {
        // The HUD is view state, not engine state: v1 kept `hudVisible` on the app model and the
        // view toggles it, so no command crosses the registry for it.
        if ui.ctx().input(|input| input.key_pressed(Key::H)) {
            self.hud_visible = !self.hud_visible;
        }
        if ui.ctx().input(|input| input.key_pressed(Key::I)) {
            self.info_visible = !self.info_visible;
        }
        if ui.ctx().input(|input| input.key_pressed(Key::G)) {
            self.view = if self.view == ScreenView::Grid { ScreenView::Loupe } else { ScreenView::Grid };
        }
        if ui.ctx().input(|input| input.key_pressed(Key::C)) {
            // v1's defaults: C 2-up, Option-C 3-up, Option-Shift-C 4-up (todo.md §10).
            let (shift, alt) = ui.ctx().input(|input| (input.modifiers.shift, input.modifiers.alt));
            let count = if shift {
                4
            } else if alt {
                3
            } else {
                2
            };
            if matches!(self.view, ScreenView::Compare(_)) {
                self.view = ScreenView::Loupe;
            } else {
                // Compare starts at fit; its zoom is shared across the panes from there.
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
                self.view = ScreenView::Compare(count);
            }
        }
        if ui.ctx().input(|input| input.key_pressed(Key::Escape) || input.key_pressed(Key::E)) {
            self.view = ScreenView::Loupe;
        }
        // A and J are session toggles (todo.md §10); A starts from the Viewer setting, J off.
        if ui.ctx().input(|input| input.key_pressed(Key::A)) {
            let current = self.af_overlay.unwrap_or_else(|| model.viewer_settings().af_overlay);
            self.af_overlay = Some(!current);
        }
        if ui.ctx().input(|input| input.key_pressed(Key::J)) {
            self.clipping_overlay = !self.clipping_overlay;
        }
        for (id, params) in queued_keys(ui, model.settings().rating_mode) {
            let _ = dispatch(model, id, &params);
        }
        self.toolbar(ui, model);
        match self.view {
            ScreenView::Loupe => {
                self.filmstrip(ui, model, images);
                if self.info_visible {
                    self.info_panel(ui, model, images.histogram);
                }
                self.loupe(ui, model, images);
            }
            ScreenView::Grid => self.grid(ui, model, images),
            ScreenView::Compare(count) => self.compare(ui, model, images, count),
        }
    }

    // ---- compare -------------------------------------------------------------------------

    /// v1's Compare (todo.md §9.6): 2–4 frames of the current batch side by side, one shared
    /// zoom and pan. The window starts at the cursor and clamps so it never runs off the batch;
    /// arrows move the cursor and slide it. A click makes a frame current; a double-click
    /// returns to the loupe.
    fn compare(&mut self, ui: &mut egui::Ui, model: &mut Model, images: &FrameImages<'_>, count: usize) {
        let count = count.clamp(2, 4);
        let batch_count = model.current_batch().map_or(0, |batch| batch.photo_ids.len());
        if batch_count == 0 {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(BACKGROUND)).show(ui, |panel| {
                panel.centered_and_justified(|panel| panel.label(RichText::new("No batch to compare").weak()));
            });
            return;
        }
        let current_index = model.current_photo_index().min(batch_count.saturating_sub(1));
        let start = current_index.min(batch_count.saturating_sub(count));
        let current_id = model.current_photo_id().map(|id| id.0);
        let batch_index = model.current_batch_index();

        // Plain copies: the paint closure only reads, the dispatches come after it.
        let mut panes: Vec<Thumb> = Vec::new();
        if let Some(batch) = model.current_batch() {
            for (photo_index, photo_id) in batch.photo_ids.iter().enumerate().skip(start).take(count) {
                let Some(photo) = model.photos().iter().find(|photo| photo.meta.id == *photo_id) else { continue };
                panes.push(Thumb { id: photo_id.0, batch_index, photo_index, file_name: file_name_of(&photo.meta.rel_path), rating: photo.rating });
            }
        }
        let pane_count = panes.len().max(1) as f32;
        let mut chosen: Option<usize> = None;
        let mut opened = false;

        egui::CentralPanel::default().frame(egui::Frame::new().fill(BACKGROUND).inner_margin(8.0)).show(ui, |panel| {
            let area = panel.max_rect();
            let gap = 6.0;
            let width = ((area.width() - gap * (pane_count - 1.0)) / pane_count).max(1.0);
            let scroll = panel.ctx().input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom = (self.zoom * (1.0 - scroll / 1_500.0)).clamp(0.2, 12.0);
            }
            for (position, pane) in panes.iter().enumerate() {
                let rect = egui::Rect::from_min_size(pos2(area.left() + position as f32 * (width + gap), area.top()), vec2(width, area.height()));
                panel.painter().rect_filled(rect, 4.0, Color32::from_rgb(24, 24, 26));
                let pixels = images
                    .display
                    .get(&pane.id)
                    .map(|pixels| pixels.as_ref())
                    .or_else(|| images.current.filter(|(id, _)| id.0 == pane.id).map(|(_, pixels)| pixels))
                    .or_else(|| images.thumbs.get(&pane.id).map(|pixels| pixels.as_ref()));
                if let Some(pixels) = pixels {
                    let texture = load_texture(&mut self.textures, panel.ctx(), pane.id, pixels);
                    let image_size = vec2(pixels.width() as f32, pixels.height() as f32);
                    let fit = fit_into(rect.size() - vec2(0.0, 22.0), image_size);
                    let display = fit * self.zoom;
                    let image_rect = egui::Rect::from_center_size(rect.center() - vec2(0.0, 8.0) + self.pan, display);
                    let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    panel.painter().with_clip_rect(rect).image(texture.id(), image_rect, uv, Color32::WHITE);
                } else {
                    panel.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "decoding…",
                        FontId::proportional(13.0),
                        Color32::from_gray(120),
                    );
                }
                let name_color = if Some(pane.id) == current_id { ACCENT } else { FOREGROUND };
                panel.painter().text(
                    rect.left_bottom() + vec2(6.0, -5.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{}   {}", pane.file_name, stars_label(&pane.rating)),
                    FontId::proportional(12.0),
                    name_color,
                );

                let response = panel.interact(rect, egui::Id::unique(("compare-pane", position)), Sense::click_and_drag());
                if response.dragged() {
                    self.pan += response.drag_delta();
                    let slack = (rect.size() * self.zoom).max(Vec2::ZERO) + rect.size();
                    self.pan = self.pan.clamp(-slack, slack);
                }
                if response.clicked() {
                    chosen = Some(pane.photo_index);
                }
                if response.double_clicked() {
                    chosen = Some(pane.photo_index);
                    opened = true;
                }
            }
        });

        if let Some(index) = chosen {
            select_photo(model, batch_index, index);
        }
        if opened {
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
            self.view = ScreenView::Loupe;
        }
    }

    // ---- grid ----------------------------------------------------------------------------

    /// The whole shoot as a contact sheet. A click moves the cursor (through the same batch-walk
    /// rule the filmstrip uses); a double-click lands in the loupe on that photo.
    fn grid(&mut self, ui: &mut egui::Ui, model: &mut Model, images: &FrameImages<'_>) {
        // v1's grid is the CURRENT batch, not the whole shoot (todo.md §9.6): compare candidates
        // sit next to each other, and the strip is the way to travel elsewhere.
        let batch_index = model.current_batch_index();
        let mut cells: Vec<Thumb> = Vec::new();
        if let Some(batch) = model.current_batch() {
            for (photo_index, photo_id) in batch.photo_ids.iter().enumerate() {
                let Some(photo) = model.photos().iter().find(|photo| photo.meta.id == *photo_id) else { continue };
                cells.push(Thumb { id: photo_id.0, batch_index, photo_index, file_name: file_name_of(&photo.meta.rel_path), rating: photo.rating });
            }
        }
        let current_id = model.current_photo_id().map(|id| id.0);
        for cell in &cells {
            if let Some(pixels) = images.thumbs.get(&cell.id) {
                load_texture(&mut self.textures, ui.ctx(), cell.id, pixels);
            }
        }
        let textures = &self.textures;

        let mut selected: Option<usize> = None;
        let mut opened: Option<usize> = None;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BACKGROUND).inner_margin(12.0)).show(ui, |panel| {
            egui::ScrollArea::vertical().show(panel, |sheet| {
                sheet.horizontal_wrapped(|sheet| {
                    for (position, cell) in cells.iter().enumerate() {
                        let response = paint_grid_cell(sheet, cell, images, textures, current_id);
                        if response.clicked() {
                            selected = Some(position);
                        }
                        if response.double_clicked() {
                            opened = Some(position);
                        }
                    }
                });
            });
        });

        for position in [selected, opened].into_iter().flatten() {
            let Some(cell) = cells.get(position) else { continue };
            select_photo(model, cell.batch_index, cell.photo_index);
        }
        if opened.is_some() {
            self.view = ScreenView::Loupe;
        }
    }

    // ---- toolbar -------------------------------------------------------------------------

    /// v1's toolbar skeleton (todo.md §9.5): batch back/forward, the batch/file title, the
    /// mode switch, and the three view buttons. Finish joins it with its own milestone.
    fn toolbar(&mut self, ui: &mut egui::Ui, model: &mut Model) {
        let mode = model.settings().rating_mode;
        let folder = model.folder_name().to_string();
        let batch_count = model.batches().len();
        let batch_index = model.current_batch_index();
        let title = model.current_photo().map_or_else(
            || format!("Batch {} of {batch_count}", batch_index + 1),
            |photo| format!("Batch {} of {batch_count} — {}", batch_index + 1, file_name_of(&photo.meta.rel_path)),
        );
        let mut navigate: Option<i32> = None;
        let mut chosen_mode = None;
        let mut chosen_view = None;
        let mut toggle_info = false;
        egui::Panel::top("toolbar").exact_size(38.0).frame(egui::Frame::new().fill(STRIP).inner_margin(8.0)).show(ui, |bar| {
            bar.horizontal(|bar| {
                if bar.button(RichText::new("‹").size(18.0)).on_hover_text("Previous batch").clicked() {
                    navigate = Some(-1);
                }
                if bar.button(RichText::new("›").size(18.0)).on_hover_text("Next batch").clicked() {
                    navigate = Some(1);
                }
                bar.add_space(6.0);
                bar.label(RichText::new(&folder).small().weak());
                bar.label(RichText::new(title).strong().color(FOREGROUND));
                bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |bar| {
                    if bar.selectable_label(self.info_visible, RichText::new("Info").small().color(FOREGROUND)).clicked() {
                        toggle_info = true;
                    }
                    if bar.selectable_label(matches!(self.view, ScreenView::Compare(_)), RichText::new("Compare").small().color(FOREGROUND)).clicked()
                    {
                        chosen_view = Some(ScreenView::Compare(2));
                    }
                    if bar.selectable_label(self.view == ScreenView::Grid, RichText::new("Grid").small().color(FOREGROUND)).clicked() {
                        chosen_view = Some(ScreenView::Grid);
                    }
                    if bar.selectable_label(self.view == ScreenView::Loupe, RichText::new("Loupe").small().color(FOREGROUND)).clicked() {
                        chosen_view = Some(ScreenView::Loupe);
                    }
                    bar.add_space(10.0);
                    if bar.selectable_label(mode == RatingMode::KeepNotKeep, RichText::new("Keep").small().color(FOREGROUND)).clicked() {
                        chosen_mode = Some(RatingMode::KeepNotKeep);
                    }
                    if bar.selectable_label(mode == RatingMode::Stars, RichText::new("Stars").small().color(FOREGROUND)).clicked() {
                        chosen_mode = Some(RatingMode::Stars);
                    }
                });
            });
        });
        if let Some(step) = navigate {
            let _ = dispatch(model, if step < 0 { "batch.previous" } else { "batch.next" }, &Json::Null);
        }
        if let Some(mode) = chosen_mode {
            let _ = dispatch(model, "settings.ratingMode", &json!({ "mode": mode.as_str() }));
        }
        if let Some(view) = chosen_view {
            if matches!(view, ScreenView::Compare(_)) || matches!(self.view, ScreenView::Compare(_)) {
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
            }
            self.view = view;
        }
        if toggle_info {
            self.info_visible = !self.info_visible;
        }
    }

    // ---- info panel ----------------------------------------------------------------------

    /// v1's right-hand inspector: the file's capture fields and the current frame's histogram.
    /// The histogram is already computed from the **oriented** display frame
    /// (`firstcut_decode::histogram`), so this panel never rotates or resamples anything.
    fn info_panel(&self, ui: &mut egui::Ui, model: &Model, histogram: Option<&Histogram>) {
        egui::Panel::right("info-panel").exact_size(280.0).frame(egui::Frame::new().fill(STRIP).inner_margin(12.0)).show(ui, |panel| {
            let Some(photo) = model.current_photo() else {
                panel.label(RichText::new("Nothing selected").weak());
                return;
            };
            panel.label(RichText::new(file_name_of(&photo.meta.rel_path)).strong().color(FOREGROUND));
            panel.add_space(6.0);
            for line in capture_lines(photo) {
                panel.label(RichText::new(line).small().color(FOREGROUND));
            }
            panel.add_space(4.0);
            panel.label(RichText::new(rating_summary(&photo.rating, model.settings().rating_mode)).small().color(ACCENT));

            panel.add_space(12.0);
            panel.label(RichText::new("HISTOGRAM").small().weak());
            draw_histogram(panel, histogram);
        });
    }

    // ---- filmstrip ------------------------------------------------------------------------

    fn filmstrip(&mut self, ui: &mut egui::Ui, model: &mut Model, images: &FrameImages<'_>) {
        // Plain copies of what the buttons need, so the paint pass below touches nothing the
        // dispatches then write: reads happen here, actions after the paint.
        let mut thumbs: Vec<Thumb> = Vec::new();
        for (batch_index, batch) in model.batches().iter().enumerate() {
            for (photo_index, photo_id) in batch.photo_ids.iter().enumerate() {
                let Some(photo) = model.photos().iter().find(|photo| photo.meta.id == *photo_id) else { continue };
                thumbs.push(Thumb { id: photo_id.0, batch_index, photo_index, file_name: file_name_of(&photo.meta.rel_path), rating: photo.rating });
            }
        }
        let current_id = model.current_photo_id().map(|id| id.0);

        // Textures staged first: the paint pass never mutates the screen.
        for thumb in &thumbs {
            if let Some(pixels) = images.thumbs.get(&thumb.id) {
                load_texture(&mut self.textures, ui.ctx(), thumb.id, pixels);
            }
        }
        let textures = &self.textures;

        let mut clicked: Vec<u64> = Vec::new();
        egui::Panel::bottom("filmstrip").exact_size(THUMB_IMAGE_HEIGHT + 46.0).frame(egui::Frame::new().fill(STRIP).inner_margin(6.0)).show(
            ui,
            |strip| {
                egui::ScrollArea::horizontal().show(strip, |strip| {
                    strip.horizontal(|row| {
                        for (position, thumb) in thumbs.iter().enumerate() {
                            if position > 0 && thumbs.get(position - 1).is_some_and(|previous| previous.batch_index != thumb.batch_index) {
                                row.add_space(16.0);
                            }
                            if paint_thumb(row, thumb, images, textures, current_id) {
                                clicked.push(thumb.id);
                            }
                        }
                    });
                });
            },
        );

        // Clicks answered through the registry only.
        for id in clicked {
            let Some(thumb) = thumbs.iter().find(|thumb| thumb.id == id) else { continue };
            select_photo(model, thumb.batch_index, thumb.photo_index);
        }
    }

    // ---- loupe ----------------------------------------------------------------------------

    fn loupe(&mut self, ui: &mut egui::Ui, model: &mut Model, images: &FrameImages<'_>) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BACKGROUND)).show(ui, |panel| {
            if let Some(error) = model.last_error() {
                // v1 showed the message over the photo; the next successful command clears it.
                // Same corner, same convention.
                panel.painter().text(
                    pos2(12.0, panel.max_rect().height() - 12.0),
                    egui::Align2::LEFT_BOTTOM,
                    error,
                    FontId::proportional(13.0),
                    Color32::from_rgb(240, 128, 128),
                );
            }

            // The display tier decodes at exactly the viewer's backing size, so record it even
            // while the first frame is still missing.
            let available = panel.max_rect();
            self.viewport_pixels = available.size() * panel.ctx().pixels_per_point();

            let Some((id, pixels)) = images.current else {
                panel.vertical_centered(|panel| {
                    panel.add_space(panel.available_height() / 2.0 - 20.0);
                    panel.label(RichText::new("decoding…").weak());
                });
                return;
            };

            // Zoom lock keeps the same level and position across a move (todo.md §7.1); with it
            // off, the next photo opens at fit.
            if self.last_photo != Some(id.0) {
                if self.last_photo.is_some() && !model.zoom_lock() {
                    self.zoom = 1.0;
                    self.pan = Vec2::ZERO;
                }
                self.last_photo = Some(id.0);
            }

            let texture = load_texture(&mut self.textures, panel.ctx(), id.0, pixels);
            let image_size = vec2(pixels.width() as f32, pixels.height() as f32);
            let fit = fit_into(available.size(), image_size);
            let display = fit * self.zoom;
            let rect = egui::Rect::from_center_size(available.center() + self.pan, display);

            let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            panel.painter().image(texture.id(), rect, uv, Color32::WHITE);

            // Overlays paint over the oriented frame. AF coordinates arrive in SENSOR space and
            // are mapped through the same EXIF transform the decoder applied; clipping samples
            // the display pixels that are already oriented.
            let viewer = model.viewer_settings();
            let af_visible = self.af_overlay.unwrap_or(viewer.af_overlay);
            if af_visible && let Some(photo) = model.current_photo() {
                draw_af_overlay(panel.painter(), rect, photo);
            }
            if self.clipping_overlay {
                draw_clipping_overlay(panel.painter(), rect, pixels, viewer.clipping_shadow_threshold, viewer.clipping_highlight_threshold);
            }

            let response = panel.allocate_rect(rect, Sense::click_and_drag());
            if response.dragged() {
                self.pan += response.drag_delta();
                // Never trap the photo off-screen: the extra slack lets it go a full pane past
                // either edge and no farther.
                let slack = (display - available.size()).max(Vec2::ZERO) + available.size();
                self.pan = self.pan.clamp(-slack, slack);
            }
            if response.double_clicked() {
                // v1: double-click jumps a whole batch, in the direction of the half clicked
                // (todo.md §7.2). It leaves the batch, so `enteringBatchBehavior` does not apply.
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
                let forward = response.interact_pointer_pos().is_none_or(|at| at.x >= rect.center().x);
                let _ = dispatch(model, "batch.jump", &json!({ "direction": if forward { "forward" } else { "back" } }));
            } else if response.clicked() {
                if self.zoom > 1.01 {
                    self.zoom = 1.0;
                    self.pan = Vec2::ZERO;
                } else {
                    // A click turns the fit view into 1:1: one sensor pixel per physical pixel,
                    // centred where the click landed (v1's one-step click zoom).
                    let at = response.interact_pointer_pos().unwrap_or(available.center());
                    let sensor_edge = model.current_photo().map_or(0, |photo| photo.meta.width.max(photo.meta.height));
                    let points_per_pixel = panel.ctx().pixels_per_point();
                    let fit_edge = fit.x.max(fit.y);
                    if let Some(target) = native_zoom(sensor_edge, fit_edge, points_per_pixel) {
                        let offset = at - available.center();
                        self.pan = zoom_anchor(self.pan, offset, self.zoom, target);
                        self.zoom = target;
                    }
                }
            }
            let scroll = panel.ctx().input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom *= 1.0 - scroll / 1_500.0;
                self.zoom = self.zoom.clamp(0.2, 12.0);
            }

            if self.hud_visible {
                self.hud(panel, model, available);
            }
        });
    }

    // ---- progress HUD ---------------------------------------------------------------------

    /// v1's floating glass HUD: where the batch sits in the shoot, the shoot's tier counts, and
    /// the rating the user is looking at. Read-only; every number comes from the model.
    fn hud(&self, panel: &egui::Ui, model: &Model, viewer: egui::Rect) {
        let batch_count = model.batches().len();
        let batch_index = model.current_batch_index();
        let photo_count = model.current_batch().map_or(0, |batch| batch.photo_ids.len());
        let photo_index = model.current_photo_index();
        let counts = model.counts_by_tier();
        let review =
            model.current_photo().map_or_else(|| String::from("no photo"), |photo| rating_summary(&photo.rating, model.settings().rating_mode));

        egui::Area::new(egui::Id::unique("cull-progress-hud"))
            .order(egui::Order::Foreground)
            .fixed_pos(viewer.left_bottom() + vec2(14.0, -14.0))
            .pivot(egui::Align2::LEFT_BOTTOM)
            .show(panel.ctx(), |hud| {
                egui::Frame::new().fill(Color32::from_black_alpha(190)).corner_radius(10.0).inner_margin(10.0).show(hud, |hud| {
                    hud.set_min_width(220.0);
                    hud.label(RichText::new(progress_line(batch_index, batch_count, photo_index, photo_count)).strong().color(FOREGROUND));
                    hud.label(RichText::new(tier_line(&counts)).small().color(FOREGROUND));
                    hud.label(RichText::new(review).small().color(ACCENT));
                });
            });
    }
}

/// The capture fields the Info panel lists, in v1's order. Missing fields are omitted rather
/// than printed as placeholders.
fn capture_lines(photo: &firstcut_engine::model::PhotoVm) -> Vec<String> {
    let meta = &photo.meta;
    let mut lines = Vec::new();
    let mut exposure = Vec::new();
    if let Some(focal) = meta.focal_length_mm {
        exposure.push(format!("{focal:.0} mm"));
    }
    if let Some(f_number) = meta.f_number {
        exposure.push(format!("f/{f_number:.1}"));
    }
    if let Some(seconds) = meta.exposure_time_s {
        exposure.push(if seconds >= 1.0 {
            format!("{seconds:.1} s")
        } else if seconds > 0.0 {
            format!("1/{} s", (1.0 / seconds).round())
        } else {
            String::from("0 s")
        });
    }
    if let Some(iso) = meta.iso {
        exposure.push(format!("ISO {iso}"));
    }
    if !exposure.is_empty() {
        lines.push(exposure.join(" · "));
    }
    let mut camera = Vec::new();
    if let Some(make) = meta.camera_make.as_deref() {
        camera.push(make.to_string());
    }
    if let Some(model) = meta.camera_model.as_deref() {
        camera.push(model.to_string());
    }
    if !camera.is_empty() {
        lines.push(camera.join(" "));
    }
    if let Some(lens) = meta.lens_model.as_deref() {
        lines.push(lens.to_string());
    }
    lines.push(format!("{} × {}", meta.width, meta.height));
    lines
}

/// Draws the 64-bin luma area and the red/green/blue traces. Each channel is normalised on the
/// **luma** maximum so the three traces share a scale and clipped highlights line up.
fn draw_histogram(panel: &mut egui::Ui, histogram: Option<&Histogram>) {
    let (rect, _) = panel.allocate_exact_size(vec2(panel.available_width(), 96.0), Sense::hover());
    let painter = panel.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(16, 16, 18));
    let Some(histogram) = histogram else {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, "histogram…", FontId::proportional(12.0), Color32::from_gray(120));
        return;
    };
    let peak = histogram.luma.iter().copied().fold(0.0f32, f32::max).max(0.000_1);
    let step = rect.width() / 64.0;
    for (index, value) in histogram.luma().iter().enumerate() {
        let height = (value / peak).clamp(0.0, 1.0) * (rect.height() - 2.0);
        let left = rect.left() + index as f32 * step;
        let bar = egui::Rect::from_min_max(pos2(left, rect.bottom() - height), pos2(left + step, rect.bottom()));
        painter.rect_filled(bar, 0.0, Color32::from_gray(180));
    }
    for (bins, color) in [
        (&histogram.red, Color32::from_rgb(255, 96, 96)),
        (&histogram.green, Color32::from_rgb(96, 220, 96)),
        (&histogram.blue, Color32::from_rgb(110, 150, 255)),
    ] {
        let points: Vec<egui::Pos2> = bins
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let x = rect.left() + (index as f32 + 0.5) * step;
                let y = rect.bottom() - (value / peak).clamp(0.0, 1.0) * (rect.height() - 2.0);
                pos2(x, y)
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(1.0, color)));
    }
}

/// "Batch 2 / 7 · photo 4 / 12" — 1-based, the way the user counts.
fn progress_line(batch_index: usize, batch_count: usize, photo_index: usize, photo_count: usize) -> String {
    let batch = if batch_count == 0 { String::from("no batch") } else { format!("Batch {} / {}", batch_index + 1, batch_count) };
    if photo_count == 0 {
        return batch;
    }
    format!("{batch} · photo {} / {photo_count}", photo_index + 1)
}

/// The shoot-wide tier counts v1 put under the progress line. Rejected is separate from
/// unrated because Finish treats it separately (todo.md §6.1).
fn tier_line(counts: &HashMap<Tier, usize>) -> String {
    let get = |tier: Tier| counts.get(&tier).copied().unwrap_or(0);
    let mut line = format!("keep {} · good {} · maybe {} · unrated {}", get(Tier::Keep), get(Tier::Good), get(Tier::Maybe), get(Tier::Unrated));
    let rejected = get(Tier::Rejected);
    if rejected > 0 {
        line.push_str(&format!(" · rejected {rejected}"));
    }
    line
}

/// The current photo's rating, worded in the active mode: keep/not keep in Keep mode, stars in
/// Stars mode. Flag and colour label ride along in both (todo.md §6.3).
fn rating_summary(rating: &Rating, mode: RatingMode) -> String {
    let mut parts = Vec::new();
    if mode == RatingMode::KeepNotKeep {
        parts.push(String::from(if rating.keep { "keep" } else { "not keep" }));
    } else {
        parts.push(match rating.stars {
            0 => String::from("unrated"),
            1 => String::from("1 star"),
            stars => format!("{stars} stars"),
        });
    }
    match rating.flag {
        Flag::Pick => parts.push(String::from("pick")),
        Flag::Reject => parts.push(String::from("reject")),
        Flag::None => {}
    }
    if let Some(label) = rating.label {
        parts.push(label.as_str().to_string());
    }
    parts.join(" · ")
}

/// One filmstrip frame's data: a plain copy the paint pass reads without touching the model.
struct Thumb {
    id: u64,
    batch_index: usize,
    /// The photo's position inside its own batch, what `photo.goto` takes.
    photo_index: usize,
    file_name: String,
    rating: Rating,
}

/// One queued command from the keyboard: the id and its parameters.
type Queued = (&'static str, Json);

/// The keys v1's DefaultKeymap puts on this slice: stars 0–5, arrows, P/X, undo/redo. "Command"
/// is the platform's own — command on the mac, ctrl elsewhere. P is mode-aware exactly as
/// todo.md §6 describes: Keep mode toggles keep, Stars mode toggles the pick flag.
fn queued_keys(ui: &egui::Ui, mode: RatingMode) -> Vec<Queued> {
    let mut queued: Vec<Queued> = Vec::new();
    ui.ctx().input(|input| {
        let command = input.modifiers.command || input.modifiers.ctrl;
        if input.key_pressed(Key::ArrowLeft) {
            queued.push((if command { "batch.previous" } else { "photo.previous" }, Json::Null));
        }
        if input.key_pressed(Key::ArrowRight) {
            queued.push((if command { "batch.next" } else { "photo.next" }, Json::Null));
        }
        for (key, stars) in [(Key::Num0, 0u8), (Key::Num1, 1), (Key::Num2, 2), (Key::Num3, 3), (Key::Num4, 4), (Key::Num5, 5)] {
            if input.key_pressed(key) {
                queued.push(("rate.stars", json!({ "stars": stars })));
            }
        }
        if input.key_pressed(Key::P) {
            queued.push((if mode == RatingMode::KeepNotKeep { "keep.toggle" } else { "flag.pick" }, Json::Null));
        }
        if input.key_pressed(Key::X) && !input.modifiers.command && !input.modifiers.ctrl {
            queued.push(("flag.reject", Json::Null));
        }
        if input.key_pressed(Key::Z) && (input.modifiers.command || input.modifiers.ctrl) {
            let id = if input.modifiers.shift { "edit.redo" } else { "edit.undo" };
            queued.push((id, Json::Null));
        }
    });
    queued
}

/// Moves the cursor to a photo that may live in another batch: landing in another batch walks
/// batch by batch (each entry lands where the entering rule says), then `photo.goto` steps to the
/// exact photo inside that batch. The guard turns a misplaced click into a stopped walk, never a
/// hang.
fn select_photo(model: &mut Model, batch_index: usize, photo_index: usize) {
    let mut walked = 0;
    while model.current_batch_index() != batch_index && walked < 96 {
        let direction = if batch_index > model.current_batch_index() { "forward" } else { "back" };
        let _ = dispatch(model, "batch.jump", &json!({ "direction": direction }));
        walked += 1;
    }
    let _ = dispatch(model, "photo.goto", &json!({ "index": photo_index }));
}

/// One grid cell: the decoded thumbnail (or the honest slot), the name and stars. The current
/// photo carries the accent stroke, exactly like its strip frame.
fn paint_grid_cell(
    sheet: &mut egui::Ui,
    cell: &Thumb,
    images: &FrameImages<'_>,
    textures: &HashMap<u64, (usize, usize, TextureHandle)>,
    current_id: Option<u64>,
) -> egui::Response {
    let selected = current_id == Some(cell.id);
    let button = match images.thumbs.get(&cell.id).and_then(|_| textures.get(&cell.id)).map(|(_, _, handle)| handle) {
        Some(texture) => {
            let image = Image::new(texture).fit_to_exact_size(vec2(GRID_WIDTH - 10.0, GRID_WIDTH * 2.0 / 3.0));
            let mut button = egui::Button::image(image);
            if selected {
                button = button.stroke(Stroke::new(2.0, ACCENT));
            }
            button
        }
        None => egui::Button::new(RichText::new("…").weak()).min_size(vec2(GRID_WIDTH - 10.0, GRID_WIDTH * 2.0 / 3.0)),
    };
    sheet
        .vertical(|cell_ui| {
            let response = cell_ui.add(button).on_hover_text(format!("grid: {}", cell.file_name));
            cell_ui.label(RichText::new(&cell.file_name).small().color(if selected { ACCENT } else { FOREGROUND }));
            cell_ui.label(RichText::new(stars_label(&cell.rating)).small().weak());
            response
        })
        .inner
}

/// One filmstrip frame: the photo's pixels (or the honest placeholder), its name and stars, an
/// outline when it is the cursor's photo. Returns whether it was clicked.
fn paint_thumb(
    row: &mut egui::Ui,
    thumb: &Thumb,
    images: &FrameImages<'_>,
    textures: &HashMap<u64, (usize, usize, TextureHandle)>,
    current_id: Option<u64>,
) -> bool {
    let selected = current_id == Some(thumb.id);
    row.vertical(|card| {
        let button = match images.thumbs.get(&thumb.id).and_then(|_| textures.get(&thumb.id)).map(|(_, _, handle)| handle) {
            Some(texture) => {
                let image = Image::new(texture).fit_to_exact_size(vec2(THUMB_WIDTH - 12.0, THUMB_IMAGE_HEIGHT));
                let mut button = egui::Button::image(image);
                if selected {
                    button = button.stroke(Stroke::new(2.0, ACCENT));
                }
                button
            }
            // Not decoded yet: the strip shows a dark slot rather than lying with a blank white
            // rectangle that reads as a blown highlight.
            None => egui::Button::new(RichText::new("…").weak()).min_size(vec2(THUMB_WIDTH - 12.0, THUMB_IMAGE_HEIGHT)),
        };
        let clicked = card.add(button).on_hover_text(format!("filmstrip: {}", thumb.file_name)).clicked();
        card.label(RichText::new(&thumb.file_name).small().color(if selected { ACCENT } else { FOREGROUND }));
        card.label(RichText::new(stars_label(&thumb.rating)).small().weak());
        clicked
    })
    .inner
}

/// GPU texture for one photo's pixels, decoded once and cached until the photo is gone.
///
/// Oversized frames are downsampled before the upload: egui asserts a texture no wider than
/// the device's `max_texture_side`, and a real 6000×4000 R8 preview crashes the first loupe
/// otherwise (the showcase run on Game1JENKS proved it). The nearest-neighbour stride keeps
/// every sampled pixel's colour and costs nothing next to the decode; M2's decode tiers
/// deliver properly-sized previews and this becomes a safety net rather than the rule.
fn load_texture<'a>(
    textures: &'a mut HashMap<u64, (usize, usize, TextureHandle)>,
    ctx: &egui::Context,
    id: u64,
    pixels: &egui::ColorImage,
) -> &'a TextureHandle {
    let size = (pixels.size[0], pixels.size[1]);
    // A zoomed re-decode changes the pixel dimensions under the same photo id; egui textures are
    // fixed-size, so the old handle is dropped and a fresh upload replaces it.
    let stale = textures.get(&id).is_some_and(|(width, height, _)| (*width, *height) != size);
    if stale {
        textures.remove(&id);
    }
    let entry = textures.entry(id).or_insert_with(|| {
        let name = format!("photo-{id}");
        let max_side = ctx.input(|input| input.max_texture_side);
        (size.0, size.1, ctx.load_texture(name, fit_to_max_side(pixels, max_side), egui::TextureOptions::LINEAR))
    });
    &entry.2
}

/// The largest copy of `pixels` whose sides respect `max_side`: ONE stride chosen from the
/// two axes applies to both, so the aspect survives; never upscaled; empty images pass
/// through untouched. Sampled pixels keep their own colour (nearest-neighbour by stride).
fn fit_to_max_side(pixels: &egui::ColorImage, max_side: usize) -> egui::ColorImage {
    let [width, height] = pixels.size;
    if (width <= max_side && height <= max_side) || width == 0 || height == 0 {
        return egui::ColorImage { size: [width, height], source_size: pixels.source_size, pixels: pixels.pixels.clone() };
    }
    let step = width.div_ceil(max_side).max(height.div_ceil(max_side));
    let new_width = width.div_ceil(step);
    let new_height = height.div_ceil(step);
    // The stride grid only ever shrinks, so the sampled copy is made of source pixels and
    // carries the original's full extent as its source size.
    let mut sampled = Vec::with_capacity(new_width * new_height);
    for y in (0..new_height).map(|row| row * step) {
        for x in (0..new_width).map(|column| column * step) {
            if let Some(color) = pixels.pixels.get(y * width + x) {
                sampled.push(*color);
            }
        }
    }
    egui::ColorImage { size: [new_width, new_height], source_size: pixels.source_size, pixels: sampled }
}

/// Stars the way v1 drew them under a strip frame; unrated reads as a dot, not an empty line.
/// ASCII on purpose: egui's bundled fonts carry no U+2605, and the missing-glyph panic must
/// never be one frame away. v1's vector stars come back with the theme assets leg.
fn stars_label(rating: &Rating) -> String {
    match rating.stars {
        0 => String::from("·"),
        stars => "*".repeat(stars as usize),
    }
}

fn file_name_of(rel_path: &str) -> String {
    rel_path.rsplit('/').next().unwrap_or(rel_path).to_string()
}

/// Maps a normalized point in the stored (sensor) frame into display orientation, using the same
/// transform as `firstcut_decode::apply_orientation` (EXIF 1..=8).
fn map_orientation(u: f32, v: f32, orientation: u8) -> (f32, f32) {
    match orientation {
        2 => (1.0 - u, v),
        3 => (1.0 - u, 1.0 - v),
        4 => (u, 1.0 - v),
        5 => (v, u),
        6 => (1.0 - v, u),
        7 => (1.0 - v, 1.0 - u),
        8 => (v, 1.0 - u),
        _ => (u, v),
    }
}

/// Whether an orientation swaps the sensor's axes, so a point's `w`/`h` swap with them.
fn orientation_swaps(orientation: u8) -> bool {
    matches!(orientation, 5..=8)
}

/// Draws the camera's in-focus AF points, mapped through orientation. Points the camera did not
/// report as in focus stay undrawn: the overlay's claim is "here is what it focused on".
fn draw_af_overlay(painter: &egui::Painter, image: egui::Rect, photo: &firstcut_engine::model::PhotoVm) {
    let Some(af) = photo.meta.af.as_ref() else { return };
    let swaps = orientation_swaps(photo.meta.orientation);
    let clipped = painter.with_clip_rect(image);
    for point in af.points.iter().filter(|point| point.in_focus) {
        let (u, v) = map_orientation(point.x, point.y, photo.meta.orientation);
        let (w, h) = if swaps { (point.h, point.w) } else { (point.w, point.h) };
        let center = pos2(image.left() + u * image.width(), image.top() + v * image.height());
        let box_rect = egui::Rect::from_center_size(center, vec2((w * image.width()).max(6.0), (h * image.height()).max(6.0)));
        clipped.rect_stroke(box_rect, 1.0, Stroke::new(1.5, ACCENT), egui::StrokeKind::Inside);
    }
}

/// v1's clipping overlay: red where a channel is at the highlight threshold, blue where the
/// frame is crushed to shadow. Sampling is capped so a 6000 px frame cannot cost more than a
/// coarse grid of checks per paint.
fn draw_clipping_overlay(painter: &egui::Painter, image: egui::Rect, pixels: &egui::ColorImage, shadow: f64, highlight: f64) {
    let low = (shadow.clamp(0.0, 1.0) * 255.0).round() as u8;
    let high = (highlight.clamp(0.0, 1.0) * 255.0).round() as u8;
    let [width, height] = [pixels.width(), pixels.height()];
    if width == 0 || height == 0 {
        return;
    }
    let stride = (width.max(height) / 400).max(1);
    let clipped = painter.with_clip_rect(image);
    let cell = vec2((image.width() / width as f32 * stride as f32).max(2.0), (image.height() / height as f32 * stride as f32).max(2.0));
    for y in (0..height).step_by(stride) {
        for x in (0..width).step_by(stride) {
            let Some(pixel) = pixels.pixels.get(y * width + x) else { continue };
            let [r, g, b, _] = pixel.to_array();
            let mark = clip_marks(r, g, b, low, high);
            let color = match mark {
                Some(Clip::Highlight) => Color32::from_rgba_unmultiplied(255, 60, 60, 210),
                Some(Clip::Shadow) => Color32::from_rgba_unmultiplied(70, 130, 255, 210),
                None => continue,
            };
            let at = pos2(image.left() + x as f32 / width as f32 * image.width(), image.top() + y as f32 / height as f32 * image.height());
            clipped.rect_filled(egui::Rect::from_min_size(at, cell), 0.0, color);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Clip {
    Highlight,
    Shadow,
}

/// Highlight wins when a channel is at the top; shadow needs all three crushed, so a single
/// black channel in a saturated colour is not called a clipped shadow.
fn clip_marks(r: u8, g: u8, b: u8, low: u8, high: u8) -> Option<Clip> {
    if r >= high || g >= high || b >= high {
        Some(Clip::Highlight)
    } else if r <= low && g <= low && b <= low {
        Some(Clip::Shadow)
    } else {
        None
    }
}

/// The zoom that maps one sensor pixel to one physical pixel: the sensor's long edge over the
/// fit view's long edge in physical pixels. `None` before layout or for a zero-sized sensor.
fn native_zoom(sensor_edge: u32, fit_edge_points: f32, points_per_pixel: f32) -> Option<f32> {
    let fit_edge = fit_edge_points * points_per_pixel;
    if sensor_edge == 0 || !fit_edge.is_finite() || fit_edge <= 0.0 {
        return None;
    }
    Some((sensor_edge as f32 / fit_edge).clamp(1.0, 12.0))
}

/// The pan that keeps the image point under `offset` (from the pane's centre) fixed when the
/// zoom changes from `zoom` to `next`. At zoom z the point under `offset` is
/// `(offset - pan) / z`; solving for the new pan is `offset - (offset - pan) * next / z`.
fn zoom_anchor(pan: Vec2, offset: Vec2, zoom: f32, next: f32) -> Vec2 {
    if zoom <= 0.0 {
        return pan;
    }
    offset - (offset - pan) * (next / zoom)
}

/// The largest `[image size]` that fits `[into]`, keeping the aspect.
fn fit_into(into: Vec2, image: Vec2) -> Vec2 {
    if image.x <= 0.0 || image.y <= 0.0 {
        return into;
    }
    let scale = (into.x / image.x).min(into.y / image.y);
    image * scale.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use firstcut_types::ColorLabel;

    /// The real-photo crash the M1 showcase run caught: a 6000×4000 preview went to
    /// `load_texture` raw and egui's max-texture-side assert killed the frame.
    #[test]
    fn a_six_thousand_pixel_preview_fits_the_texture_limit() {
        let width = 6000;
        let height = 4000;
        let mut image = egui::ColorImage::new([width, height], vec![egui::Color32::BLACK; width * height]);
        // A sentinel in a sampled position (both coordinates are multiples of the stride 3)
        // and one on the far edge, to prove the stride grid still reaches it.
        image.pixels[0] = egui::Color32::from_rgba_unmultiplied(11, 0, 0, 255);
        image.pixels[3999 * width + 5997] = egui::Color32::from_rgba_unmultiplied(200, 0, 0, 255);

        let fitted = fit_to_max_side(&image, 2048);

        assert!(fitted.size[0] <= 2048 && fitted.size[1] <= 2048, "{:?}", fitted.size);
        // The single uniform stride keeps the 3:2 aspect: 2000×1334.
        assert_eq!(fitted.size, [2000, 1334]);
        assert_eq!(fitted.pixels[0].r(), 11, "the origin pixel is sampled");
        assert_eq!(fitted.pixels[1333 * 2000 + 1999].r(), 200, "the far edge is sampled");
    }

    #[test]
    fn small_images_are_never_upscaled_and_empty_ones_pass_through() {
        let small = egui::ColorImage::new([100, 60], vec![egui::Color32::from_rgba_unmultiplied(7, 0, 0, 255); 100 * 60]);
        let copy = fit_to_max_side(&small, 2048);
        assert_eq!(copy.size, [100, 60]);
        assert_eq!(copy.pixels[1234].r(), 7, "the copy keeps the original pixels");

        let exactly_at_limit = egui::ColorImage::new([2048, 2048], vec![egui::Color32::BLACK; 2048 * 2048]);
        assert_eq!(fit_to_max_side(&exactly_at_limit, 2048).size, [2048, 2048]);

        let empty = egui::ColorImage { size: [0, 0], source_size: Vec2::ZERO, pixels: Vec::new() };
        assert_eq!(fit_to_max_side(&empty, 2048).size, [0, 0]);
    }

    #[test]
    fn the_hud_counts_from_one_and_words_the_active_mode() {
        assert_eq!(progress_line(0, 3, 0, 12), "Batch 1 / 3 · photo 1 / 12");
        assert_eq!(progress_line(0, 0, 0, 0), "no batch", "Welcome has no batch to name");

        let rated = Rating { stars: 4, flag: Flag::Pick, label: Some(ColorLabel::Red), ..Rating::default() };
        assert_eq!(rating_summary(&rated, RatingMode::Stars), "4 stars · pick · Red");
        assert_eq!(rating_summary(&Rating::default(), RatingMode::Stars), "unrated");
        assert_eq!(rating_summary(&Rating { keep: true, ..Rating::default() }, RatingMode::KeepNotKeep), "keep");
        assert_eq!(rating_summary(&Rating::default(), RatingMode::KeepNotKeep), "not keep");
    }

    #[test]
    fn the_tier_line_shows_rejected_only_when_something_was_rejected() {
        let mut counts = HashMap::new();
        counts.insert(Tier::Keep, 2);
        counts.insert(Tier::Unrated, 5);
        assert_eq!(tier_line(&counts), "keep 2 · good 0 · maybe 0 · unrated 5");
        counts.insert(Tier::Rejected, 1);
        assert_eq!(tier_line(&counts), "keep 2 · good 0 · maybe 0 · unrated 5 · rejected 1");
    }

    #[test]
    fn af_points_follow_the_exif_orientation() {
        assert_eq!(map_orientation(0.25, 0.1, 1), (0.25, 0.1));
        assert_eq!(map_orientation(0.25, 0.1, 3), (0.75, 0.9));
        assert_eq!(map_orientation(0.25, 0.1, 6), (0.9, 0.25));
        assert_eq!(map_orientation(0.25, 0.1, 8), (0.1, 0.75));
        assert!(orientation_swaps(5) && orientation_swaps(8));
        assert!(!orientation_swaps(1) && !orientation_swaps(4));
    }

    #[test]
    fn clipping_marks_follow_the_thresholds() {
        assert_eq!(clip_marks(255, 0, 0, 5, 250), Some(Clip::Highlight));
        assert_eq!(clip_marks(0, 0, 0, 5, 250), Some(Clip::Shadow));
        assert_eq!(clip_marks(128, 128, 128, 5, 250), None);
        assert_eq!(clip_marks(250, 10, 10, 5, 250), Some(Clip::Highlight), "the highlight threshold is inclusive");
        assert_eq!(clip_marks(5, 6, 5, 5, 250), None, "one channel above the shadow threshold is not crushed");
    }

    #[test]
    fn a_click_zoom_maps_one_sensor_pixel_to_one_physical_pixel() {
        assert_eq!(native_zoom(6000, 800.0, 2.0), Some(3.75));
        assert_eq!(native_zoom(1000, 800.0, 2.0), Some(1.0), "a small frame is never enlarged by the click");
        assert_eq!(native_zoom(6000, 0.0, 2.0), None, "no layout yet");
        assert_eq!(native_zoom(0, 800.0, 2.0), None, "a nameless frame");
    }

    #[test]
    fn zoom_anchoring_keeps_the_point_under_the_cursor() {
        let pan = vec2(12.0, -8.0);
        let offset = vec2(100.0, -50.0);
        let anchored = zoom_anchor(pan, offset, 1.0, 2.0);
        let before = (offset - pan) / 1.0;
        let after = (offset - anchored) / 2.0;
        assert!((before - after).length() < 1e-4, "{before:?} vs {after:?}");
    }

    #[test]
    fn a_two_pixel_wide_strip_is_bounded_on_its_long_side() {
        let tall = egui::ColorImage::new([2, 4096], vec![egui::Color32::BLACK; 2 * 4096]);
        let fitted = fit_to_max_side(&tall, 2048);
        assert!(fitted.size[0] <= 2048 && fitted.size[1] <= 2048);
        assert_eq!(fitted.size, [1, 2048]);
    }
}
