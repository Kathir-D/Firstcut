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

/// A decoded frame: RGBA8, ready for a GPU texture or an egui `ColorImage`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, alpha 255.
    pub rgba: Vec<u8>,
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
}

/// Decodes the photograph's own pixels: the full-resolution embedded JPEG, falling back to the
/// smaller `PRVW` one. The result is the display image for the loupe — clipped or zoomed it
/// stays at this resolution until the Exact RAW tier lands (M2).
pub fn decode_display(path: &Path, meta: &PhotoMeta) -> Result<DecodedImage, DecodeError> {
    let shown = meta.full_preview.or(meta.preview).ok_or_else(|| DecodeError::NoEmbeddedJpeg { path: path.display().to_string() })?;
    read_range(path, shown.range)
}

/// Reads a file's byte range and decodes it as JPEG. The range is what `core-meta` recorded at
/// scan time; a file that changed underneath the session reports itself.
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

/// The full RAW decode seam a backend crate implements (M2's LibRaw bridge). The embedded-JPEG
/// path never calls in to this; the pipelines hold a `&dyn RawDecode` and choose per work item.
pub trait RawDecode: Send + Sync {
    /// Decodes `path` at the photo's own resolution. Errors are typed here, so a backend's C
    /// failures still surface the way the never-crash rule wants.
    fn decode_raw(&self, path: &Path, meta: &PhotoMeta) -> Result<DecodedImage, DecodeError>;
}
