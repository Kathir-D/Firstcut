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

use firstcut_meta::meta::{ByteRange, EmbeddedPreview, PhotoMeta};
use thiserror::Error;
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;
use zune_jpeg::errors::DecodeErrors;

/// An RGBA8 buffer whose allocation is 4-byte aligned. `Color32` is four bytes with an align of
/// four, so the window can take the buffer over as egui's colour type with `bytemuck::try_cast_vec`
/// — no per-pixel conversion and no second 28 MB copy for a 4K display frame (§5.1).
///
/// The backing store is `Vec<u32>` (one word per pixel) purely for its alignment; the public
/// view is bytes, so callers keep reading RGBA channels in the order the decoder wrote them.
#[derive(Clone, PartialEq, Eq)]
pub struct RgbaBuffer {
    words: Vec<u32>,
    /// The logical byte length, which can be shorter than `words.len() * 4` only for a buffer
    /// built from deliberately malformed input in a test. Decoded frames are always exact.
    len: usize,
}

impl RgbaBuffer {
    /// A zeroed buffer of `bytes` bytes, rounded up to whole pixels in the backing store.
    pub fn zeroed(bytes: usize) -> Self {
        let words = bytes / 4 + usize::from(!bytes.is_multiple_of(4));
        Self { words: vec![0; words], len: bytes }
    }

    /// Copies `bytes` into an aligned buffer. Used by the RAW backends, whose pixels arrive in
    /// their own vectors; the JPEG path allocates the aligned buffer first and decodes into it.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut buffer = Self::zeroed(bytes.len());
        buffer.as_mut_slice().copy_from_slice(bytes);
        buffer
    }

    /// The byte view, always exactly the logical length.
    pub fn as_slice(&self) -> &[u8] {
        let bytes = bytemuck::cast_slice::<u32, u8>(&self.words);
        bytes.get(..self.len).unwrap_or(&[])
    }

    /// The mutable byte view, always exactly the logical length.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        let bytes = bytemuck::cast_slice_mut::<u32, u8>(&mut self.words);
        let len = self.len;
        bytes.get_mut(..len).unwrap_or(&mut [])
    }

    /// The backing words, moved out for a zero-copy handoff to a 4-byte-aligned colour type.
    /// Only valid callers should use this: the length is `words.len() * 4`.
    pub fn into_words(self) -> Vec<u32> {
        self.words
    }
}

impl std::ops::Deref for RgbaBuffer {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl std::ops::DerefMut for RgbaBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut_slice()
    }
}

impl std::fmt::Debug for RgbaBuffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("RgbaBuffer").field("len", &self.len).finish()
    }
}

impl From<Vec<u8>> for RgbaBuffer {
    fn from(bytes: Vec<u8>) -> Self {
        Self::from_bytes(&bytes)
    }
}

/// A decoded frame: RGBA8, ready for a GPU texture or an egui `ColorImage`. The pixels are
/// already in display orientation (EXIF orientation applied exactly once, in this crate); a
/// caller must never rotate them again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, alpha 255, in an aligned buffer.
    pub rgba: RgbaBuffer,
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

/// The display image at `target_edge` on its longer side. The smallest embedded JPEG that
/// actually covers the target is chosen first: on a small window the camera's 1620×1080 `PRVW`
/// is enough, so the 6000×4000 entropy decode is never touched. A target beyond the `PRVW` uses
/// the full preview, and any remaining reduction is nearest-neighbour. The EXIF orientation is
/// applied last, so the returned longest edge is the target edge as the photograph is shown.
pub fn decode_display_at(path: &Path, meta: &PhotoMeta, target_edge: u32) -> Result<DecodedImage, DecodeError> {
    let shown = select_display_source(meta.preview, meta.full_preview, target_edge)
        .ok_or_else(|| DecodeError::NoEmbeddedJpeg { path: path.display().to_string() })?;
    let image = downsample_to(read_range(path, shown.range)?, target_edge);
    apply_orientation(image, meta.orientation)
}

/// Picks the smallest embedded JPEG whose longer side covers `target_edge`. A full preview is
/// the test image for the 1:1 path (`u32::MAX`). A preview-only file still returns its preview
/// even when the target is larger — a slight upscale is better than no image.
fn select_display_source(preview: Option<EmbeddedPreview>, full: Option<EmbeddedPreview>, target_edge: u32) -> Option<EmbeddedPreview> {
    let preview_edge = preview.map(|image| image.width.max(image.height)).unwrap_or(0);
    match (preview, full) {
        (Some(preview), _) if target_edge <= preview_edge => Some(preview),
        (_, Some(full)) => Some(full),
        (preview, None) => preview,
    }
}

/// The filmstrip's tier: the small `PRVW` JPEG when the container has one (1620×1080 rather
/// than 6000×4000), reduced to `target_edge`, then oriented. Falls back to the full preview for
/// a file whose only JPEG is the large one.
pub fn decode_thumbnail(path: &Path, meta: &PhotoMeta, target_edge: u32) -> Result<DecodedImage, DecodeError> {
    let shown = meta.preview.or(meta.full_preview).ok_or_else(|| DecodeError::NoEmbeddedJpeg { path: path.display().to_string() })?;
    let image = downsample_to(read_range(path, shown.range)?, target_edge);
    apply_orientation(image, meta.orientation)
}

/// Nearest-neighbour reduction to exactly `target_edge` on the longer side. The output geometry
/// is proportional, not an integer stride: asking for 512 px from a 1200×1000 frame used to give
/// 400×334 (a 22% undershoot that the window then upscaled). It now gives 512×427. The sampler
/// still reads real source pixels, so no colour is invented. A frame already small enough is
/// returned as-is. This is the 256 px reduction the filmstrip and the histogram use.
pub fn downsample_to(image: DecodedImage, target_edge: u32) -> DecodedImage {
    let DecodedImage { width, height, rgba } = image;
    let long = width.max(height);
    if width == 0 || height == 0 || target_edge == 0 || long <= target_edge {
        return DecodedImage { width, height, rgba };
    }
    let new_width = proportional_edge(width, target_edge, long);
    let new_height = proportional_edge(height, target_edge, long);
    let Some(new_width_usize) = usize::try_from(new_width).ok() else {
        return DecodedImage { width: 0, height: 0, rgba: RgbaBuffer::zeroed(0) };
    };
    let Some(new_height_usize) = usize::try_from(new_height).ok() else {
        return DecodedImage { width: 0, height: 0, rgba: RgbaBuffer::zeroed(0) };
    };
    let Some(capacity) = new_width_usize.checked_mul(new_height_usize).and_then(|n| n.checked_mul(4)) else {
        return DecodedImage { width: 0, height: 0, rgba: RgbaBuffer::zeroed(0) };
    };
    let Some(row_bytes) = new_width_usize.checked_mul(4) else {
        return DecodedImage { width: 0, height: 0, rgba: RgbaBuffer::zeroed(0) };
    };
    let width_usize = usize::try_from(width).unwrap_or(usize::MAX);
    let height_usize = usize::try_from(height).unwrap_or(usize::MAX);
    let mut reduced = RgbaBuffer::zeroed(capacity);
    // A 2048-edge display frame is ~11 MB after reduction; reducing it on one core costs
    // several milliseconds that a held arrow cannot spare. Rows are independent, so a large
    // output is parallel while the 256 px filmstrip stays sequential (rayon's fan-out would
    // cost more than the copy).
    let source = rgba.as_slice();
    if capacity > 1_000_000 && new_height_usize > 1 {
        reduced.as_mut_slice().par_chunks_mut(row_bytes).enumerate().for_each(|(y, row)| {
            downsample_row(row, source, width_usize, height_usize, new_width_usize, new_height_usize, y);
        });
    } else {
        for (y, row) in reduced.as_mut_slice().chunks_mut(row_bytes).enumerate() {
            downsample_row(row, source, width_usize, height_usize, new_width_usize, new_height_usize, y);
        }
    }
    DecodedImage { width: new_width, height: new_height, rgba: reduced }
}

/// One axis of the proportional reduction, rounded to nearest. `long` is the source's longer
/// side; dividing by it makes the longer output side exactly `target_edge` without overflowing
/// `u32` intermediate products.
fn proportional_edge(edge: u32, target_edge: u32, long: u32) -> u32 {
    if long == 0 {
        return edge.max(1);
    }
    let numerator = u128::from(edge) * u128::from(target_edge) + u128::from(long / 2);
    u32::try_from(numerator / u128::from(long)).unwrap_or(u32::MAX).max(1)
}

/// Copies the nearest source pixel for one destination row. Every index is checked: a scan
/// result that disagrees with the buffer returns a black pixel instead of panicking.
fn downsample_row(row: &mut [u8], rgba: &[u8], width: usize, height: usize, new_width: usize, new_height: usize, y: usize) {
    let Some(last_y) = height.checked_sub(1) else { return };
    let sy = scaled_index(y, new_height, height).min(last_y);
    let Some(last_x) = width.checked_sub(1) else { return };
    for (x, destination) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let sx = scaled_index(x, new_width, width).min(last_x);
        let Some(source) = sy.checked_mul(width).and_then(|base| base.checked_add(sx)).and_then(|pixel| pixel.checked_mul(4)) else {
            continue;
        };
        let Some(end) = source.checked_add(4) else { continue };
        if let Some(pixel) = rgba.get(source..end) {
            destination.copy_from_slice(pixel);
        }
    }
}

/// `destination * source / destination_len`, in u64 so a 6000 px axis cannot overflow. The
/// caller only passes non-zero lengths.
fn scaled_index(destination: usize, destination_len: usize, source_len: usize) -> usize {
    let numerator = (destination as u64).saturating_mul(source_len as u64);
    usize::try_from(numerator / destination_len.max(1) as u64).unwrap_or(usize::MAX)
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
    // Decode into a buffer we allocate: it is 4-byte aligned (so the window can adopt it as
    // `Color32` with no copy) and it replaces the decoder's own output allocation rather than
    // adding one. The headers give the exact RGBA byte count before any entropy decode.
    decoder.decode_headers()?;
    let Some(info) = decoder.info() else {
        return Err(DecodeError::SizeMismatch { width: 0, height: 0, expected: 0 });
    };
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    let Some(expected) = usize::from(info.width).checked_mul(usize::from(info.height)).and_then(|area| area.checked_mul(4)) else {
        return Err(DecodeError::SizeMismatch { width, height, expected: usize::MAX });
    };
    let mut rgba = RgbaBuffer::zeroed(expected);
    decoder.decode_into(rgba.as_mut_slice())?;
    Ok(DecodedImage { width, height, rgba })
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
        return Ok(DecodedImage {
            width: u32::try_from(out_width).unwrap_or(0),
            height: u32::try_from(out_height).unwrap_or(0),
            rgba: RgbaBuffer::zeroed(0),
        });
    }
    let row_bytes = out_width.checked_mul(4).ok_or(DecodeError::OrientationOverflow { width: u32::MAX, height: u32::MAX, orientation })?;
    let source = rgba.as_slice();
    let mut out = RgbaBuffer::zeroed(expected);
    // Every EXIF transform walks the source in a straight line for one destination row:
    // `source = base + dx * step`. Hoisting that origin and step out of the pixel loop is what
    // keeps a transposing rotate cheap — the inner loop is one add, one checked 4-byte load and
    // one 4-byte store, instead of a match plus two checked multiplications per pixel. The
    // destination is still written a whole contiguous row at a time.
    let column = width.checked_mul(4).and_then(|bytes| isize::try_from(bytes).ok()).ok_or(DecodeError::OrientationOverflow {
        width: u32::MAX,
        height: u32::MAX,
        orientation,
    })?;
    // The step is in bytes: one destination pixel is four of them.
    let step: isize = match orientation {
        2 | 3 => -4,
        5 | 8 => column,
        6 | 7 => -column,
        _ => 4,
    };
    let last_row = height.saturating_sub(1);
    let last_col = width.saturating_sub(1);
    out.as_mut_slice().par_chunks_mut(row_bytes).enumerate().for_each(|(dy, row)| {
        // The source pixel under the destination row's first column.
        let mut index = match orientation {
            2 => dy.saturating_mul(width).saturating_add(last_col),
            3 => last_row.saturating_sub(dy).saturating_mul(width).saturating_add(last_col),
            4 => last_row.saturating_sub(dy).saturating_mul(width),
            5 => dy,
            6 => last_row.saturating_mul(width).saturating_add(dy),
            7 => last_row.saturating_mul(width).saturating_add(last_col.saturating_sub(dy)),
            8 => last_col.saturating_sub(dy),
            _ => dy.saturating_mul(width),
        }
        .saturating_mul(4);
        for pixel in row.as_chunks_mut::<4>().0 {
            // A geometry that walks off the source leaves the pixel zeroed instead of panicking;
            // the checked dimensions above make that unreachable for a real frame.
            if let Some(from) = source.get(index..index.saturating_add(4)) {
                pixel.copy_from_slice(from);
            }
            index = index.wrapping_add_signed(step);
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

#[cfg(test)]
mod tests {
    use super::select_display_source;
    use firstcut_meta::meta::{ByteRange, EmbeddedPreview};

    fn embedded(width: u32, height: u32) -> EmbeddedPreview {
        EmbeddedPreview { range: ByteRange { offset: 0, len: 0 }, width, height }
    }

    #[test]
    fn the_smallest_embedded_jpeg_that_covers_the_target_is_the_one_decoded() {
        let preview = embedded(1620, 1080);
        let full = embedded(6000, 4000);
        assert_eq!(select_display_source(Some(preview), Some(full), 1500).map(|image| image.width), Some(1620));
        assert_eq!(select_display_source(Some(preview), Some(full), 1620).map(|image| image.width), Some(1620));
        assert_eq!(select_display_source(Some(preview), Some(full), 1621).map(|image| image.width), Some(6000));
        assert_eq!(select_display_source(Some(preview), Some(full), 3240).map(|image| image.width), Some(6000));
    }

    #[test]
    fn a_preview_only_file_still_has_an_image() {
        let preview = embedded(1620, 1080);
        assert_eq!(select_display_source(Some(preview), None, 4000).map(|image| image.width), Some(1620));
        assert!(select_display_source(None, None, 4000).is_none());
    }
}
