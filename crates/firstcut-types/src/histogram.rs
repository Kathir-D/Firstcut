//! The histogram the Info panel draws: 64 bins each for red, green, blue and Rec. 601 luma, as
//! fractions of the sampled pixels (docs/contracts/pipeline-api.md). Pure data, so the UI crate
//! can name it without depending on a decoder.
//!
//! Bins are computed by `firstcut_decode::histogram` from an **already oriented** display frame;
//! applying EXIF orientation later would sample a different grid and shift the bins.

/// One histogram, in display orientation.
#[derive(Clone, Debug, PartialEq)]
pub struct Histogram {
    pub red: [f32; 64],
    pub green: [f32; 64],
    pub blue: [f32; 64],
    pub luma: [f32; 64],
    /// Pixels the bins were computed over, after the 256 px reduction.
    pub sampled_pixels: usize,
}

impl Histogram {
    /// An empty histogram (a frame with no pixels).
    pub fn empty() -> Self {
        Self { red: [0.0; 64], green: [0.0; 64], blue: [0.0; 64], luma: [0.0; 64], sampled_pixels: 0 }
    }

    /// The luminance bins, which is what the Info panel's default view draws.
    pub fn luma(&self) -> &[f32; 64] {
        &self.luma
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self::empty()
    }
}
