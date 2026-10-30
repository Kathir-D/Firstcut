//! Where the Exact RAW seconds actually go.
//!
//! `bench --decode --stages` needs a breakdown it can trust before anyone spends a week on a
//! decoder: the plan's §5 row records "decode 0.12 + demosaic 0.10 + tail 0.07", and the only way
//! to know which part to attack is to time them on the same file in the same process.
//!
//! The rows are the pipeline production actually runs — [`crate::fused::Plan`] — measured stage by
//! stage on one decoded frame, and the last row is the real [`firstcut_decode::RawDecode::decode_raw`]
//! call, so the table cannot drift from production. Files the fused plan declines (a non-Bayer
//! sensor, a multi-component frame) fall back to rawler's own develop; that table is reported too,
//! because those files still have to be developed and their costs are not the fused ones.
//!
//! Measurement only: nothing on the interactive path calls this module, and it is compiled into
//! release builds because the bench must time the shipped profile (`lto = "fat"`).

use std::path::Path;
use std::time::Instant;

use firstcut_decode::apply_orientation;
use firstcut_meta::meta::PhotoMeta;
use rawler::RawImage;
use rawler::imgop::develop::{Intermediate, ProcessingStep::*, RawDevelop};

use crate::DecodeError;
use crate::fused;

/// One photograph's exact-RAW timeline, in seconds (warm: the caller has read the file).
#[derive(Clone, Debug, Default)]
pub struct StageTimings {
    /// Which pipeline produced the rows: `"fused"` or `"rawler"`.
    pub pipeline: &'static str,
    /// Container parse plus the CRX entropy/wavelet decode — rawler's `decode_file`.
    pub decode: f64,
    /// u16 → f32 rescale of the whole sensor frame, and reading it as one plane.
    pub rescale: f64,
    /// The demosaic itself (PPG for Bayer), including the active-area ROI.
    pub demosaic: f64,
    /// White balance, the camera matrix, rawler's clipping, the sRGB table and the RGBA pack —
    /// one pass over the crop rectangle, no rotate.
    pub pack: f64,
    /// What the orientation adds on top of `pack`: the same frame, rotated.
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
            pipeline: self.pipeline,
            decode: self.decode.min(other.decode),
            rescale: self.rescale.min(other.rescale),
            demosaic: self.demosaic.min(other.demosaic),
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
            ("pack", self.pack),
            ("orientation", self.orientation),
        ];
        rows.sort_by(|a, b| b.1.total_cmp(&a.1));
        rows
    }
}

/// Times every stage of the exact path for one RAW file.
///
/// The file is decoded once, then the fused plan's stages run in production order on that one
/// frame. The published `measured` row is a fresh `decode_raw` over the file, so it includes the
/// decode the stage rows measure separately.
pub fn stage_timings(path: &Path, meta: &PhotoMeta) -> Result<StageTimings, DecodeError> {
    let display = path.display().to_string();
    let start = Instant::now();
    let raw = rawler::decode_file(path).map_err(|error| DecodeError::Raw { path: display.clone(), message: error.to_string() })?;
    let decode = start.elapsed().as_secs_f64();

    let timings = match fused::Plan::new(&raw, meta.orientation) {
        Some(plan) => fused_stages(&raw, &plan, meta, path, decode, &display)?,
        None => rawler_stages(&raw, meta, path, decode, &display)?,
    };
    Ok(timings)
}

/// The production pipeline, stage by stage.
fn fused_stages(raw: &RawImage, plan: &fused::Plan, meta: &PhotoMeta, path: &Path, decode: f64, display: &str) -> Result<StageTimings, DecodeError> {
    // Production scales the decoded frame in place. The copy is outside the timed region: the
    // reference frame is kept untouched so the `measured` run below decodes a cold file exactly as
    // the app does.
    let mut scaled = raw.clone();
    let start = Instant::now();
    scaled.apply_scaling().map_err(|error| DecodeError::Raw { path: display.to_string(), message: error.to_string() })?;
    let plane = scaled.data.as_f32();
    let rescale = start.elapsed().as_secs_f64();

    let start = Instant::now();
    let planes = plan.demosaic(&plane).ok_or_else(|| declined(display))?;
    let demosaic = start.elapsed().as_secs_f64();

    let start = Instant::now();
    let packed = plan.pack(&planes).ok_or_else(|| declined(display))?;
    let pack = start.elapsed().as_secs_f64();

    let (width, height) = (packed.width as usize, packed.height as usize);
    let start = Instant::now();
    let upright = apply_orientation(packed, plan.orientation())?;
    let orientation = start.elapsed().as_secs_f64();
    std::hint::black_box(&upright);
    drop(planes);

    let mut timings = StageTimings {
        pipeline: "fused",
        decode,
        rescale,
        demosaic,
        pack,
        orientation,
        sum: 0.0,
        measured: 0.0,
        width,
        height,
        megapixels: (width as f64 * height as f64) / 1_000_000.0,
        threads: rayon::current_num_threads(),
    };
    timings.sum = timings.decode + timings.rescale + timings.demosaic + timings.pack + timings.orientation;
    timings.measured = measure(path, meta)?;
    Ok(timings)
}

/// rawler's own develop, for files the fused plan declines. Same shape, rawler's passes.
fn rawler_stages(raw: &RawImage, meta: &PhotoMeta, path: &Path, decode: f64, display: &str) -> Result<StageTimings, DecodeError> {
    let rescale = time_develop(raw, &[Rescale])?;
    let demosaic = time_develop(raw, &[Rescale, Demosaic])?;
    let color = time_develop(raw, &[Rescale, Demosaic, FujiRotate, CropActiveArea, WhiteBalance, Calibrate])?;

    // The last develop repeats what the fallback runs, so the tail is timed on the real frame and
    // the real output rectangle.
    let linear = RawDevelop::new_with(&[Rescale, Demosaic, FujiRotate, CropActiveArea, WhiteBalance, Calibrate])
        .develop_intermediate(raw)
        .map_err(|error| DecodeError::Raw { path: display.to_string(), message: error.to_string() })?;
    let crop = crate::output_crop(raw);
    let (pack, orientation, width, height) = time_tail(&linear, crop, meta.orientation, display)?;

    let mut timings = StageTimings {
        pipeline: "rawler",
        decode,
        rescale,
        demosaic: (demosaic - rescale).max(0.0),
        pack: (color - demosaic).max(0.0) + pack,
        orientation,
        sum: 0.0,
        measured: 0.0,
        width,
        height,
        megapixels: (width as f64 * height as f64) / 1_000_000.0,
        threads: rayon::current_num_threads(),
    };
    timings.sum = timings.decode + timings.rescale + timings.demosaic + timings.pack + timings.orientation;
    timings.measured = measure(path, meta)?;
    Ok(timings)
}

/// The published number, measured the way the app and the §5 row measure it.
fn measure(path: &Path, meta: &PhotoMeta) -> Result<f64, DecodeError> {
    let start = Instant::now();
    let measured = crate::backend()
        .ok_or_else(|| DecodeError::Raw { path: path.display().to_string(), message: "no RAW backend on this platform".to_string() })?
        .decode_raw(path, meta)?;
    std::hint::black_box(&measured);
    Ok(start.elapsed().as_secs_f64())
}

/// The fused plan accepted the file but could not develop this frame — a layout it could not
/// cover. The bench reports it instead of publishing a table that does not add up.
fn declined(path: &str) -> DecodeError {
    DecodeError::Raw { path: path.to_string(), message: "the fused plan declined a frame it had accepted".to_string() }
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
/// orientation, so the rotate's own share is the difference. The crop rectangle is read in place,
/// as the fallback does.
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
    let upright = crate::pack_frame(frame, 1, path)?;
    let pack = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let oriented = crate::pack_frame(frame, orientation, path)?;
    let rotated = start.elapsed().as_secs_f64();
    std::hint::black_box(&upright);
    std::hint::black_box(&oriented);
    Ok((pack, (rotated - pack).max(0.0), out_width, out_height))
}
