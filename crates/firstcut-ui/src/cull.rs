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
use firstcut_types::Rating;
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

pub struct CullScreen {
    /// Decoded pixels → GPU textures, keyed by photo id, so an unchanged frame reuses them all.
    textures: HashMap<u64, TextureHandle>,
    /// The loupe's zoom, relative to the fit-to-pane size (1.0 = the whole photo shown).
    zoom: f32,
    /// The loupe's pan while zoomed in; the pane centres the photo at rest.
    pan: Vec2,
}

impl Default for CullScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl CullScreen {
    pub fn new() -> Self {
        Self { textures: HashMap::new(), zoom: 1.0, pan: Vec2::ZERO }
    }

    /// Paints one frame. Takes the model mutable because dispatching is this surface's only way
    /// to act, and the keyboard is dispatching.
    pub fn show(&mut self, ui: &mut egui::Ui, model: &mut Model, images: &FrameImages<'_>) {
        // Photos that left the shoot no longer need their GPU memory; a rescan can drop hundreds.
        let known: Vec<u64> = model.photos().iter().map(|photo| photo.meta.id.0).collect();
        self.textures.retain(|id, _| known.contains(id));

        match model.phase() {
            Phase::Welcome => self.show_welcome(ui),
            Phase::Culling => self.show_culling(ui, model, images),
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
        for (id, params) in queued_keys(ui) {
            let _ = dispatch(model, id, &params);
        }
        self.filmstrip(ui, model, images);
        self.loupe(ui, model, images);
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

        // Clicks answered through the registry only. Landing in another batch walks batch by
        // batch (each entry lands where the entering rule says), then `photo.goto` steps to the
        // exact photo inside that batch. The guard turns a misplaced click into a stopped walk,
        // never a hang.
        for id in clicked {
            let Some(thumb) = thumbs.iter().find(|thumb| thumb.id == id) else { continue };
            let mut walked = 0;
            while model.current_batch_index() != thumb.batch_index && walked < 96 {
                let direction = if thumb.batch_index > model.current_batch_index() { "forward" } else { "back" };
                let _ = dispatch(model, "batch.jump", &json!({ "direction": direction }));
                walked += 1;
            }
            let _ = dispatch(model, "photo.goto", &json!({ "index": thumb.photo_index }));
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

            let Some((id, pixels)) = images.current else {
                panel.vertical_centered(|panel| {
                    panel.add_space(panel.available_height() / 2.0 - 20.0);
                    panel.label(RichText::new("decoding…").weak());
                });
                return;
            };

            let texture = load_texture(&mut self.textures, panel.ctx(), id.0, pixels);
            let available = panel.max_rect();
            let image_size = vec2(pixels.width() as f32, pixels.height() as f32);
            let fit = fit_into(available.size(), image_size);
            let display = fit * self.zoom;
            let rect = egui::Rect::from_center_size(available.center() + self.pan, display);

            let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            panel.painter().image(texture.id(), rect, uv, Color32::WHITE);

            let response = panel.allocate_rect(rect, Sense::click_and_drag());
            if response.dragged() {
                self.pan += response.drag_delta();
                // Never trap the photo off-screen: the extra slack lets it go a full pane past
                // either edge and no farther.
                let slack = (display - available.size()).max(Vec2::ZERO) + available.size();
                self.pan = self.pan.clamp(-slack, slack);
            }
            if response.double_clicked() {
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
            }
            let scroll = panel.ctx().input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom *= 1.0 - scroll / 1_500.0;
                self.zoom = self.zoom.clamp(0.2, 12.0);
            }
        });
    }
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

/// The keys v1's DefaultKeymap puts on this slice: stars 0–5, arrows, undo/redo. "Command" is the
/// platform's own — command on the mac, ctrl elsewhere.
fn queued_keys(ui: &egui::Ui) -> Vec<Queued> {
    let mut queued: Vec<Queued> = Vec::new();
    ui.ctx().input(|input| {
        if input.key_pressed(Key::ArrowLeft) {
            queued.push(("photo.previous", Json::Null));
        }
        if input.key_pressed(Key::ArrowRight) {
            queued.push(("photo.next", Json::Null));
        }
        for (key, stars) in [(Key::Num0, 0u8), (Key::Num1, 1), (Key::Num2, 2), (Key::Num3, 3), (Key::Num4, 4), (Key::Num5, 5)] {
            if input.key_pressed(key) {
                queued.push(("rate.stars", json!({ "stars": stars })));
            }
        }
        if input.key_pressed(Key::Z) && (input.modifiers.command || input.modifiers.ctrl) {
            let id = if input.modifiers.shift { "edit.redo" } else { "edit.undo" };
            queued.push((id, Json::Null));
        }
    });
    queued
}

/// One filmstrip frame: the photo's pixels (or the honest placeholder), its name and stars, an
/// outline when it is the cursor's photo. Returns whether it was clicked.
fn paint_thumb(row: &mut egui::Ui, thumb: &Thumb, images: &FrameImages<'_>, textures: &HashMap<u64, TextureHandle>, current_id: Option<u64>) -> bool {
    let selected = current_id == Some(thumb.id);
    row.vertical(|card| {
        let button = match images.thumbs.get(&thumb.id).and_then(|_| textures.get(&thumb.id)) {
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
fn load_texture<'a>(textures: &'a mut HashMap<u64, TextureHandle>, ctx: &egui::Context, id: u64, pixels: &egui::ColorImage) -> &'a TextureHandle {
    textures.entry(id).or_insert_with(|| {
        let name = format!("photo-{id}");
        ctx.load_texture(name, pixels.clone(), egui::TextureOptions::LINEAR)
    })
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

/// The largest `[image size]` that fits `[into]`, keeping the aspect.
fn fit_into(into: Vec2, image: Vec2) -> Vec2 {
    if image.x <= 0.0 || image.y <= 0.0 {
        return into;
    }
    let scale = (into.x / image.x).min(into.y / image.y);
    image * scale.max(0.0)
}
