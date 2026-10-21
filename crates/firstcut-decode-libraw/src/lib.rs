//! LibRaw bindings backend for [`firstcut_decode`]: develops RAW sensor data through
//! LibRaw (approved C/C++ dependency, decision D3 in docs/v2-plan.md).
//!
//! THE SINGLE `unsafe` CARVE-OUT OF THE WORKSPACE — docs/v2-plan.md §4.5. The
//! workspace forbids `unsafe_code` everywhere; this crate's own `[lints.rust]`
//! table (in Cargo.toml) is the one permitted exception, and `cargo xtask layers`
//! fails CI if any other crate carries its own lints table. Unsafe code here must
//! stay confined to FFI bindings and be shadowed by checked, safe wrappers the
//! rest of the ladder calls.
//!
//! # How this backend is wired
//!
//! `rsraw` vendors LibRaw 0.21.1 and builds it with `cc`. Its build script refuses
//! MSVC, so the dependency is target-gated to macOS and Linux (Cargo.toml) and
//! [`backend`] answers `None` on Windows. The Exact RAW tier then falls back to the
//! camera's embedded JPEG instead of losing the loupe; that is a recorded platform
//! gap (docs/v2-plan.md §6), not a silent one.
//!
//! Orientation is applied **exactly once**, here: LibRaw is told to develop in
//! sensor orientation (`user_flip = 0`) and the already-oriented
//! [`firstcut_decode::apply_orientation`] seam rotates the result to display
//! orientation. Rotating inside LibRaw as well would rotate orientations 5–8 twice.

use std::sync::Arc;

use firstcut_decode::RawDecode;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::path::Path;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use firstcut_decode::{DecodeError, DecodedImage, apply_orientation};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use firstcut_meta::meta::PhotoMeta;

/// The develop backend on platforms where the vendored LibRaw can build, `None` elsewhere.
///
/// Callers attach this to a decode tier; a `None` is a platform fact, not a failure.
pub fn backend() -> Option<Arc<dyn RawDecode>> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        Some(Arc::new(LibRaw))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct LibRaw;

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl RawDecode for LibRaw {
    fn decode_raw(&self, path: &Path, meta: &PhotoMeta) -> Result<DecodedImage, DecodeError> {
        let bytes = std::fs::read(path).map_err(|source| DecodeError::Read { path: path.display().to_string(), source })?;
        let image = develop(&bytes, &path.display().to_string())?;
        apply_orientation(image, meta.orientation)
    }
}

/// Develops sensor data at full resolution into RGBA8, in the sensor's own orientation.
///
/// Kept separate from [`RawDecode`] so the orientation rule has one seam: the trait impl above
/// is the only caller that rotates. Everything here is checked arithmetic and length checks
/// against what LibRaw reports, so a malformed file errors instead of indexing past a buffer.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn develop(bytes: &[u8], path: &str) -> Result<DecodedImage, DecodeError> {
    let mut raw = rsraw::RawImage::open(bytes).map_err(|err| DecodeError::Raw { path: path.to_owned(), message: err.to_string() })?;
    // The camera's own white balance and colour matrix, which is the rendering its JPEG preview
    // approximates; LibRaw's default output space is sRGB.
    raw.set_use_camera_wb(true);
    // Develop unrotated: `apply_orientation` is the single orientation seam.
    raw.as_mut().params.user_flip = 0;
    raw.unpack().map_err(|err| DecodeError::Raw { path: path.to_owned(), message: err.to_string() })?;
    let processed = raw.process::<8>().map_err(|err| DecodeError::Raw { path: path.to_owned(), message: err.to_string() })?;

    let width = processed.width();
    let height = processed.height();
    let colors = processed.colors();
    let bits = processed.bits();
    let pixels = pixel_count(width, height);
    // LibRaw hands us an sRGB bitmap for `process`: three channels, eight bits. Anything else is a
    // backend/library disagreement, reported instead of guessed at.
    let expected = pixels
        .filter(|_| rsraw::ImageFormat::Bitmap == processed.image_format() && bits == 8 && (colors == 3 || colors == 4))
        .and_then(|count| count.checked_mul(usize::from(colors)));
    let Some(expected) = expected else { return Err(DecodeError::RawLayout { path: path.to_owned(), width, height, colors }) };
    let rgb: &[u8] = &processed;
    if rgb.len() != expected {
        return Err(DecodeError::RawLayout { path: path.to_owned(), width, height, colors });
    }
    let capacity =
        pixels.and_then(|count| count.checked_mul(4)).ok_or_else(|| DecodeError::RawLayout { path: path.to_owned(), width, height, colors })?;
    let mut rgba = Vec::with_capacity(capacity);
    if colors == 4 {
        rgba.extend_from_slice(rgb);
    } else {
        // `chunks_exact(3)` cannot yield a short tail; the length check above already proved the
        // buffer is exactly `pixels * 3`.
        for [r, g, b] in rgb.as_chunks::<3>().0 {
            rgba.extend_from_slice(&[*r, *g, *b, 255]);
        }
    }
    if rgba.len() != capacity {
        return Err(DecodeError::RawLayout { path: path.to_owned(), width, height, colors });
    }
    Ok(DecodedImage { width, height, rgba })
}

/// `width * height` as a `usize`, `None` when either side does not fit the host.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn pixel_count(width: u32, height: u32) -> Option<usize> {
    usize::try_from(width).ok()?.checked_mul(usize::try_from(height).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backend_is_attached_where_libraw_can_build() {
        let backend = backend();
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            assert!(backend.is_some(), "LibRaw must back the Exact RAW tier on macOS and Linux");
        } else {
            assert!(backend.is_none(), "other platforms fall back to the embedded JPEG, deliberately");
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_file_that_is_not_a_raw_is_an_error_not_a_crash() {
        let error = develop(b"this is not a RAW file", "synthetic.cr3").expect_err("garbage must not develop");
        assert!(matches!(error, DecodeError::Raw { .. }), "expected a typed RAW error, got {error:?}");
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn pixel_count_refuses_a_dimension_that_cannot_fit() {
        assert_eq!(pixel_count(6000, 4000), Some(24_000_000));
        assert_eq!(pixel_count(0, 4000), Some(0));
    }
}
