//! Where the Exact RAW seconds actually go.
//!
//! `bench --decode --stages` needs a breakdown it can trust before anyone spends a week on a
//! decoder: the plan's §5 row records "decode 0.12 + develop 0.18 + pack/orientation 0.04", but
//! develop is itself several passes over 24 MP and the only way to know which one to attack is to
//! time them on the same file in the same process.
//!
//! Rawler exposes its pipeline as a list of `ProcessingStep`s, so each develop stage is measured as
//! the difference between two runs of `develop_intermediate` that differ by exactly that step. The
//! numbers are cumulative-run differences, not isolated microbenchmarks: they include the work the
//! earlier steps must do anyway, which is what the real develop pays. The tail is timed on the
//! frame the production path actually packs, and the last row is the real
//! [`firstcut_decode::RawDecode::decode_raw`] call, so the table cannot drift from production.
//!
//! Measurement only: nothing on the interactive path calls this module, and it is compiled into
//! release builds because the bench must time the shipped profile (`lto = "fat"`).

use std::path::Path;
use std::time::Instant;

use firstcut_decode::DecodedImage;
use firstcut_meta::meta::PhotoMeta;
use rawler::RawImage;
use rawler::imgop::develop::{Intermediate, ProcessingStep::*, RawDevelop};

use crate::DecodeError;

/// One photograph's exact-RAW timeline, in seconds (warm: the caller has read the file).
#[derive(Clone, Debug, Default)]
pub struct StageTimings {
    /// Container parse plus the CRX entropy/wavelet decode — rawler's `decode_file`.
    pub decode: f64,
    /// u16 → f32 rescale of the whole sensor frame.
    pub rescale: f64,
    /// The demosaic itself (PPG for Bayer), including the active-area ROI.
    pub demosaic: f64,
    /// White balance, the camera matrix, and rawler's per-pixel clipping/normalisation.
    pub color: f64,
    /// The default crop copy to the camera's output rectangle.
    pub crop: f64,
    /// The fused tail without a rotate: sRGB gamma encode (table lookup) + f32 RGB → RGBA8.
    pub pack: f64,
    /// What the orientation adds on top of `pack`: the same fused pass, rotated.
    pub orientation: f64,
    /// The sum of the rows above.
    pub sum: f64,
    /// The real `decode_raw` call across the same file: the number the §5 row publishes.
    pub measured: f64,
    pub width: usize,
    pub height: usize,
    pub megapixels: f64,
    pub threads: usize,
}

impl StageTimings {
    /// Element-wise minimum, for a "best of N rounds" table.
    pub fn min_each(&self, other: &Self) -> Self {
        Self {
            decode: self.decode.min(other.decode),
            rescale: self.rescale.min(other.rescale),
            demosaic: self.demosaic.min(other.demosaic),
            color: self.color.min(other.color),
            crop: self.crop.min(other.crop),
            pack: self.pack.min(other.pack),
            orientation: self.orientation.min(other.orientation),
            sum: self.sum.min(other.sum),
            measured: self.measured.min(other.measured),
            width: self.width,
            height: self.height,
            megapixels: self.megapixels,
            threads: self.threads,
        }
    }

    /// The rows, largest first, as `(name, seconds)` — so the next optimisation is obvious.
    pub fn rows(&self) -> Vec<(&'static str, f64)> {
        let mut rows = vec![
            ("decode", self.decode),
            ("rescale", self.rescale),
            ("demosaic", self.demosaic),
            ("color", self.color),
            ("pack", self.pack),
            ("orientation", self.orientation),
        ];
        rows.sort_by(|a, b| b.1.total_cmp(&a.1));
        rows
    }
}

/// Times every stage of the exact path for one RAW file.
///
/// The file is decoded once and developed four times with a growing step list, so the difference
/// between consecutive runs is one stage. That is deliberately more work than a production develop
/// — the bench is not on the interactive path — but it is the only way to separate rawler's passes
/// without forking the dependency.
pub fn stage_timings(path: &Path, meta: &PhotoMeta) -> Result<StageTimings, DecodeError> {
    let display = path.display().to_string();
    let start = Instant::now();
    let raw = rawler::decode_file(path).map_err(|error| DecodeError::Raw { path: display.clone(), message: error.to_string() })?;
    let decode = start.elapsed().as_secs_f64();

    let rescale = time_develop(&raw, &[Rescale])?;
    let demosaic = time_develop(&raw, &[Rescale, Demosaic])?;
    let color = time_develop(&raw, &[Rescale, Demosaic, FujiRotate, CropActiveArea, WhiteBalance, Calibrate])?;
    // What rawler's `CropDefault` pass costs, measured rather than guessed: the same develop plus
    // that one step. Production no longer runs it, so the row is the pass the crop-in-place
    // removed.
    let cropped = time_develop(&raw, &[Rescale, Demosaic, FujiRotate, CropActiveArea, WhiteBalance, Calibrate, CropDefault])?;

    // The last develop repeats exactly what production runs, so the tail is timed on the real
    // frame and the real output rectangle.
    let linear = RawDevelop::new_with(&[Rescale, Demosaic, FujiRotate, CropActiveArea, WhiteBalance, Calibrate])
        .develop_intermediate(&raw)
        .map_err(|error| DecodeError::Raw { path: display.clone(), message: error.to_string() })?;
    let crop = crate::output_crop(&raw);
    let (pack, orientation, width, height) = time_tail(&linear, crop, meta.orientation, &display)?;

    let mut timings = StageTimings {
        decode,
        rescale,
        demosaic: (demosaic - rescale).max(0.0),
        color: (color - demosaic).max(0.0),
        crop: (cropped - color).max(0.0),
        pack,
        orientation,
        sum: 0.0,
        measured: 0.0,
        width,
        height,
        megapixels: (width as f64 * height as f64) / 1_000_000.0,
        threads: rayon::current_num_threads(),
    };
    timings.sum = timings.decode + timings.rescale + timings.demosaic + timings.color + timings.pack + timings.orientation;

    // The published number, measured the way the app and the §5 row measure it.
    let start = Instant::now();
    let measured = crate::backend()
        .ok_or_else(|| DecodeError::Raw { path: display.clone(), message: "no RAW backend on this platform".to_string() })?
        .decode_raw(path, meta)?;
    std::hint::black_box(&measured);
    timings.measured = start.elapsed().as_secs_f64();
    Ok(timings)
}

/// Runs one `develop_intermediate` with `steps` and returns its wall time in seconds.
fn time_develop(raw: &RawImage, steps: &[rawler::imgop::develop::ProcessingStep]) -> Result<f64, DecodeError> {
    let start = Instant::now();
    let intermediate = RawDevelop::new_with(steps)
        .develop_intermediate(raw)
        .map_err(|error| DecodeError::Raw { path: "<stage timing>".to_string(), message: error.to_string() })?;
    let seconds = start.elapsed().as_secs_f64();
    std::hint::black_box(&intermediate);
    Ok(seconds)
}

/// Times the fused tail twice: once unrotated (gamma + pack) and once at the photograph's real
/// orientation, so the rotate's own share is the difference. The `crop` row above is *when* the
/// pass production no longer runs; the tail below reads the crop rectangle in place, as production
/// does.
fn time_tail(
    intermediate: &Intermediate,
    crop: Option<rawler::imgop::Rect>,
    orientation: u8,
    path: &str,
) -> Result<(f64, f64, usize, usize), DecodeError> {
    let (pixels, width, height) = match intermediate {
        Intermediate::ThreeColor(pixels) => (pixels.pixels().to_vec(), pixels.width, pixels.height),
        Intermediate::FourColor(pixels) => (pixels.pixels().iter().map(|p| [p[0], p[1], p[2]]).collect(), pixels.width, pixels.height),
        Intermediate::Monochrome(_) => {
            return Err(DecodeError::RawLayout { path: path.to_string(), width: 0, height: 0, colors: 1 });
        }
    };
    let whole = crate::LinearFrame::whole(&pixels, width, height);
    let frame = crop
        .map(|area| (area.p.x, area.p.y, area.d.w, area.d.h))
        .and_then(|(x, y, w, h)| crate::LinearFrame::crop(whole, x, y, w, h))
        .unwrap_or(whole);
    let (out_width, out_height) = (frame.width, frame.height);
    let start = Instant::now();
    let upright: DecodedImage = crate::pack_frame(frame, 1, path)?;
    let pack = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let oriented = crate::pack_frame(frame, orientation, path)?;
    let rotated = start.elapsed().as_secs_f64();
    std::hint::black_box(&upright);
    std::hint::black_box(&oriented);
    Ok((pack, (rotated - pack).max(0.0), out_width, out_height))
}
