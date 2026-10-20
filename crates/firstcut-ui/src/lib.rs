//! The egui/eframe presentation: windows, panels, theme, widget ids (all stable strings). One
//! swappable crate; nothing below it knows a UI exists.
//!
//! The UI is a **surface**, not a brain (docs/v2-plan.md §4.3): it reads the model it is bound
//! to and acts on it only through [`firstcut_engine::registry::dispatch`]. Decoded pixels are not
//! the UI's business either — the app supplies [`FrameImages`] for the photos, and the crate only
//! puts them on screen, with a texture cache so an unchanged frame costs nothing.

use std::collections::HashMap;

use firstcut_types::PhotoId;

mod cull;

pub use cull::CullScreen;

/// What one frame may paint: the current photo's decoded pixels and the filmstrip's, keyed by
/// photo id. The app builds this per frame from its own decode cache; a photo it has not paid
/// for yet is simply absent until the decode lands.
pub struct FrameImages<'a> {
    /// The photo under the cursor — the loupe's subject.
    pub current: Option<(PhotoId, &'a egui::ColorImage)>,
    /// The filmstrip's photos, as far as decided to decode. Absent ids render as a placeholder.
    pub thumbs: &'a HashMap<u64, egui::ColorImage>,
}

impl FrameImages<'_> {
    /// The frame before anything is decoded (a fresh Welcome, for instance).
    pub fn empty() -> FrameImages<'static> {
        FrameImages { current: None, thumbs: &EMPTY_THUMBS }
    }
}

static EMPTY_THUMBS: std::sync::LazyLock<std::collections::HashMap<u64, egui::ColorImage>> = std::sync::LazyLock::new(std::collections::HashMap::new);
