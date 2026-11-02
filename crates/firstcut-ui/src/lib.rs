//! The egui/eframe presentation: windows, panels, theme, widget ids (all stable strings). One
//! swappable crate; nothing below it knows a UI exists.
//!
//! The UI is a **surface**, not a brain (docs/v2-plan.md §4.3): it reads the model it is bound
//! to and acts on it only through [`firstcut_engine::registry::dispatch`]. Decoded pixels are not
//! the UI's business either — the app supplies [`FrameImages`] for the photos, and the crate only
//! puts them on screen, with a texture cache so an unchanged frame costs nothing.

use std::collections::HashMap;
use std::sync::Arc;

use firstcut_types::{Histogram, PhotoId};

mod cull;
mod finish_sheet;
mod settings_screen;

pub use cull::CullScreen;

/// Installs the bundled Inter family (D10) as the proportional face.
///
/// The three OSes ship different default UI fonts, and M4's snapshots paid for that with a
/// per-pixel tolerance wide enough to hide real glyph regressions. One font in `assets/`
/// removes the variable from the picture and makes Windows and Linux draw the same text as
/// macOS. `egui` selects by family, not by weight, so the regular file is the face; the
/// medium and semibold files are loaded under their own names for widgets that ask for a
/// heavier look through `FontId` later.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("inter-regular", &include_bytes!("../../../assets/fonts/Inter-Regular.ttf")[..]),
        ("inter-medium", &include_bytes!("../../../assets/fonts/Inter-Medium.ttf")[..]),
        ("inter-semibold", &include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf")[..]),
    ] {
        fonts.font_data.insert(name.to_owned(), std::sync::Arc::new(egui::FontData::from_static(bytes)));
    }
    fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "inter-regular".to_owned());
    // Monospace stays egui's bundled face: a proportional UI font is not a code font, and the
    // shortcut sheet and paths read better in the default monospace.
    ctx.set_fonts(fonts);
}

/// What one frame may paint: the current photo's decoded pixels and the filmstrip's, keyed by
/// photo id. The app builds this per frame from its own decode cache; a photo it has not paid
/// for yet is simply absent until the decode lands.
pub struct FrameImages<'a> {
    /// The photo under the cursor — the loupe's subject. The `Arc` is what lets the texture
    /// upload borrow the decoded pixels when no size reduction is needed, instead of copying
    /// the whole frame once more (§5.1).
    pub current: Option<(PhotoId, &'a Arc<egui::ColorImage>)>,
    /// The filmstrip's photos, as far as decided to decode. Absent ids render as a placeholder.
    pub thumbs: &'a HashMap<u64, Arc<egui::ColorImage>>,
    /// The current photo's histogram (already orientation-safe). M3's Info panel paints it.
    pub histogram: Option<&'a Histogram>,
    /// Display-tier pixels by photo id, for screens that show more than the cursor's photo
    /// (Compare). Absent ids fall back to the thumbnail, then to the honest placeholder.
    pub display: &'a HashMap<u64, Arc<egui::ColorImage>>,
}

impl FrameImages<'_> {
    /// The frame before anything is decoded (a fresh Welcome, for instance).
    pub fn empty() -> FrameImages<'static> {
        FrameImages { current: None, thumbs: &EMPTY_THUMBS, histogram: None, display: &EMPTY_THUMBS }
    }
}

static EMPTY_THUMBS: std::sync::LazyLock<HashMap<u64, Arc<egui::ColorImage>>> = std::sync::LazyLock::new(HashMap::new);
