//! The fused exact develop: from the CFA plane to upright RGBA8 in one pipeline.
//!
//! rawler's [`RawDevelop`](rawler::imgop::develop::RawDevelop) does this as six full-frame
//! passes — rescale, demosaic, white balance, calibrate, crop, sRGB — and the demosaic expands the
//! sensor frame into a 288 MB interleaved f32 RGB buffer that four further passes then walk. This
//! module keeps only the three channel planes the demosaic actually produces, applies the crop as
//! a rectangle instead of a copy, folds the white balance and the camera matrix into the pass that
//! reads those planes, and fuses the sRGB encode and the RGBA pack into that same pass.
//!
//! The demosaic is three row-parallel sweeps (green; own colour plus red/blue at the green pixels;
//! the missing channel at red/blue pixels), each of which fetches the five rows it reads once and
//! then walks its half of the row with a stride of two, so the inner loops do no index arithmetic
//! and no per-pixel CFA lookup. The output pass then reads three complete rows and writes four
//! bytes per pixel.
//!
//! # Provenance
//!
//! The demosaic is Patterned Pixel Grouping by Chuan-kai Lin, as implemented by rawler 0.8
//! (`imgop/sensor/bayer/ppg.rs`, LGPL-2.1, Copyright 2022 Daniel Vogelbacher). The interpolation
//! rules, their arithmetic order, and the 3-pixel bilinear border are copied from there, because
//! this backend's pixels have to be rawler's pixels: the plan's §5 numbers and the quality gate are
//! measured against that pipeline. Only the data layout and the fusion are new. rawler is
//! LGPL-2.1-or-later, which this GPL-3.0-only workspace may incorporate.
//!
//! # Refusal, not error
//!
//! [`Plan::new`] returns `None` for anything it has not been proven on — a non-Bayer sensor, a
//! multi-component frame, a Fuji rotation, a matrix it cannot size, a crop that does not fit. The
//! caller then falls back to rawler's own develop, which handles all of those. A refusal is never
//! an error: it only means this fast path declines the file.

use firstcut_decode::{DecodedImage, RgbaBuffer};
use rawler::RawImage;
use rawler::cfa::{CFA, CFA_COLOR_B, CFA_COLOR_G, CFA_COLOR_R};
use rawler::imgop::chromatic_adaption::adapt_bradford;
use rawler::imgop::matrix::{IDENTITY_MATRIX_3, multiply, normalize, pseudo_inverse, transform_1d};
use rawler::imgop::raw::clip_euclidean_norm_avg;
use rawler::imgop::sensor::SensorType;
use rawler::imgop::xyz::{Illuminant, SRGB_TO_XYZ_D65};
use rawler::imgop::{Dim2, Point, Rect};
use rawler::rawimage::RawPhotometricInterpretation;

use crate::{lut_value, srgb_lut};

/// The interpolation border PPG cannot make a directional decision across. rawler interpolates
/// these three pixels bilinearly, so this pass must too.
const BORDER: usize = 3;

/// A plane value at a row-major coordinate, or `0.0` outside it. Every demosaic read is in range
/// by construction; the fallback keeps a malformed layout from panicking. Only the tests index a
/// plane by coordinate; the production passes hoist a whole row instead.
#[cfg(test)]
#[inline]
fn at(values: &[f32], width: usize, row: usize, col: usize) -> f32 {
    row.checked_mul(width).and_then(|base| base.checked_add(col)).and_then(|index| values.get(index)).copied().unwrap_or(0.0)
}

/// Where the sensor data lives, and what pattern it carries.
///
/// All coordinates in the demosaic are **active-area local**: `row`/`col` count from the top left
/// of the region rawler would demosaic, which is what its `roi` parameter means.
pub(crate) struct Grid {
    /// Pixels per row of the *whole* sensor frame.
    pub stride: usize,
    /// The active area, in whole-frame coordinates.
    pub roi: Rect,
    /// The CFA as seen from the active area's origin.
    pub cfa: CFA,
}

/// The three planes the demosaic produces, at active-area size, each complete: every pixel holds
/// all three channels, so the output pass is three plane reads and no CFA work at all.
pub(crate) struct Planes {
    pub green: Vec<f32>,
    pub red: Vec<f32>,
    pub blue: Vec<f32>,
}

/// The five CFA rows one PPG pixel reads: two above, its own, two below. A row outside the frame
/// is an empty slice, which `Grid::value` reads as `0.0`. It is a plain bundle of slices, so it
/// is copied freely rather than borrowed.
#[derive(Clone, Copy)]
struct Window<'a> {
    n2: &'a [f32],
    n1: &'a [f32],
    cur: &'a [f32],
    s1: &'a [f32],
    s2: &'a [f32],
}

/// The three green-plane rows one PPG pixel reads.
#[derive(Clone, Copy)]
struct Green<'a> {
    n1: &'a [f32],
    cur: &'a [f32],
    s1: &'a [f32],
}

impl Grid {
    #[inline]
    pub(crate) fn width(&self) -> usize {
        self.roi.d.w
    }

    #[inline]
    pub(crate) fn height(&self) -> usize {
        self.roi.d.h
    }

    /// One active-area row of the **whole-frame** CFA plane, or `None` when it falls outside the
    /// frame. Each stage hoists the five rows it reads once per row, so the inner loops do no
    /// index arithmetic and one bounds check per read.
    fn sensor_row<'a>(&self, plane: &'a [f32], row: usize) -> Option<&'a [f32]> {
        let y = self.roi.p.y.checked_add(row)?;
        let start = y.checked_mul(self.stride)?.checked_add(self.roi.p.x)?;
        plane.get(start..start.checked_add(self.width())?)
    }

    /// The same row of a plane that is already active-area sized (its stride is the active
    /// width, not the sensor frame's).
    #[inline]
    fn plane_row<'a>(&self, values: &'a [f32], row: usize) -> Option<&'a [f32]> {
        let start = row.checked_mul(self.width())?;
        values.get(start..start.checked_add(self.width())?)
    }

    /// `sensor_row` for a signed offset, with an empty slice for a row outside the frame. Every
    /// stage reads two rows above and two below, so the border rows need this rather than an
    /// early return; the layout itself was checked once before the passes started.
    #[inline]
    fn sensor_row_at<'a>(&self, plane: &'a [f32], row: isize) -> &'a [f32] {
        usize::try_from(row).ok().and_then(|row| self.sensor_row(plane, row)).unwrap_or(&[])
    }

    /// `plane_row` for a signed offset, with an empty slice for a row outside the plane.
    #[inline]
    fn plane_row_at<'a>(&self, values: &'a [f32], row: isize) -> &'a [f32] {
        usize::try_from(row).ok().and_then(|row| self.plane_row(values, row)).unwrap_or(&[])
    }

    /// The colours at even and odd columns of an active-area row. A Bayer pattern repeats every
    /// two pixels, so these two decide the whole row and the inner loops need no pattern lookup.
    fn row_colors(&self, row: usize) -> (usize, usize) {
        (self.cfa.color_at(row, 0), self.cfa.color_at(row, 1))
    }

    /// A row value, or `0.0` outside the row. Every read is in range by construction; the
    /// fallback keeps a malformed layout from panicking.
    #[inline]
    fn value(row: &[f32], index: usize) -> f32 {
        row.get(index).copied().unwrap_or(0.0)
    }

    /// True for the three-pixel ring rawler interpolates bilinearly instead of with PPG.
    #[inline]
    fn is_border(&self, row: usize, col: usize) -> bool {
        row < BORDER || col < BORDER || row.saturating_add(BORDER) >= self.height() || col.saturating_add(BORDER) >= self.width()
    }

    /// The five CFA rows around an active-area row, fetched once per row.
    #[inline]
    fn window<'a>(&self, plane: &'a [f32], row: usize) -> Window<'a> {
        let offset = isize::try_from(row).unwrap_or(isize::MAX);
        Window {
            n2: self.sensor_row_at(plane, offset.saturating_sub(2)),
            n1: self.sensor_row_at(plane, offset.saturating_sub(1)),
            cur: self.sensor_row_at(plane, offset),
            s1: self.sensor_row_at(plane, offset.saturating_add(1)),
            s2: self.sensor_row_at(plane, offset.saturating_add(2)),
        }
    }

    /// The three green-plane rows around an active-area row, fetched once per row.
    #[inline]
    fn green_rows<'a>(&self, green: &'a [f32], row: usize) -> Green<'a> {
        let offset = isize::try_from(row).unwrap_or(isize::MAX);
        Green {
            n1: self.plane_row_at(green, offset.saturating_sub(1)),
            cur: self.plane_row_at(green, offset),
            s1: self.plane_row_at(green, offset.saturating_add(1)),
        }
    }

    /// rawler's `interpolate_borders`: the mean of every raw sample of `channel` in the 3×3 window,
    /// accumulated in the same order, or `0.0` when the window holds none (which is what rawler
    /// leaves in the expanded buffer). Those samples are the pixel's own channel value, so no
    /// plane is needed.
    fn border_average(&self, plane: &[f32], row: usize, col: usize, channel: usize) -> f32 {
        let mut sum = 0.0f32;
        let mut count = 0usize;
        for y in row.saturating_sub(1)..=row.saturating_add(1) {
            if y >= self.height() {
                continue;
            }
            let Some(values) = self.sensor_row(plane, y) else { continue };
            for x in col.saturating_sub(1)..=col.saturating_add(1) {
                if x >= self.width() {
                    continue;
                }
                if self.cfa.color_at(y, x) == channel {
                    sum += Self::value(values, x);
                    count = count.saturating_add(1);
                }
            }
        }
        if count == 0 { 0.0 } else { sum / count as f32 }
    }

    /// rawler's `interpolate_green` for one red/blue pixel in the interior, from the five rows it
    /// reads. Arithmetic order preserved.
    fn green_interior(window: Window<'_>, col: usize) -> f32 {
        let Window { n2, n1, cur, s1, s2 } = window;
        let x = Self::value(cur, col);
        let n_1 = Self::value(n1, col);
        let n_2 = Self::value(n2, col);
        let e_1 = Self::value(cur, col.saturating_add(1));
        let e_2 = Self::value(cur, col.saturating_add(2));
        let s_1 = Self::value(s1, col);
        let s_2 = Self::value(s2, col);
        let w_1 = Self::value(cur, col.saturating_sub(1));
        let w_2 = Self::value(cur, col.saturating_sub(2));

        let n = (x - n_2).abs() * 2.0 + (n_1 - s_1).abs();
        let e = (x - e_2).abs() * 2.0 + (w_1 - e_1).abs();
        let w = (x - w_2).abs() * 2.0 + (w_1 - e_1).abs();
        let s = (x - s_2).abs() * 2.0 + (n_1 - s_1).abs();

        let mut min = n;
        if e < min {
            min = e;
        }
        if w < min {
            min = w;
        }
        if s < min {
            min = s;
        }

        if min == n {
            (n_1 * 3.0 + s_1 + x - n_2) / 4.0
        } else if min == e {
            (e_1 * 3.0 + w_1 + x - e_2) / 4.0
        } else if min == w {
            (w_1 * 3.0 + e_1 + x - w_2) / 4.0
        } else {
            (s_1 * 3.0 + n_1 + x - s_2) / 4.0
        }
    }

    /// rawler's `interpolate_rb_at_green`: hue transit horizontally for the row's channel and
    /// vertically for the column's. Returns `(red, blue)`. `gn`/`g`/`gs` are the green plane's
    /// rows above, at and below; `n1`/`cur`/`s1` the CFA plane's.
    fn rb_at_green(&self, green: Green<'_>, cfa: Window<'_>, row: usize, col: usize) -> (f32, f32) {
        let Green { n1: gn, cur: g, s1: gs } = green;
        let Window { n1, cur, s1, .. } = cfa;
        let g_x = Self::value(g, col);
        let g_w = Self::value(g, col.saturating_sub(1));
        let g_e = Self::value(g, col.saturating_add(1));
        let g_n = Self::value(gn, col);
        let g_s = Self::value(gs, col);
        let h_w = Self::value(cur, col.saturating_sub(1));
        let h_e = Self::value(cur, col.saturating_add(1));
        let v_n = Self::value(n1, col);
        let v_s = Self::value(s1, col);
        let horizontal = hue_transit(g_w, g_x, g_e, h_w, h_e);
        let vertical = hue_transit(g_n, g_x, g_s, v_n, v_s);
        // The horizontal neighbours carry the row's colour, the vertical ones the column's.
        if self.cfa.color_at(row, col.saturating_add(1)) == CFA_COLOR_R { (horizontal, vertical) } else { (vertical, horizontal) }
    }

    /// rawler's `interpolate_rb_at_non_green`: pick the diagonal pair the gradients prefer and
    /// transit the hue along it. Returns the missing channel's value.
    ///
    /// A diagonal neighbour of a red pixel is a blue pixel and vice versa, so every `y` and `x`
    /// sample below is that neighbour's own raw sample — only green is computed, which is why this
    /// pass reads the CFA plane and the green plane and nothing else.
    fn rb_at_non_green(cfa: Window<'_>, green: Green<'_>, col: usize) -> f32 {
        let Window { n2, n1, cur, s1, s2 } = cfa;
        let Green { n1: gn, cur: g, s1: gs } = green;
        let plus_1 = |index: usize| index.saturating_add(1);
        let minus_1 = |index: usize| index.saturating_sub(1);
        let plus_2 = |index: usize| index.saturating_add(2);
        let minus_2 = |index: usize| index.saturating_sub(2);

        let x_center = Self::value(cur, col);
        let y_ne_1 = Self::value(n1, plus_1(col));
        let y_sw_1 = Self::value(s1, minus_1(col));
        let x_ne_2 = Self::value(n2, plus_2(col));
        let x_sw_2 = Self::value(s2, minus_2(col));
        let g_ne_1 = Self::value(gn, plus_1(col));
        let g_center = Self::value(g, col);
        let g_sw_1 = Self::value(gs, minus_1(col));
        let y_nw_1 = Self::value(n1, minus_1(col));
        let y_se_1 = Self::value(s1, plus_1(col));
        let x_nw_2 = Self::value(n2, minus_2(col));
        let x_se_2 = Self::value(s2, plus_2(col));
        let g_nw_1 = Self::value(gn, minus_1(col));
        let g_se_1 = Self::value(gs, plus_1(col));

        let ne =
            (y_ne_1 - y_sw_1).abs() + (x_ne_2 - x_center).abs() + (x_center - x_sw_2).abs() + (g_ne_1 - g_center).abs() + (g_center - g_sw_1).abs();
        let nw =
            (y_nw_1 - y_se_1).abs() + (x_nw_2 - x_center).abs() + (x_center - x_se_2).abs() + (g_nw_1 - g_center).abs() + (g_center - g_se_1).abs();

        if ne < nw { hue_transit(g_ne_1, g_center, g_sw_1, y_ne_1, y_sw_1) } else { hue_transit(g_nw_1, g_center, g_se_1, y_nw_1, y_se_1) }
    }

    /// The demosaiced RGB at one active-area-local pixel. The production output pass reads whole
    /// rows, so this coordinate form exists for the tests that compare against rawler.
    #[cfg(test)]
    pub(crate) fn rgb(&self, planes: &Planes, row: usize, col: usize) -> [f32; 3] {
        let width = self.width();
        [at(&planes.red, width, row, col), at(&planes.green, width, row, col), at(&planes.blue, width, row, col)]
    }

    /// The demosaic: three row-parallel passes that fill all three planes.
    ///
    /// Each pass reads only the CFA plane and the planes the pass before it finished, so the whole
    /// demosaic is three sweeps with no cross-row write dependency and no interleaved
    /// three-channel buffer. Half of every Bayer row is green and half is red or blue, so each
    /// pass walks its own half with a stride of two instead of branching per pixel.
    ///
    /// Returns `None` when the plane cannot cover the active area, the one layout check that
    /// cannot be made from the geometry alone.
    pub(crate) fn demosaic(&self, plane: &[f32]) -> Option<Planes> {
        use rayon::prelude::*;
        let (width, height) = (self.width(), self.height());
        if width == 0 || height == 0 {
            return None;
        }
        // The last element the demosaicker can touch: the end of the active area's final row.
        let last_row = self.roi.p.y.checked_add(height)?.checked_sub(1)?;
        let last = last_row.checked_mul(self.stride)?.checked_add(self.roi.p.x)?.checked_add(width)?;
        if last > plane.len() {
            return None;
        }
        let count = width.checked_mul(height)?;
        let mut planes = Planes { green: vec![0.0; count], red: vec![0.0; count], blue: vec![0.0; count] };

        // Stage 1, green. A green pixel is its own sample; the rest is PPG in the interior and
        // rawler's bilinear ring at the edges. Every row is fetched once, so the inner loop is a
        // handful of slice reads with no index arithmetic.
        planes.green.par_chunks_mut(width).enumerate().for_each(|(row, out)| {
            let window = self.window(plane, row);
            let green_parity = usize::from(self.row_colors(row).0 != CFA_COLOR_G);
            for col in (green_parity..width).step_by(2) {
                if let Some(slot) = out.get_mut(col) {
                    *slot = Self::value(window.cur, col);
                }
            }
            for col in ((1 - green_parity)..width).step_by(2) {
                let value =
                    if self.is_border(row, col) { self.border_average(plane, row, col, CFA_COLOR_G) } else { Self::green_interior(window, col) };
                if let Some(slot) = out.get_mut(col) {
                    *slot = value;
                }
            }
        });

        // Stage 2, the pixel's own colour, plus red and blue at the green pixels. It reads the
        // finished green plane around each green pixel, so it is row-parallel against an immutable
        // neighbour plane and writes two disjoint planes.
        {
            let Planes { green, red, blue } = &mut planes;
            let green = green.as_slice();
            red.par_chunks_mut(width).zip(blue.par_chunks_mut(width)).enumerate().for_each(|(row, (red_row, blue_row))| {
                let green_rows = self.green_rows(green, row);
                let cfa = self.window(plane, row);
                let (even, odd) = self.row_colors(row);
                let green_parity = usize::from(even != CFA_COLOR_G);
                // Half of a Bayer row is green; the other half is one colour, which is the one
                // that keeps its own sample here and the one that is missing at the green pixels.
                let own = if green_parity == 0 { odd } else { even };
                for col in (green_parity..width).step_by(2) {
                    let (r, b) = if self.is_border(row, col) {
                        (self.border_average(plane, row, col, CFA_COLOR_R), self.border_average(plane, row, col, CFA_COLOR_B))
                    } else {
                        self.rb_at_green(green_rows, cfa, row, col)
                    };
                    if let Some(slot) = red_row.get_mut(col) {
                        *slot = r;
                    }
                    if let Some(slot) = blue_row.get_mut(col) {
                        *slot = b;
                    }
                }
                for col in ((1 - green_parity)..width).step_by(2) {
                    let value = Self::value(cfa.cur, col);
                    let slot = if own == CFA_COLOR_R { red_row.get_mut(col) } else { blue_row.get_mut(col) };
                    if let Some(slot) = slot {
                        *slot = value;
                    }
                }
            });
        }

        // Stage 3, the channel that is missing at every red/blue pixel. Its diagonal neighbours are
        // the *other* colour's own samples, so this pass needs only the CFA plane and the finished
        // green plane — it never reads a plane it writes.
        {
            let Planes { green, red, blue } = &mut planes;
            let green = green.as_slice();
            red.par_chunks_mut(width).zip(blue.par_chunks_mut(width)).enumerate().for_each(|(row, (red_row, blue_row))| {
                let cfa = self.window(plane, row);
                let green_rows = self.green_rows(green, row);
                let (even, odd) = self.row_colors(row);
                let green_parity = usize::from(even != CFA_COLOR_G);
                let own = if green_parity == 0 { odd } else { even };
                let missing = if own == CFA_COLOR_R { CFA_COLOR_B } else { CFA_COLOR_R };
                for col in ((1 - green_parity)..width).step_by(2) {
                    let value = if self.is_border(row, col) {
                        self.border_average(plane, row, col, missing)
                    } else {
                        Self::rb_at_non_green(cfa, green_rows, col)
                    };
                    let slot = if missing == CFA_COLOR_B { blue_row.get_mut(col) } else { red_row.get_mut(col) };
                    if let Some(slot) = slot {
                        *slot = value;
                    }
                }
            });
        }
        Some(planes)
    }
}

/// PPG's hue transit (Chuan-kai Lin), arithmetic order preserved from rawler.
#[inline(always)]
fn hue_transit(l1: f32, l2: f32, l3: f32, v1: f32, v3: f32) -> f32 {
    if (l1 < l2 && l2 < l3) || (l1 > l2 && l2 > l3) { v1 + (v3 - v1) * (l2 - l1) / (l3 - l1) } else { (v1 + v3) / 2.0 + (l2 * 2.0 - l1 - l3) / 4.0 }
}

/// The whole exact develop for one file: demosaic, white balance, camera matrix, sRGB, pack, and
/// the display orientation.
pub(crate) struct Plan {
    grid: Grid,
    /// The camera's output rectangle, active-area local.
    crop: Rect,
    wb: [f32; 4],
    /// Camera RGB → linear sRGB, the matrix rawler's `Calibrate` builds.
    cam2rgb: [[f32; 4]; 3],
    orientation: u8,
}

impl Plan {
    /// Resolves everything the fused path needs from the file's metadata, or `None` to decline.
    pub(crate) fn new(raw: &RawImage, orientation: u8) -> Option<Self> {
        if raw.cpp != 1 {
            return None;
        }
        let RawPhotometricInterpretation::Cfa(config) = &raw.photometric else {
            return None;
        };
        if config.sensor != SensorType::Bayer || !config.cfa.is_rgb() || raw.fuji_rotation_width.is_some() {
            return None;
        }
        let frame = Rect::new(Point::zero(), Dim2::new(raw.width, raw.height));
        let roi = raw.active_area.unwrap_or(frame);
        if roi.d.w == 0 || roi.d.h == 0 {
            return None;
        }
        if roi.p.x.checked_add(roi.d.w)? > raw.width || roi.p.y.checked_add(roi.d.h)? > raw.height {
            return None;
        }
        // rawler's `CropDefault` rectangle: the camera's crop inside the active area, which the
        // demosaic already restricted itself to. `adapt` asserts; this returns `None` instead.
        let crop = match (raw.crop_area.or(raw.active_area), raw.active_area) {
            (Some(crop), Some(active)) => {
                if crop.p.x < active.p.x || crop.p.y < active.p.y || crop.d.w > active.d.w || crop.d.h > active.d.h {
                    return None;
                }
                Rect::new(Point::new(crop.p.x - active.p.x, crop.p.y - active.p.y), crop.d)
            }
            (Some(crop), None) => crop,
            (None, _) => Rect::new(Point::zero(), Dim2::new(roi.d.w, roi.d.h)),
        };
        if crop.d.w == 0 || crop.d.h == 0 {
            return None;
        }
        if crop.p.x.checked_add(crop.d.w)? > roi.d.w || crop.p.y.checked_add(crop.d.h)? > roi.d.h {
            return None;
        }

        // rawler's `Calibrate` matrix resolution, in the same illuminant preference order.
        let mut xyz2cam = [[0.0f32; 3]; 4];
        let (illuminant, matrix) = raw
            .color_matrix_find_first([
                Illuminant::D65,
                Illuminant::A,
                Illuminant::B,
                Illuminant::C,
                Illuminant::D50,
                Illuminant::D55,
                Illuminant::D75,
                Illuminant::Daylight,
                Illuminant::Flash,
            ])
            .unwrap_or((Illuminant::D65, IDENTITY_MATRIX_3.as_flattened().to_vec()));
        let d65 = if illuminant == Illuminant::D65 {
            matrix
        } else {
            if matrix.len() != 9 {
                return None;
            }
            let adapted: [[f32; 3]; 3] = transform_1d::<3, 3>(&matrix)?;
            adapt_bradford(&illuminant, &Illuminant::D65, &adapted).as_flattened().to_vec()
        };
        if d65.len() % 3 != 0 {
            return None;
        }
        let components = d65.len() / 3;
        if components == 0 || components > 4 {
            return None;
        }
        for i in 0..components {
            for j in 0..3 {
                xyz2cam.get_mut(i).and_then(|row| row.get_mut(j)).map(|slot| *slot = d65[i * 3 + j])?;
            }
        }
        let rgb2cam = normalize(multiply(&xyz2cam, &SRGB_TO_XYZ_D65));
        let cam2rgb = pseudo_inverse(rgb2cam);

        // rawler assumes unity white balance when the file carries none.
        let wb = if raw.wb_coeffs.first().copied().unwrap_or(1.0).is_nan() { [1.0; 4] } else { raw.wb_coeffs };

        Some(Self { grid: Grid { stride: raw.width, roi, cfa: config.cfa.shift(roi.p.x, roi.p.y) }, crop, wb, cam2rgb, orientation })
    }

    /// White balance, the camera matrix, and rawler's clipping — `map_3ch_to_rgb`, per pixel.
    fn srgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        let r = rgb[0] * self.wb[0];
        let g = rgb[1] * self.wb[1];
        let b = rgb[2] * self.wb[2];
        let m = &self.cam2rgb;
        clip_euclidean_norm_avg(&[
            m[0][0] * r + m[0][1] * g + m[0][2] * b,
            m[1][0] * r + m[1][1] * g + m[1][2] * b,
            m[2][0] * r + m[2][1] * g + m[2][2] * b,
        ])
    }

    /// The display orientation this plan rotates to.
    pub(crate) fn orientation(&self) -> u8 {
        self.orientation
    }

    /// The demosaic alone: three planes at active-area size, still in sensor orientation.
    ///
    /// Split out of [`Plan::develop`] so the bench can time the demosaic and the output pass
    /// separately; production calls `develop`.
    pub(crate) fn demosaic(&self, plane: &[f32]) -> Option<Planes> {
        self.grid.demosaic(plane)
    }

    /// Colour, sRGB encode and pack into RGBA8 over the crop rectangle, still in sensor
    /// orientation. `None` declines the frame so the caller can fall back.
    ///
    /// The three planes are already complete, so this is a straight read of three rows, the
    /// camera matrix, the sRGB table and one 4-byte store: no CFA pattern, no branching.
    pub(crate) fn pack(&self, planes: &Planes) -> Option<DecodedImage> {
        use rayon::prelude::*;
        let width = self.crop.d.w;
        let height = self.crop.d.h;
        let plane_width = self.grid.width();
        let capacity = width.checked_mul(height)?.checked_mul(4)?;
        let row_bytes = width.checked_mul(4)?;
        // The last plane element the crop can reach. `demosaic` sized every plane to the active
        // area and `Plan::new` checked the crop inside it, so this cannot trip on a real frame —
        // it is what stops a malformed geometry from reading zeros.
        let last_row = self.crop.p.y.checked_add(height)?.checked_sub(1)?;
        let last = last_row.checked_mul(plane_width)?.checked_add(self.crop.p.x)?.checked_add(width)?;
        if last > planes.green.len() || last > planes.red.len() || last > planes.blue.len() {
            return None;
        }
        let mut rgba = RgbaBuffer::zeroed(capacity);
        let lut = srgb_lut();
        rgba.as_mut_slice().par_chunks_exact_mut(row_bytes).enumerate().for_each(|(y, out)| {
            let row = self.crop.p.y.saturating_add(y);
            let start = row.saturating_mul(plane_width).saturating_add(self.crop.p.x);
            let end = start.saturating_add(width);
            let (Some(red), Some(green), Some(blue)) = (planes.red.get(start..end), planes.green.get(start..end), planes.blue.get(start..end)) else {
                return;
            };
            for (((pixel, &r), &g), &b) in out.as_chunks_mut::<4>().0.iter_mut().zip(red).zip(green).zip(blue) {
                let srgb = self.srgb([r, g, b]);
                pixel.copy_from_slice(&[lut_value(lut, srgb[0]), lut_value(lut, srgb[1]), lut_value(lut, srgb[2]), 255]);
            }
        });
        let image = DecodedImage { width: u32::try_from(width).ok()?, height: u32::try_from(height).ok()?, rgba };
        Some(image)
    }

    /// Develops an already-rescaled CFA plane (black/white corrected, `f32`) into display-ready
    /// upright RGBA8: demosaic, then the output pass, then the one rotate. `None` declines the
    /// frame so the caller can fall back.
    pub(crate) fn develop(&self, plane: &[f32]) -> Option<DecodedImage> {
        let planes = self.demosaic(plane)?;
        let image = self.pack(&planes)?;
        firstcut_decode::apply_orientation(image, self.orientation).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rawler::cfa::PlaneColor;
    use rawler::imgop::sensor::Demosaic;
    use rawler::imgop::sensor::bayer::ppg::PPGDemosaic;
    use rawler::pixarray::PixF32;

    /// A deterministic frame that is neither symmetric nor flat, so a border or a diagonal being
    /// read from the wrong side changes the pixels.
    fn synthetic(width: usize, height: usize) -> Vec<f32> {
        (0..width * height)
            .map(|index| {
                let x = index % width;
                let y = index / width;
                let checker = ((x * 7 + y * 13) % 31) as f32 / 31.0;
                checker + (x as f32) * 0.013 - (y as f32) * 0.021
            })
            .collect()
    }

    /// The fused demosaic must be rawler's PPG, pixel for pixel, for every Bayer order and for a
    /// region of interest that does not start at the frame origin.
    #[test]
    fn the_fused_demosaic_is_rawlers_ppg() {
        for pattern in ["RGGB", "BGGR", "GRBG", "GBRG"] {
            let (width, height) = (41, 33);
            let data = synthetic(width, height);
            let cfa = CFA::new(pattern);
            let colors = PlaneColor::new(pattern);
            let roi = Rect::new(Point::zero(), Dim2::new(width, height));
            let reference = PPGDemosaic::new().demosaic(&PixF32::new_with(data.clone(), width, height), &cfa, &colors, roi);
            let grid = Grid { stride: width, roi, cfa: cfa.clone() };
            let planes = grid.demosaic(&data).expect("a synthetic frame demosaics");
            for row in 0..height {
                for col in 0..width {
                    let ours = grid.rgb(&planes, row, col);
                    let expected = reference.data[row * width + col];
                    for (channel, (a, b)) in ours.iter().zip(expected.iter()).enumerate() {
                        assert!((a - b).abs() <= 1e-6, "{pattern} at ({row},{col}) channel {channel}: fused {a} vs rawler {b}");
                    }
                }
            }
        }
    }

    /// The same equivalence when the active area starts inside the frame: the CFA has to shift
    /// with it, and the plane stride is still the whole frame's.
    #[test]
    fn the_fused_demosaic_follows_an_offset_active_area() {
        let (width, height) = (44, 36);
        let data = synthetic(width, height);
        let roi = Rect::new(Point::new(2, 2), Dim2::new(38, 30));
        let cfa = CFA::new("GBRG");
        let reference = PPGDemosaic::new().demosaic(&PixF32::new_with(data.clone(), width, height), &cfa, &PlaneColor::new("GBRG"), roi);
        let grid = Grid { stride: width, roi, cfa: cfa.shift(roi.p.x, roi.p.y) };
        let planes = grid.demosaic(&data).expect("an offset frame demosaics");
        for row in 0..roi.d.h {
            for col in 0..roi.d.w {
                let ours = grid.rgb(&planes, row, col);
                let expected = reference.data[row * roi.d.w + col];
                for (channel, (a, b)) in ours.iter().zip(expected.iter()).enumerate() {
                    assert!((a - b).abs() <= 1e-6, "at ({row},{col}) channel {channel}: fused {a} vs rawler {b}");
                }
            }
        }
    }

    /// A plane that cannot cover the active area is a refusal, not a panic and not a guess.
    #[test]
    fn a_plane_that_is_too_small_is_refused() {
        let grid = Grid { stride: 20, roi: Rect::new(Point::zero(), Dim2::new(20, 10)), cfa: CFA::new("RGGB") };
        assert!(grid.demosaic(&vec![0.0f32; 200]).is_some(), "the exact frame is enough");
        assert!(grid.demosaic(&vec![0.0f32; 199]).is_none(), "one pixel short is refused");
    }
}
