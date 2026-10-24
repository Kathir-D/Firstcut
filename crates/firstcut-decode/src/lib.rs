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

use rayon::prelude::*;
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
    #[error("{path}: the RAW developer refused the file: {message}")]
    Raw { path: String, message: String },
    #[error("{path}: the RAW developer returned {width}x{height} with {colors} colour channels, which is not a displayable bitmap")]
    RawLayout { path: String, width: u32, height: u32, colors: u16 },
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
    let Some(new_width_usize) = usize::try_from(new_width).ok() else {
        return DecodedImage { width: 0, height: 0, rgba: Vec::new() };
    };
    let Some(new_height_usize) = usize::try_from(new_height).ok() else {
        return DecodedImage { width: 0, height: 0, rgba: Vec::new() };
    };
    let Some(capacity) = new_width_usize.checked_mul(new_height_usize).and_then(|n| n.checked_mul(4)) else {
        return DecodedImage { width: 0, height: 0, rgba: Vec::new() };
    };
    let Some(row_bytes) = new_width_usize.checked_mul(4) else {
        return DecodedImage { width: 0, height: 0, rgba: Vec::new() };
    };
    let width_usize = usize::try_from(width).unwrap_or(usize::MAX);
    let height_usize = usize::try_from(height).unwrap_or(usize::MAX);
    let step_usize = usize::try_from(step).unwrap_or(usize::MAX);
    let mut reduced = vec![0u8; capacity];
    // A full 24 MP display frame is ~11 MB after reduction; reducing it on one core costs
    // several milliseconds that a held arrow cannot spare. Rows are independent, so the
    // large case is parallel while the 256 px filmstrip stays sequential (rayon's fan-out
    // would cost more than the copy).
    if rgba.len() > 4_000_000 && new_height_usize > 1 {
        reduced.par_chunks_mut(row_bytes).enumerate().for_each(|(y, row)| {
            downsample_row(row, &rgba, width_usize, height_usize, step_usize, y);
        });
    } else {
        for (y, row) in reduced.chunks_mut(row_bytes).enumerate() {
            downsample_row(row, &rgba, width_usize, height_usize, step_usize, y);
        }
    }
    DecodedImage { width: new_width, height: new_height, rgba: reduced }
}

/// Copies every `step`-th pixel of one destination row. Every index is checked: a scan result
/// that disagrees with the buffer returns a black pixel instead of panicking.
fn downsample_row(row: &mut [u8], rgba: &[u8], width: usize, height: usize, step: usize, y: usize) {
    let Some(last_y) = height.checked_sub(1) else { return };
    let sy = y.saturating_mul(step).min(last_y);
    let Some(last_x) = width.checked_sub(1) else { return };
    for (x, destination) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let sx = x.saturating_mul(step).min(last_x);
        let Some(source) = sy.checked_mul(width).and_then(|base| base.checked_add(sx)).and_then(|pixel| pixel.checked_mul(4)) else {
            continue;
        };
        let Some(end) = source.checked_add(4) else { continue };
        if let Some(pixel) = rgba.get(source..end) {
            destination.copy_from_slice(pixel);
        }
    }
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
    let width = usize::try_from(width).map_err(|_| DecodeError::OrientationOverflow { width, height, orientation })?;
    let height = usize::try_from(height).map_err(|_| DecodeError::OrientationOverflow { width: u32::MAX, height, orientation })?;
    let out_width = usize::try_from(out_width).map_err(|_| DecodeError::OrientationOverflow { width: u32::MAX, height: u32::MAX, orientation })?;
    let out_height = usize::try_from(out_height).map_err(|_| DecodeError::OrientationOverflow { width: u32::MAX, height: u32::MAX, orientation })?;
    let expected = out_width.checked_mul(out_height).and_then(|area| area.checked_mul(4)).ok_or(DecodeError::OrientationOverflow {
        width: u32::MAX,
        height: u32::MAX,
        orientation,
    })?;
    if expected != rgba.len() {
        return Err(DecodeError::SizeMismatch {
            width: u32::try_from(out_width).unwrap_or(u32::MAX),
            height: u32::try_from(out_height).unwrap_or(u32::MAX),
            expected,
        });
    }
    if out_width == 0 || out_height == 0 {
        return Ok(DecodedImage { width: u32::try_from(out_width).unwrap_or(0), height: u32::try_from(out_height).unwrap_or(0), rgba: Vec::new() });
    }
    let row_bytes = out_width.checked_mul(4).ok_or(DecodeError::OrientationOverflow { width: u32::MAX, height: u32::MAX, orientation })?;
    let mut out = vec![0u8; expected];
    // Destination-oriented and row-parallel: each output row writes a contiguous slice, so a
    // portrait 24 MP rotate is memory-bandwidth bound instead of a scalar pointer chase. The
    // inverse of the EXIF transform names the source pixel for each destination pixel.
    out.par_chunks_mut(row_bytes).enumerate().for_each(|(dy, row)| {
        for dx in 0..out_width {
            let (sx, sy) = match orientation {
                2 => (width - 1 - dx, dy),
                3 => (width - 1 - dx, height - 1 - dy),
                4 => (dx, height - 1 - dy),
                5 => (dy, dx),
                6 => (dy, height - 1 - dx),
                7 => (width - 1 - dy, height - 1 - dx),
                8 => (width - 1 - dy, dx),
                _ => (dx, dy),
            };
            let Some(source) = sy.checked_mul(width).and_then(|row| row.checked_add(sx)).and_then(|pixel| pixel.checked_mul(4)) else { continue };
            let destination = dx * 4;
            if let (Some(from), Some(to)) = (rgba.get(source..source.saturating_add(4)), row.get_mut(destination..destination.saturating_add(4))) {
                to.copy_from_slice(from);
            }
        }
    });
    Ok(DecodedImage { width: u32::try_from(out_width).unwrap_or(u32::MAX), height: u32::try_from(out_height).unwrap_or(u32::MAX), rgba: out })
}

/// The histogram lives in `firstcut-types` so the Info panel can name it without depending on a
/// decoder. Computed from an already-oriented frame, so a sideways photograph cannot produce a
/// shifted histogram (the v1 audit's histogram/orientation defect).
pub use firstcut_types::Histogram;

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
