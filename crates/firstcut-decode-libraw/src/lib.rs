//! Native exact-RAW backends for [`firstcut_decode`]: the fast rawler developer,
//! with LibRaw as the fallback on macOS and Linux (decision D3, docs/v2-plan.md).
//!
//! THE SINGLE `unsafe` CARVE-OUT OF THE WORKSPACE — docs/v2-plan.md §4.5. The
//! workspace forbids `unsafe_code` everywhere; this crate's own `[lints.rust]`
//! table (in Cargo.toml) is the one permitted exception, and `cargo xtask layers`
//! fails CI if any other crate carries its own lints table. Unsafe code here must
//! stay confined to FFI bindings and be shadowed by checked, safe wrappers the
//! rest of the ladder calls. rawler is safe Rust; the carve-out exists for the
//! LibRaw bindings underneath it.
//!
//! # How this backend is wired
//!
//! [`backend`] always answers `Some`:
//!
//! * **rawler** is the exact-RAW developer on every platform. It is pure Rust, so
//!   Windows gets real sensor data instead of falling back to the camera JPEG; on
//!   the benchmark machine it develops a 6000×4000 CR3 in ~0.34 s versus LibRaw's
//!   ~1.03 s. That is the M5 cross-platform fix and the §5 exact-RAW improvement.
//! * **LibRaw** (via `rsraw`, vendored 0.21.1) is the fallback when rawler refuses
//!   a file or a layout. `rsraw` refuses MSVC, so it is target-gated to macOS and
//!   Linux; on Windows a rawler failure is reported as a typed error and the
//!   pipeline shows the embedded JPEG.
//!
//! Orientation is applied **exactly once**, in [`RawDecode::decode_raw`]: both
//! developers produce sensor orientation and the already-oriented
//! [`firstcut_decode::apply_orientation`] seam rotates the result to display
//! orientation. Rotating inside a developer as well would rotate orientations 5–8
//! twice.

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;

use firstcut_decode::{DecodeError, DecodedImage, RawDecode, apply_orientation};
use firstcut_meta::meta::PhotoMeta;

/// The exact-RAW backend. It is attached on every platform; `None` is kept in the
/// signature because the decode tier's seam is optional and some future platform may
/// have no developer at all.
pub fn backend() -> Option<Arc<dyn RawDecode>> {
    Some(Arc::new(ExactRaw))
}

struct ExactRaw;

impl RawDecode for ExactRaw {
    fn decode_raw(&self, path: &Path, meta: &PhotoMeta) -> Result<DecodedImage, DecodeError> {
        match develop_fast(path) {
            Ok(image) => apply_orientation(image, meta.orientation),
            Err(fast_error) => {
                #[cfg(any(target_os = "macos", target_os = "linux"))]
                {
                    // LibRaw is slower but covers formats rawler has not ported. The fast
                    // error is only surfaced if both refuse the file.
                    let _ = fast_error;
                    develop_libraw(path, meta)
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                {
                    Err(fast_error)
                }
            }
        }
    }
}

/// rawler's whole develop pipeline: decode, demosaic, calibrate to the camera's
/// matrix, and encode sRGB. It returns display-ready pixels in sensor orientation.
fn develop_fast(path: &Path) -> Result<DecodedImage, DecodeError> {
    let display = path.display().to_string();
    // rawler has a `panic!`/`todo!` on malformed layouts. A dependency may not take
    // the process down; the catch turns it into the same typed error as a refusal.
    let developed = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let raw = rawler::decode_file(path).map_err(|error| error.to_string())?;
        let intermediate = rawler::imgop::develop::RawDevelop::default().develop_intermediate(&raw).map_err(|error| error.to_string())?;
        Ok::<_, String>(intermediate)
    }));
    let intermediate = match developed {
        Ok(Ok(intermediate)) => intermediate,
        Ok(Err(message)) => return Err(DecodeError::Raw { path: display, message }),
        Err(_) => return Err(DecodeError::Raw { path: display, message: "the fast RAW developer panicked".to_string() }),
    };

    use rawler::imgop::develop::Intermediate;
    match intermediate {
        Intermediate::ThreeColor(pixels) => rgba_from_rgb(&pixels.data, pixels.width, pixels.height, &display),
        Intermediate::FourColor(pixels) => {
            let rgba = pixels.data.iter().map(|pixel| [pixel[0], pixel[1], pixel[2]]).collect::<Vec<_>>();
            rgba_from_rgb(&rgba, pixels.width, pixels.height, &display)
        }
        Intermediate::Monochrome(pixels) => {
            let dim = pixels.dim();
            Err(DecodeError::RawLayout {
                path: display,
                width: u32::try_from(dim.w).unwrap_or(u32::MAX),
                height: u32::try_from(dim.h).unwrap_or(u32::MAX),
                colors: 1,
            })
        }
    }
}

/// Turns rawler's f32 sRGB pixels into the RGBA8 buffer the rest of the app consumes.
fn rgba_from_rgb(pixels: &[[f32; 3]], width: usize, height: usize, path: &str) -> Result<DecodedImage, DecodeError> {
    let width_u32 = u32::try_from(width).map_err(|_| DecodeError::RawLayout { path: path.to_owned(), width: u32::MAX, height: 0, colors: 3 })?;
    let height_u32 =
        u32::try_from(height).map_err(|_| DecodeError::RawLayout { path: path.to_owned(), width: width_u32, height: u32::MAX, colors: 3 })?;
    let expected =
        width.checked_mul(height).ok_or_else(|| DecodeError::RawLayout { path: path.to_owned(), width: width_u32, height: height_u32, colors: 3 })?;
    if pixels.len() != expected {
        return Err(DecodeError::RawLayout { path: path.to_owned(), width: width_u32, height: height_u32, colors: 3 });
    }
    let capacity =
        expected.checked_mul(4).ok_or_else(|| DecodeError::RawLayout { path: path.to_owned(), width: width_u32, height: height_u32, colors: 3 })?;
    let mut rgba = Vec::with_capacity(capacity);
    for pixel in pixels {
        rgba.extend_from_slice(&[channel(pixel[0]), channel(pixel[1]), channel(pixel[2]), 255]);
    }
    if rgba.len() != capacity {
        return Err(DecodeError::RawLayout { path: path.to_owned(), width: width_u32, height: height_u32, colors: 3 });
    }
    Ok(DecodedImage { width: width_u32, height: height_u32, rgba })
}

fn channel(value: f32) -> u8 {
    if value.is_finite() { (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 } else { 0 }
}

/// The LibRaw fallback, kept unrotated so the trait impl above is the one rotation seam.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn develop_libraw(path: &Path, meta: &PhotoMeta) -> Result<DecodedImage, DecodeError> {
    let bytes = std::fs::read(path).map_err(|source| DecodeError::Read { path: path.display().to_string(), source })?;
    let image = develop(&bytes, &path.display().to_string())?;
    apply_orientation(image, meta.orientation)
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
    fn the_backend_is_attached_on_every_platform() {
        assert!(backend().is_some(), "rawler is pure Rust and backs the Exact RAW tier on all three OSes");
    }

    #[test]
    fn a_file_that_is_not_a_raw_is_an_error_not_a_crash_in_the_fast_developer() {
        let error = develop_fast(Path::new("synthetic.cr3")).expect_err("garbage must not develop");
        assert!(matches!(error, DecodeError::Raw { .. }), "expected a typed RAW error, got {error:?}");
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
