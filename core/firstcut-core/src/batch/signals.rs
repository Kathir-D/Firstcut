//! Per-pair burst-boundary signals and the local frame interval that makes thresholds adaptive.
//!
//! Task.md §5.2 lists the signals; §5.3 gives the algorithm. This module computes them and decides
//! `Join` / `Split` / `Ambiguous` for one consecutive pair. Keeping it pure and separate from the
//! batching pass is what makes the thresholds tunable from real data (`firstcut gaps`).

/// Gaps at or above this are never counted as "inside a burst" when estimating the frame interval,
/// so a long pause between two bursts can't inflate the local rate (task.md §5.3 step 1).
pub const LOCAL_WINDOW_MAX_MS: i64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Same burst, decided without scoring.
    Join,
    /// Different burst, decided without scoring.
    Split,
    /// Inside the zone where timing alone can't decide; needs the score.
    Ambiguous,
}

impl Decision {
    #[must_use]
    pub fn is_split(self) -> bool {
        matches!(self, Decision::Split)
    }
}

/// Adaptive Δt thresholds around a local frame interval `f` (task.md §5.3 steps 2–3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    pub frame_interval_ms: i64,
    /// Δt ≤ this is always a join.
    pub join_ms: i64,
    /// Δt > this is always a split.
    pub split_ms: i64,
}

/// Why a Δt is not trustworthy, if it isn't.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeQuality {
    /// Both frames carry a real EXIF capture time.
    Exif,
    /// At least one side fell back to the filesystem mtime. Coarse or shared mtimes (exFAT's 2 s
    /// granularity, a plain `cp`, a folder of files that never had EXIF) make the resulting Δt
    /// meaningless, so it must never produce a hard decision.
    Fallback,
    /// The later frame's effective time is *before* the earlier one. Only reachable through an
    /// mtime fallback or corrupt data, but a wrong merge is the failure this project exists to
    /// prevent, so it splits rather than clamping to zero (REV-63).
    Backwards,
    /// At least one frame has no usable time at all.
    Missing,
}

/// Every signal task.md §5.2 lists, for one consecutive pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairSignals {
    /// Signed. Negative means the effective times run backwards; see [`TimeQuality::Backwards`].
    pub dt_ms: i64,
    pub time_quality: TimeQuality,
    pub serial_changed: bool,
    pub orientation_changed: bool,
    /// `cur.shutter_count - prev.shutter_count`, when both bodies reported one.
    pub shutter_count_gap: Option<i64>,
    /// `|log2(focal_cur / focal_prev)|`, in stops.
    pub focal_stops: f32,
    /// Aperture + shutter change in stops, plus ISO drift at a third of the weight.
    pub exposure_ev: f32,
}

impl PairSignals {
    #[must_use]
    pub fn compute<P: crate::batch::Photo + ?Sized>(prev: &P, cur: &P) -> Self {
        let (ta, tb) = (
            crate::batch::view::effective_time_ms(prev),
            crate::batch::view::effective_time_ms(cur),
        );
        // Kept signed: clamping a backwards gap to zero turned it into a hard join, which is the
        // one outcome visual signatures can never revisit (REV-63).
        let dt_ms = match (ta, tb) {
            (Some(a), Some(b)) => b - a,
            _ => 0,
        };
        let time_quality = match (ta, tb) {
            (Some(a), Some(b)) if b < a => TimeQuality::Backwards,
            (Some(_), Some(_)) => {
                if crate::batch::view::time_is_fallback(prev)
                    || crate::batch::view::time_is_fallback(cur)
                {
                    TimeQuality::Fallback
                } else {
                    TimeQuality::Exif
                }
            }
            _ => TimeQuality::Missing,
        };
        let shutter_count_gap = match (prev.shutter_count(), cur.shutter_count()) {
            (Some(a), Some(b)) => Some(b as i64 - a as i64),
            _ => None,
        };
        Self {
            dt_ms,
            time_quality,
            serial_changed: prev.camera_serial() != cur.camera_serial(),
            orientation_changed: prev.orientation() != cur.orientation(),
            shutter_count_gap,
            focal_stops: stops_between(prev.focal_length_mm(), cur.focal_length_mm()),
            exposure_ev: exposure_ev_between(prev, cur),
        }
    }

    /// True when Δt comes from real EXIF on both sides, so it can be trusted for a hard decision.
    #[must_use]
    pub fn has_exif_time(&self) -> bool {
        self.time_quality == TimeQuality::Exif
    }

    /// Hard joins and hard splits, in task.md §5.3 order. `Ambiguous` means timing can't decide.
    ///
    /// Two rules here are deliberately conservative, because both would otherwise produce a hard
    /// decision that visual signatures can never revisit:
    ///
    /// - a **backwards** Δt splits (REV-63): the ordering said these frames are one way round and
    ///   the timestamps say the other, which is a data problem, not a burst;
    /// - a **fallback** or **missing** Δt goes to `Ambiguous` rather than falling back on the hard
    ///   join, so the other signals decide and the batch stays `provisional`.
    #[must_use]
    pub fn decide(&self, t: Thresholds) -> Decision {
        if self.serial_changed {
            return Decision::Split;
        }
        if self.orientation_changed {
            return Decision::Split;
        }
        if self.time_quality == TimeQuality::Backwards {
            return Decision::Split;
        }
        if !self.has_exif_time() {
            return Decision::Ambiguous;
        }
        if self.dt_ms > t.split_ms {
            return Decision::Split;
        }
        if self.dt_ms <= t.join_ms {
            return Decision::Join;
        }
        Decision::Ambiguous
    }
}

/// `|log2(b / a)|` over positive values, `0.0` when either side is missing or non-positive.
#[must_use]
pub fn stops_between(a: Option<f32>, b: Option<f32>) -> f32 {
    match (a, b) {
        (Some(a), Some(b)) if a > 0.0 && b > 0.0 => (b / a).log2().abs(),
        _ => 0.0,
    }
}

/// Aperture + shutter change in stops. ISO drift is weighted down because a burst in changing light
/// walks the ISO several stops without changing the moment (task.md §5.2: "small auto-ISO drift
/// ignored").
#[must_use]
pub fn exposure_ev_between<P: crate::batch::Photo + ?Sized>(prev: &P, cur: &P) -> f32 {
    let mut ev = 0.0f32;
    if let (Some(t0), Some(t1), Some(n0), Some(n1)) = (
        prev.exposure_time_s(),
        cur.exposure_time_s(),
        prev.f_number(),
        cur.f_number(),
    ) && t0 > 0.0
        && t1 > 0.0
        && n0 > 0.0
        && n1 > 0.0
    {
        ev += (t1 / t0).log2().abs() + 2.0 * (n1 / n0).log2().abs();
    }
    if let (Some(i0), Some(i1)) = (prev.iso(), cur.iso())
        && i0 > 0
        && i1 > 0
    {
        ev += ISO_DRIFT_WEIGHT * (i1 as f32 / i0 as f32).log2().abs();
    }
    ev
}

const ISO_DRIFT_WEIGHT: f32 = 0.35;

/// Rolling median of Δt among the neighbouring short gaps, so thresholds adapt from 6 fps to 11 fps
/// to 40 fps instead of being hard-coded to one speed (task.md §5.3 step 1).
///
/// `gaps[i]` is the Δt from element `i-1` to element `i` in capture order; `gaps[0]` is unused.
#[derive(Debug, Clone)]
pub struct FrameIntervals {
    gaps: Vec<i64>,
    /// Median over the whole sequence, used when a local window finds nothing.
    global: Option<i64>,
    default: i64,
}

impl FrameIntervals {
    #[must_use]
    pub fn new(gaps: Vec<i64>, default: i64) -> Self {
        let mut all: Vec<i64> = gaps
            .iter()
            .skip(1)
            .copied()
            .filter(|&g| g > 0 && g < LOCAL_WINDOW_MAX_MS)
            .collect();
        let global = if all.is_empty() {
            None
        } else {
            all.sort_unstable();
            Some(median(&all))
        };
        Self {
            gaps,
            global,
            default,
        }
    }

    /// Local frame interval at boundary `i` (between element `i-1` and element `i`).
    ///
    /// Widens the window twice before giving up, so a single frame surrounded by long pauses still
    /// gets a usable local rate instead of a global average from the other end of the game.
    #[must_use]
    pub fn at(&self, i: usize) -> i64 {
        for radius in [5usize, 25] {
            let lo = i.saturating_sub(radius);
            let hi = (i + radius).min(self.gaps.len().saturating_sub(1));
            let mut sample: Vec<i64> = self.gaps[lo..=hi]
                .iter()
                .copied()
                .filter(|&g| g > 0 && g < LOCAL_WINDOW_MAX_MS)
                .collect();
            if !sample.is_empty() {
                sample.sort_unstable();
                return median(&sample);
            }
        }
        self.global.unwrap_or(self.default)
    }
}

/// Median of a sorted slice; the mean of the two middle values when the count is even.
fn median(sorted: &[i64]) -> i64 {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_even_count_averages_the_middle() {
        assert_eq!(median(&[10, 20, 30, 40]), 25);
        assert_eq!(median(&[10, 20, 30]), 20);
        assert_eq!(median(&[]), 0);
    }

    #[test]
    fn local_window_ignores_long_pauses() {
        // 1_000_000 marks a multi-minute pause; it must not raise the local frame interval.
        let gaps = vec![0, 90, 90, 1_000_000, 90, 90];
        let fi = FrameIntervals::new(gaps, 100);
        // Boundary 4 sits next to the pause, but its radius-5 window still sees short gaps.
        assert_eq!(fi.at(4), 90);
    }

    #[test]
    fn local_window_falls_back_to_global_then_default() {
        let fi = FrameIntervals::new(vec![0, 5_000, 6_000, 7_000], 100);
        assert_eq!(fi.at(1), 100, "no short gaps anywhere: use the default");

        let fi = FrameIntervals::new(vec![0, 5_000, 100, 5_000], 100);
        assert_eq!(fi.at(1), 100, "one short gap in the run: global median");
    }

    #[test]
    fn stops_between_handles_missing_and_zero() {
        assert!((stops_between(Some(200.0), Some(200.0))).abs() < 1e-6);
        assert!((stops_between(Some(100.0), Some(200.0)) - 1.0).abs() < 1e-5);
        assert!((stops_between(Some(200.0), Some(100.0)) - 1.0).abs() < 1e-5);
        assert_eq!(stops_between(None, Some(200.0)), 0.0);
        assert_eq!(stops_between(Some(0.0), Some(200.0)), 0.0);
    }
}
