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

pub mod fused;
pub mod profile;

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;

use firstcut_decode::{DecodeError, DecodedImage, RawDecode, RgbaBuffer, apply_orientation};
use firstcut_meta::meta::PhotoMeta;
use rayon::prelude::*;

/// The exact-RAW backend. It is attached on every platform; `None` is kept in the
/// signature because the decode tier's seam is optional and some future platform may
/// have no developer at all.
pub fn backend() -> Option<Arc<dyn RawDecode>> {
    Some(Arc::new(ExactRaw))
}

struct ExactRaw;

/// What the fast developer produced: a finished frame (the fused path), or rawler's intermediate
/// and the crop rectangle the pack still has to read it through.
enum Fused {
    Image(DecodedImage),
    Rawler(rawler::imgop::develop::Intermediate, Option<rawler::imgop::Rect>),
}

impl RawDecode for ExactRaw {
    fn decode_raw(&self, path: &Path, meta: &PhotoMeta) -> Result<DecodedImage, DecodeError> {
        // `develop_fast` applies the orientation inside its fused pack pass; `develop_libraw`
        // applies it through `apply_orientation`. Each path rotates exactly once.
        match develop_fast(path, meta.orientation) {
            Ok(image) => Ok(image),
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

/// The exact develop: [`fused::Plan`] when it accepts the file, rawler's own pipeline otherwise.
///
/// The fused path runs the whole pixel pipeline — PPG demosaic, white balance, camera matrix,
/// sRGB encode, pack and orientation — as three plane passes and one output pass, instead of
/// rawler's six passes over a 288 MB interleaved RGB buffer. It declines anything it has not been
/// proven on (`fused::Plan::new` returns `None`), and then this function runs rawler's pipeline
/// exactly as before: demosaic, white balance and calibrate, with the crop read in place
/// ([`LinearFrame`]) and the sRGB encode fused into the pack and the rotate ([`pack_frame`]).
///
/// The returned pixels are display-ready RGBA in **display** orientation: this is the whole
/// develop, and the trait impl above adds nothing.
fn develop_fast(path: &Path, orientation: u8) -> Result<DecodedImage, DecodeError> {
    use rawler::imgop::develop::{Intermediate, ProcessingStep::*, RawDevelop};
    let display = path.display().to_string();
    // rawler has a `panic!`/`todo!` on malformed layouts. A dependency may not take
    // the process down; the catch turns it into the same typed error as a refusal.
    let developed = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut raw = rawler::decode_file(path).map_err(|error| error.to_string())?;
        // The fused path wants the corrected CFA plane and nothing else, so it scales the decoded
        // frame in place instead of letting `develop_intermediate` clone it first. Scaling twice
        // is a no-op (it resets the levels it consumed), so a fused refusal can still fall through
        // to rawler below with the same buffer.
        if let Some(plan) = fused::Plan::new(&raw, orientation) {
            raw.apply_scaling().map_err(|error| error.to_string())?;
            let plane = raw.data.as_f32();
            if let Some(image) = plan.develop(&plane) {
                return Ok::<_, String>(Fused::Image(image));
            }
        }
        let steps = [Rescale, Demosaic, FujiRotate, CropActiveArea, WhiteBalance, Calibrate];
        let intermediate = RawDevelop::new_with(&steps).develop_intermediate(&raw).map_err(|error| error.to_string())?;
        Ok::<_, String>(Fused::Rawler(intermediate, output_crop(&raw)))
    }));
    let developed = match developed {
        Ok(Ok(developed)) => developed,
        Ok(Err(message)) => return Err(DecodeError::Raw { path: display, message }),
        Err(_) => return Err(DecodeError::Raw { path: display, message: "the fast RAW developer panicked".to_string() }),
    };
    let (intermediate, crop) = match developed {
        Fused::Image(image) => return Ok(image),
        Fused::Rawler(intermediate, crop) => (intermediate, crop),
    };

    match intermediate {
        Intermediate::ThreeColor(pixels) => {
            let whole = LinearFrame::whole(&pixels.data, pixels.width, pixels.height);
            let frame = crop.map(|area| (area.p.x, area.p.y, area.d.w, area.d.h)).and_then(|(x, y, w, h)| LinearFrame::crop(whole, x, y, w, h));
            // A crop rectangle that does not fit the demosaiced frame is a layout this code has
            // not seen; fall back to the unmodified frame rather than guess at the geometry.
            match frame.or(Some(whole)) {
                Some(frame) => pack_frame(frame, orientation, &display),
                None => pack_srgb_oriented(&pixels.data, pixels.width, pixels.height, orientation, &display),
            }
        }
        Intermediate::FourColor(pixels) => {
            let rgb = pixels.data.par_iter().map(|pixel| [pixel[0], pixel[1], pixel[2]]).collect::<Vec<_>>();
            pack_srgb_oriented(&rgb, pixels.width, pixels.height, orientation, &display)
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

/// The camera's output rectangle, expressed **inside the active area** the demosaic restricted
/// itself to. This is the rectangle rawler's `CropDefault` step copies out; reading it in place is
/// how this backend avoids the copy. `None` when the file declares no crop at all.
fn output_crop(raw: &rawler::RawImage) -> Option<rawler::imgop::Rect> {
    raw.crop_area.or(raw.active_area).map(|mut area| {
        if let Some(active) = raw.active_area.as_ref() {
            area = area.adapt(active);
        }
        area
    })
}

fn channel(value: f32) -> u8 {
    if value.is_finite() { (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 } else { 0 }
}

/// How finely the linear-light value is quantised before the sRGB table looks it up. 16 bits is
/// far more than the 8-bit output can see: the steepest part of the sRGB curve (its linear toe)
/// moves the output by 3295 levels per unit of linear light, so a 1/65535 step moves it by 0.05
/// of one level. The table is built from rawler's own gamma function, so the encode itself is the
/// same curve the rest of the pipeline was measured against.
const SRGB_LUT_STEPS: usize = 1 << 16;

/// A rectangle of linear-light pixels inside a wider demosaiced frame.
///
/// rawler demosaics the active area into one buffer and then copies the camera's output rectangle
/// out of it (its `CropDefault` step, measured at 22 ms on 24 MP — a 96 MB copy). Reading the
/// rectangle in place here removes that pass instead of making it faster.
#[derive(Clone, Copy)]
struct LinearFrame<'a> {
    pixels: &'a [[f32; 3]],
    /// Pixels per row of the *whole* demosaiced buffer.
    stride: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl<'a> LinearFrame<'a> {
    fn whole(pixels: &'a [[f32; 3]], width: usize, height: usize) -> Self {
        Self { pixels, stride: width, x: 0, y: 0, width, height }
    }

    /// The frame inside `whole` at `(x, y)` sized `width × height`, or `None` when the rectangle
    /// does not fit — a malformed crop must fall back, never index past the buffer.
    fn crop(whole: Self, x: usize, y: usize, width: usize, height: usize) -> Option<Self> {
        let right = x.checked_add(width)?;
        let bottom = y.checked_add(height)?;
        let fits = right <= whole.width && bottom <= whole.height && whole.stride.checked_mul(whole.height)? <= whole.pixels.len();
        fits.then_some(Self { x: whole.x.checked_add(x)?, y: whole.y.checked_add(y)?, width, height, ..whole })
    }

    /// The pixel at `(x, y)` inside this frame.
    fn get(&self, x: usize, y: usize) -> Option<&'a [f32; 3]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let row = self.y.checked_add(y)?.checked_mul(self.stride)?;
        let index = row.checked_add(self.x.checked_add(x)?)?;
        self.pixels.get(index)
    }
}

/// Encode + pack + rotate, in one destination-oriented pass.
///
/// Three separate passes used to do this — rawler's `sRGB` step (a `powf` per channel, 44 ms on a
/// 24 MP frame), the f32 → RGBA pack (12 ms), and `firstcut_decode::apply_orientation` (32 ms) —
/// and each one rewrote the whole frame. Fusing them costs one read of the linear pixels and one
/// write of the upright RGBA buffer, with the gamma encode reduced to a table lookup. The pixel
/// values are the same: the table is built from `rawler`'s own `srgb_apply_gamma`, and the index
/// quantises the *input* to 16 bits, which is invisible at 8-bit output.
fn pack_srgb_oriented(pixels: &[[f32; 3]], width: usize, height: usize, orientation: u8, path: &str) -> Result<DecodedImage, DecodeError> {
    let expected =
        width.checked_mul(height).ok_or_else(|| DecodeError::RawLayout { path: path.to_owned(), width: u32::MAX, height: 0, colors: 3 })?;
    if pixels.len() != expected {
        return Err(DecodeError::RawLayout { path: path.to_owned(), width: u32::MAX, height: 0, colors: 3 });
    }
    pack_frame(LinearFrame::whole(pixels, width, height), orientation, path)
}

/// [`pack_srgb_oriented`] over a rectangle of a wider demosaiced frame.
fn pack_frame(frame: LinearFrame<'_>, orientation: u8, path: &str) -> Result<DecodedImage, DecodeError> {
    let (width, height) = (frame.width, frame.height);
    let width_u32 = u32::try_from(width).map_err(|_| DecodeError::RawLayout { path: path.to_owned(), width: u32::MAX, height: 0, colors: 3 })?;
    let height_u32 =
        u32::try_from(height).map_err(|_| DecodeError::RawLayout { path: path.to_owned(), width: width_u32, height: u32::MAX, colors: 3 })?;
    let layout = |path: &str| DecodeError::RawLayout { path: path.to_owned(), width: width_u32, height: height_u32, colors: 3 };
    let transposed = (5..=8).contains(&orientation);
    let (out_width, out_height) = if transposed { (height, width) } else { (width, height) };
    let out_width_u32 = u32::try_from(out_width).map_err(|_| layout(path))?;
    let out_height_u32 = u32::try_from(out_height).map_err(|_| layout(path))?;
    let capacity = out_width.checked_mul(out_height).and_then(|area| area.checked_mul(4)).ok_or_else(|| layout(path))?;
    let row_bytes = out_width.checked_mul(4).ok_or_else(|| layout(path))?;
    let mut rgba = RgbaBuffer::zeroed(capacity);
    if row_bytes == 0 || out_width == 0 || out_height == 0 {
        return Ok(DecodedImage { width: out_width_u32, height: out_height_u32, rgba });
    }

    let lut = srgb_lut();
    match orientation {
        // No rotate, or a ±1-in-x walk: one destination row per task, both sides sequential
        // except at the ends of a mirrored row, which the cache absorbs.
        0..=4 => {
            rgba.as_mut_slice().par_chunks_exact_mut(row_bytes).enumerate().for_each(|(dy, row)| {
                let source_y = match orientation {
                    3 | 4 => height.saturating_sub(1).saturating_sub(dy),
                    _ => dy,
                };
                let mirrored = orientation == 2 || orientation == 3;
                for (dx, dst) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let source_x = if mirrored { width.saturating_sub(1).saturating_sub(dx) } else { dx };
                    if let Some(pixel) = frame.get(source_x, source_y) {
                        dst.copy_from_slice(&[lut_value(lut, pixel[0]), lut_value(lut, pixel[1]), lut_value(lut, pixel[2]), 255]);
                    }
                }
            });
        }
        // 5..=8 swap the axes: a destination row reads a source *column*, one cache line per
        // pixel, which measured 54 ms when the walk ran the full frame height. Blocking both
        // dimensions (64 destination rows × 64 columns) keeps the 64-row source tile a block
        // reads resident while it writes 64 contiguous pixels of each of its rows.
        _ => {
            const TILE_ROWS: usize = 64;
            const TILE_COLS: usize = 64;
            let tile_bytes = row_bytes.saturating_mul(TILE_ROWS);
            rgba.as_mut_slice().par_chunks_mut(tile_bytes).enumerate().for_each(|(tile, band)| {
                let dy_start = tile.saturating_mul(TILE_ROWS);
                let rows = out_height.saturating_sub(dy_start).min(TILE_ROWS);
                let mut dx_start = 0;
                while dx_start < out_width {
                    let columns = out_width.saturating_sub(dx_start).min(TILE_COLS);
                    for row_in_tile in 0..rows {
                        let dy = dy_start.saturating_add(row_in_tile);
                        // The source position for this destination row's first column, and the
                        // walk down the columns: every transposing orientation reads one source
                        // row per destination column.
                        let (sx, sy_start, step) = match orientation {
                            5 => (dy, dx_start, 1i64),
                            6 => (dy, height.saturating_sub(1).saturating_sub(dx_start), -1),
                            7 => (width.saturating_sub(1).saturating_sub(dy), height.saturating_sub(1).saturating_sub(dx_start), -1),
                            _ => (width.saturating_sub(1).saturating_sub(dy), dx_start, 1),
                        };
                        let write_at = row_in_tile.saturating_mul(row_bytes).saturating_add(dx_start.saturating_mul(4));
                        let mut sy = i64::try_from(sy_start).unwrap_or(i64::MAX);
                        for column in 0..columns {
                            let destination = write_at.saturating_add(column.saturating_mul(4));
                            let source_y = usize::try_from(sy).unwrap_or(usize::MAX);
                            if let (Some(pixel), Some(slot)) = (frame.get(sx, source_y), band.get_mut(destination..destination.saturating_add(4))) {
                                slot.copy_from_slice(&[lut_value(lut, pixel[0]), lut_value(lut, pixel[1]), lut_value(lut, pixel[2]), 255]);
                            }
                            sy = sy.saturating_add(step);
                        }
                    }
                    dx_start = dx_start.saturating_add(TILE_COLS);
                }
            });
        }
    }
    if rgba.len() != capacity {
        return Err(layout(path));
    }
    Ok(DecodedImage { width: out_width_u32, height: out_height_u32, rgba })
}

/// Linear light → 8-bit sRGB, through a table built from rawler's gamma curve.
pub(crate) fn lut_value(lut: &[u8], value: f32) -> u8 {
    if !value.is_finite() {
        return 0;
    }
    let scaled = value.clamp(0.0, 1.0) * (SRGB_LUT_STEPS - 1) as f32;
    let index = (scaled + 0.5) as usize;
    lut.get(index.min(SRGB_LUT_STEPS - 1)).copied().unwrap_or(0)
}

/// The sRGB encode table, built once per process.
pub(crate) fn srgb_lut() -> &'static [u8] {
    static LUT: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        (0..SRGB_LUT_STEPS).map(|step| channel(rawler::imgop::srgb::srgb_apply_gamma(step as f32 / (SRGB_LUT_STEPS - 1) as f32))).collect()
    })
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
    let mut rgba = RgbaBuffer::zeroed(capacity);
    if colors == 4 {
        rgba.as_mut_slice().copy_from_slice(rgb);
    } else {
        // `chunks_exact(3)` cannot yield a short tail; the length check above already proved the
        // buffer is exactly `pixels * 3`.
        rgba.as_mut_slice().par_chunks_exact_mut(4).zip(rgb.as_chunks::<3>().0.par_iter()).for_each(|(destination, [r, g, b])| {
            destination.copy_from_slice(&[*r, *g, *b, 255]);
        });
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
    fn the_fused_pack_keeps_pixel_order_and_clamps() {
        let pixels = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.5, 0.5, 0.5]];
        let image = pack_srgb_oriented(&pixels, 2, 2, 1, "synthetic").expect("a 2x2 pack is valid");
        assert_eq!(image.width, 2);
        assert_eq!(image.height, 2);
        assert_eq!(&image.rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&image.rgba[4..8], &[0, 255, 0, 255]);
        assert_eq!(&image.rgba[8..12], &[0, 0, 255, 255]);
        let mid = channel(rawler::imgop::srgb::srgb_apply_gamma(0.5));
        assert_eq!(&image.rgba[12..16], &[mid, mid, mid, 255], "the grey keeps the gamma curve's value");
        assert_eq!(channel(f32::NAN), 0, "a non-finite channel is black, not a crash");
        assert_eq!(channel(2.0), 255, "out-of-range channels clamp");
    }

    #[test]
    fn a_file_that_is_not_a_raw_is_an_error_not_a_crash_in_the_fast_developer() {
        let error = develop_fast(Path::new("synthetic.cr3"), 1).expect_err("garbage must not develop");
        assert!(matches!(error, DecodeError::Raw { .. }), "expected a typed RAW error, got {error:?}");
    }

    /// The fused tail must be the same pixels as a pack followed by `apply_orientation`, which is
    /// the definition of correct orientation in this workspace. Byte equality, all eight values.
    #[test]
    fn the_fused_encode_pack_and_rotate_matches_a_pack_then_rotate() {
        let (width, height) = (3, 2);
        let pixels: Vec<[f32; 3]> = (0..width * height)
            .map(|i| {
                let v = i as f32 / (width * height) as f32;
                [v, 1.0 - v, (v * 4.0).fract()]
            })
            .collect();
        let upright = pack_srgb_oriented(&pixels, width, height, 1, "synthetic").expect("the unrotated frame packs");
        for orientation in 1..=8u8 {
            let fused = pack_srgb_oriented(&pixels, width, height, orientation, "synthetic").expect("every orientation packs");
            let reference = firstcut_decode::apply_orientation(upright.clone(), orientation).expect("the reference rotate accepts 1..=8");
            assert_eq!((fused.width, fused.height), (reference.width, reference.height), "orientation {orientation} has the wrong frame size");
            assert_eq!(fused.rgba.as_slice(), reference.rgba.as_slice(), "orientation {orientation} moved a pixel");
        }
    }

    /// The table is built from rawler's own gamma curve, so its only error is the 16-bit index.
    /// The bound is checked at the curve's steepest point, which is the linear toe.
    #[test]
    fn the_srgb_table_is_within_one_level_of_the_real_curve() {
        let lut = srgb_lut();
        let mut worst = 0i32;
        for step in 0..=4096 {
            let value = step as f32 / 4096.0;
            let expected = i32::from(channel(rawler::imgop::srgb::srgb_apply_gamma(value)));
            let actual = i32::from(lut_value(lut, value));
            worst = worst.max((expected - actual).abs());
        }
        assert!(worst <= 1, "the table drifted {worst} levels from rawler's curve");
        assert_eq!(lut_value(lut, f32::NAN), 0, "a non-finite channel is black, not a crash");
        assert_eq!(lut_value(lut, 2.0), 255, "out-of-range channels clamp");
    }

    /// A frame size that cannot be represented must error, not panic or wrap.
    #[test]
    fn the_fused_pass_refuses_a_length_that_does_not_match_the_dimensions() {
        let pixels = [[0.0f32; 3]; 3];
        let error = pack_srgb_oriented(&pixels, 2, 2, 1, "synthetic").expect_err("3 pixels is not a 2x2 frame");
        assert!(matches!(error, DecodeError::RawLayout { .. }), "expected a layout error, got {error:?}");
    }

    /// Reading the camera's output rectangle in place must be the same picture as copying it out
    /// first — that copy is the pass this replaced, so this is the test that says it was safe.
    #[test]
    fn the_fused_pass_reads_a_crop_rectangle_as_if_it_had_been_copied() {
        let (width, height) = (5, 4);
        let pixels: Vec<[f32; 3]> = (0..width * height)
            .map(|i| {
                let v = i as f32 / (width * height) as f32;
                [v, (v * 3.0).fract(), (v * 7.0).fract()]
            })
            .collect();
        let whole = LinearFrame::whole(&pixels, width, height);
        let frame = LinearFrame::crop(whole, 1, 2, 3, 2).expect("a 3x2 window at (1,2) fits a 5x4 frame");
        let in_place = pack_frame(frame, 1, "synthetic").expect("the window packs");

        let mut copied = Vec::with_capacity(3 * 2);
        for y in 2..4 {
            for x in 1..4 {
                if let Some(pixel) = pixels.get(y * width + x) {
                    copied.push(*pixel);
                }
            }
        }
        let by_copy = pack_srgb_oriented(&copied, 3, 2, 1, "synthetic").expect("the copy packs");
        assert_eq!(in_place.width, by_copy.width);
        assert_eq!(in_place.height, by_copy.height);
        assert_eq!(in_place.rgba.as_slice(), by_copy.rgba.as_slice());
    }

    /// A rectangle that does not fit must be refused, so the caller can fall back instead of
    /// reading past the buffer.
    #[test]
    fn a_crop_rectangle_outside_the_frame_is_refused() {
        let pixels = [[0.0f32; 3]; 25];
        let whole = LinearFrame::whole(&pixels, 5, 5);
        assert!(LinearFrame::crop(whole, 3, 0, 5, 5).is_none(), "a window that runs off the right edge is refused");
        assert!(LinearFrame::crop(whole, 0, 3, 5, 5).is_none(), "a window that runs off the bottom is refused");
        assert!(LinearFrame::crop(whole, 0, 0, 5, 5).is_some(), "the whole frame is a valid window");
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
