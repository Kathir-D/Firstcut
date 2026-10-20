//! The display-image fast path: JPEGs out of the file's own embedded byte ranges, without
//! touching a RAW decoder (todo.md §7.5, photo-meta.md).
//!
//! A CR3 carries three JPEGs and they are not interchangeable: `THMB` is 160×120, `PRVW` is
//! 1620×1080, and the first `trak` is the sensor's full frame. The display image is the
//! full-resolution one ([`PhotoMeta::full_preview`]), which is what [`decode_display`] picks;
//! `preview` is the fallback a file without a full-size JPEG offers. Conflating the two is the
//! bug that was made twice in v1.
//!
//! Full RAW decode (Exact tier) attaches at M2 via the backend trait [`RawDecode`]; this crate
//! holds the trait so the surfaces can already compile against "a decode", not "a backend".

use std::path::Path;

use firstcut_meta::meta::{ByteRange, PhotoMeta};
use thiserror::Error;
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;
use zune_jpeg::errors::DecodeErrors;

/// A decoded frame: RGBA8, ready for a GPU texture or an egui `ColorImage`. The pixels are
/// already in display orientation (EXIF orientation applied exactly once, in this crate); a
/// caller must never rotate them again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, alpha 255.
    pub rgba: Vec<u8>,
}

impl DecodedImage {
    /// The pixel buffer's size in bytes, which is what the decode budget counts.
    pub fn byte_len(&self) -> usize {
        self.rgba.len()
    }

    /// The longer side, which is what a viewport asks for.
    pub fn longest_edge(&self) -> u32 {
        self.width.max(self.height)
    }
}

/// Everything the fast path refuses, with the file named so a surface can show it.
#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("reading {path}: {source}")]
    Read { path: String, source: std::io::Error },
    #[error("{path}: preview byte range {range_offset}+{range_len} runs past the file size {file_size}")]
    RangePastEnd { path: String, range_offset: u64, range_len: u64, file_size: u64 },
    #[error("{path}: has no embedded JPEG to display")]
    NoEmbeddedJpeg { path: String },
    #[error("decoding the embedded JPEG: {0}")]
    Jpeg(#[from] DecodeErrors),
    #[error("the decoded size {width}x{height}×4 does not match the {expected} byte buffer")]
    SizeMismatch { width: u32, height: u32, expected: usize },
    #[error("{width}x{height} at orientation {orientation} does not fit a pixel buffer")]
    OrientationOverflow { width: u32, height: u32, orientation: u8 },
}

/// Decodes the photograph's own pixels: the full-resolution embedded JPEG, falling back to the
/// smaller `PRVW` one, at the photograph's native size. This is the loupe's display image.
pub fn decode_display(path: &Path, meta: &PhotoMeta) -> Result<DecodedImage, DecodeError> {
    decode_display_at(path, meta, u32::MAX)
}

/// The same decode, subsampled by the JPEG decoder to at most `target_edge` on both axes. A
/// thumbnail is this call at 256 px; the display tier asks for the viewer's backing edge. The
/// EXIF orientation is applied after the subsample, so the returned longest edge is the target
/// edge rotated as the photograph will be shown.
pub fn decode_display_at(path: &Path, meta: &PhotoMeta, target_edge: u32) -> Result<DecodedImage, DecodeError> {
    let shown = meta.full_preview.or(meta.preview).ok_or_else(|| DecodeError::NoEmbeddedJpeg { path: path.display().to_string() })?;
    let image = downsample_to(read_range(path, shown.range)?, target_edge);
    apply_orientation(image, meta.orientation)
}

/// The filmstrip's tier: the small `PRVW` JPEG when the container has one (1620×1080 rather
/// than 6000×4000), reduced to `target_edge`, then oriented. Falls back to the full preview for
/// a file whose only JPEG is the large one.
pub fn decode_thumbnail(path: &Path, meta: &PhotoMeta, target_edge: u32) -> Result<DecodedImage, DecodeError> {
    let shown = meta.preview.or(meta.full_preview).ok_or_else(|| DecodeError::NoEmbeddedJpeg { path: path.display().to_string() })?;
    let image = downsample_to(read_range(path, shown.range)?, target_edge);
    apply_orientation(image, meta.orientation)
}

/// Nearest-neighbour reduction to at most `target_edge` on the longer side. The stride is
/// shared by both axes so the aspect survives; a frame already small enough is returned as-is.
/// This is the 256 px reduction the filmstrip and the histogram use.
pub fn downsample_to(image: DecodedImage, target_edge: u32) -> DecodedImage {
    let DecodedImage { width, height, rgba } = image;
    if width == 0 || height == 0 || target_edge == 0 || width.max(height) <= target_edge {
        return DecodedImage { width, height, rgba };
    }
    let step = width.div_ceil(target_edge).max(height.div_ceil(target_edge)).max(1);
    let new_width = width.div_ceil(step);
    let new_height = height.div_ceil(step);
    let Some(capacity) =
        usize::try_from(new_width).ok().and_then(|w| usize::try_from(new_height).ok().and_then(|h| w.checked_mul(h))).and_then(|n| n.checked_mul(4))
    else {
        return DecodedImage { width: 0, height: 0, rgba: Vec::new() };
    };
    let mut reduced = Vec::with_capacity(capacity);
    let mut y = 0;
    while y < height {
        let mut x = 0;
        while x < width {
            let index = (usize::try_from(y).unwrap_or(0) * usize::try_from(width).unwrap_or(0) + usize::try_from(x).unwrap_or(0)).saturating_mul(4);
            if let Some(pixel) = rgba.get(index..index.saturating_add(4)) {
                reduced.extend_from_slice(pixel);
            }
            x = x.saturating_add(step);
        }
        y = y.saturating_add(step);
    }
    DecodedImage { width: new_width, height: new_height, rgba: reduced }
}

/// Reads a file's byte range and decodes it as JPEG. The range is what `core-meta` recorded at
/// scan time; a file that changed underneath the session reports itself. No orientation is
/// applied here — [`decode_display`] is the orientation-applying seam.
pub fn read_range(path: &Path, range: ByteRange) -> Result<DecodedImage, DecodeError> {
    let path_string = path.display().to_string();
    // Checked arithmetic all the way down: a corrupt scan result must error, not wrap.
    let ends_at = range.offset.checked_add(range.len).ok_or(DecodeError::RangePastEnd {
        path: path_string.clone(),
        range_offset: range.offset,
        range_len: range.len,
        file_size: u64::MAX,
    })?;
    let file = std::fs::File::open(path).map_err(|source| DecodeError::Read { path: path_string.clone(), source })?;
    let file_size = file.metadata().map_err(|source| DecodeError::Read { path: path_string.clone(), source })?.len();
    if ends_at > file_size {
        return Err(DecodeError::RangePastEnd { path: path_string, range_offset: range.offset, range_len: range.len, file_size });
    }
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file;
    file.seek(SeekFrom::Start(range.offset)).map_err(|source| DecodeError::Read { path: path_string.clone(), source })?;
    let mut bytes = vec![0; range.len.min(usize::MAX as u64) as usize];
    file.read_exact(&mut bytes).map_err(|source| DecodeError::Read { path: path_string, source })?;
    decode_jpeg(&bytes)
}

/// Decodes JPEG bytes to RGBA8. The requested output colorspace is the one the surfaces draw
/// from, so the hot loop is a copy, not a conversion.
pub fn decode_jpeg(bytes: &[u8]) -> Result<DecodedImage, DecodeError> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let pixels = decoder.decode()?;
    let Some(info) = decoder.info() else {
        // decode() answered, so the headers decoded; the None arm is for the paranoid path.
        return Err(DecodeError::SizeMismatch { width: 0, height: 0, expected: pixels.len() });
    };
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    let expected = usize::from(info.width).checked_mul(usize::from(info.height)).and_then(|area| area.checked_mul(4)).unwrap_or(0);
    if pixels.len() != expected {
        return Err(DecodeError::SizeMismatch { width, height, expected: pixels.len() });
    }
    Ok(DecodedImage { width, height, rgba: pixels })
}

/// Whether the EXIF orientation swaps the frame's axes. Values 5..=8 are the rotated/transposed
/// orientations; 1..=4 keep the sensor's aspect.
pub fn orientation_swaps_axes(orientation: u8) -> bool {
    (5..=8).contains(&orientation)
}

/// Applies EXIF orientation `orientation` (1..=8) to a decoded frame. This is the **only** place
/// orientation is applied: the embedded-JPEG path and the RAW backend both return pixels in
/// display orientation, and every later stage (histogram, clipping, zoom) reads them as-is.
/// Values outside 1..=8 leave the frame alone, matching the reader's "not a rotation" rule.
pub fn apply_orientation(image: DecodedImage, orientation: u8) -> Result<DecodedImage, DecodeError> {
    if !(2..=8).contains(&orientation) {
        return Ok(image);
    }
    let DecodedImage { width, height, rgba } = image;
    let swaps = orientation_swaps_axes(orientation);
    let (out_width, out_height) = if swaps { (height, width) } else { (width, height) };
    let expected = usize::try_from(out_width)
        .ok()
        .and_then(|w| usize::try_from(out_height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|area| area.checked_mul(4))
        .ok_or(DecodeError::OrientationOverflow { width, height, orientation })?;
    if expected != rgba.len() {
        return Err(DecodeError::SizeMismatch { width: out_width, height: out_height, expected });
    }
    let mut out = vec![0u8; expected];
    for y in 0..height {
        for x in 0..width {
            // EXIF orientation is a transform of the *stored* frame into display orientation.
            let (dx, dy) = match orientation {
                2 => (width - 1 - x, y),
                3 => (width - 1 - x, height - 1 - y),
                4 => (x, height - 1 - y),
                5 => (y, x),
                6 => (height - 1 - y, x),
                7 => (height - 1 - y, width - 1 - x),
                8 => (y, width - 1 - x),
                _ => (x, y),
            };
            let source = (usize::try_from(y).unwrap_or(0) * usize::try_from(width).unwrap_or(0) + usize::try_from(x).unwrap_or(0)).saturating_mul(4);
            let destination =
                (usize::try_from(dy).unwrap_or(0) * usize::try_from(out_width).unwrap_or(0) + usize::try_from(dx).unwrap_or(0)).saturating_mul(4);
            // Checked slices: a corrupt dimension must never index past the buffer.
            if let (Some(from), Some(to)) = (rgba.get(source..source.saturating_add(4)), out.get_mut(destination..destination.saturating_add(4))) {
                to.copy_from_slice(from);
            }
        }
    }
    Ok(DecodedImage { width: out_width, height: out_height, rgba: out })
}

/// 64 bins each for red, green, blue and Rec. 601 luma, as fractions of the sampled pixels.
/// Computed from an already-oriented frame, so a sideways photograph cannot produce a shifted
/// histogram (the v1 audit's histogram/orientation defect).
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

/// The v1 pipeline's 256-pixel reduction: the longest side is capped at 256 and every sampled
/// pixel's own colour is counted (nearest-neighbour), so the bins describe the picture the user
/// is looking at rather than an average of it.
pub const HISTOGRAM_EDGE: u32 = 256;

/// Computes the histogram of an oriented frame.
pub fn histogram(image: &DecodedImage) -> Histogram {
    let [width, height] = [image.width, image.height];
    if width == 0 || height == 0 || image.rgba.len() < 4 {
        return Histogram::empty();
    }
    let step = width.div_ceil(HISTOGRAM_EDGE).max(height.div_ceil(HISTOGRAM_EDGE)).max(1);
    let mut bins = Histogram::empty();
    let mut counted = 0usize;
    for y in (0..height).step_by(step as usize) {
        for x in (0..width).step_by(step as usize) {
            let index = (usize::try_from(y).unwrap_or(0) * usize::try_from(width).unwrap_or(0) + usize::try_from(x).unwrap_or(0)).saturating_mul(4);
            let Some(pixel) = image.rgba.get(index..index.saturating_add(4)) else { continue };
            let (Some(&r), Some(&g), Some(&b)) = (pixel.first(), pixel.get(1), pixel.get(2)) else { continue };
            // Shifting a u8 by 2 lands in 0..=63 for every possible byte.
            if let Some(bin) = bins.red.get_mut(usize::from(r >> 2)) {
                *bin += 1.0;
            }
            if let Some(bin) = bins.green.get_mut(usize::from(g >> 2)) {
                *bin += 1.0;
            }
            if let Some(bin) = bins.blue.get_mut(usize::from(b >> 2)) {
                *bin += 1.0;
            }
            let luma = (0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b)).round().clamp(0.0, 255.0) as u8;
            if let Some(bin) = bins.luma.get_mut(usize::from(luma >> 2)) {
                *bin += 1.0;
            }
            counted += 1;
        }
    }
    if counted > 0 {
        let total = counted as f32;
        for bin in bins.red.iter_mut().chain(bins.green.iter_mut()).chain(bins.blue.iter_mut()).chain(bins.luma.iter_mut()) {
            *bin /= total;
        }
    }
    bins.sampled_pixels = counted;
    bins
}

/// The full RAW decode seam a backend crate implements (M2's LibRaw bridge). The embedded-JPEG
/// path never calls in to this; the pipelines hold a `&dyn RawDecode` and choose per work item.
pub trait RawDecode: Send + Sync {
    /// Decodes `path` at the photo's own resolution. Errors are typed here, so a backend's C
    /// failures still surface the way the never-crash rule wants.
    fn decode_raw(&self, path: &Path, meta: &PhotoMeta) -> Result<DecodedImage, DecodeError>;
}
