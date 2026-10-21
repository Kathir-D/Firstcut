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
use firstcut_types::{Flag, Rating, RatingMode, Tier};
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
}

impl Default for CullScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl CullScreen {
    pub fn new() -> Self {
        Self { textures: HashMap::new(), zoom: 1.0, pan: Vec2::ZERO, viewport_pixels: Vec2::ZERO, hud_visible: true }
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

            let texture = load_texture(&mut self.textures, panel.ctx(), id.0, pixels);
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
    fn a_two_pixel_wide_strip_is_bounded_on_its_long_side() {
        let tall = egui::ColorImage::new([2, 4096], vec![egui::Color32::BLACK; 2 * 4096]);
        let fitted = fit_to_max_side(&tall, 2048);
        assert!(fitted.size[0] <= 2048 && fitted.size[1] <= 2048);
        assert_eq!(fitted.size, [1, 2048]);
    }
}
