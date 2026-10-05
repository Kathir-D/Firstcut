//! Per-pair burst-boundary signals and the local frame interval that makes thresholds adaptive.
//!
//! Task.md §5.2 lists the signals; §5.3 gives the algorithm. This module computes them and decides
//! `Join` / `Split` / `Ambiguous` for one consecutive pair. Keeping it pure and separate from the
//! batching pass is what makes the thresholds tunable from real data (`firstcut gaps`).

/// Gaps at or above this are never counted as "inside a burst" when estimating the frame interval,
/// so a long pause between two bursts can't inflate the local rate (todo.md §5.3 step 1).
pub const LOCAL_WINDOW_MAX_MS: i64 = 500;

/// How many gaps either side of a boundary the local rate is estimated from, and how far it widens
/// before falling back to the whole sequence's median.
const LOCAL_RADIUS: usize = 5;
const LOCAL_RADIUS_FALLBACK: usize = 25;

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

/// Adaptive Δt thresholds around a local frame interval `f` (todo.md §5.3 steps 2–3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    pub frame_interval_ms: i64,
    /// Δt ≤ this is always a join.
    pub join_ms: i64,
    /// Δt > this is always a split.
    pub split_ms: i64,
}

/// Why a Δt is not trustworthy, if it isn't.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    #[default]
    Missing,
}

/// Every signal todo.md §5.2 lists, for one consecutive pair.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
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
        // one outcome visual signatures can never revisit (REV-63). Saturating rather than
        // subtracting, because a corrupt timestamp near `i64::MIN` panicked in debug and wrapped
        // to a negative number in release — which then reads as a huge Δt or a backwards one
        // depending on which side the corrupt value was on.
        let dt_ms = match (ta, tb) {
            (Some(a), Some(b)) => b.saturating_sub(a),
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

    /// True when Δt is a real number this pair can be scored on.
    ///
    /// Deliberately wider than [`Self::has_exif_time`]: a fallback (mtime) gap is coarse but it is
    /// still real evidence about how far apart two frames are, and the score is advisory where the
    /// hard decision is not. Narrower than "some time exists": a **backwards** gap would normalise
    /// to `0.0`, which reads as "same instant" and drags the pair toward one burst, and a **missing**
    /// gap carries `dt_ms == 0` for the same reason. Both are excluded so the other signals decide.
    #[must_use]
    pub fn has_scorable_time(&self) -> bool {
        matches!(self.time_quality, TimeQuality::Exif | TimeQuality::Fallback)
    }

    /// Hard joins and hard splits, in todo.md §5.3 order. `Ambiguous` means timing can't decide.
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
/// walks the ISO several stops without changing the moment (todo.md §5.2: "small auto-ISO drift
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
/// to 40 fps instead of being hard-coded to one speed (todo.md §5.3 step 1).
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
    ///
    /// Allocates a sample vector per call, which is why the batching pass does not use it: it
    /// asks for every boundary in one shoot, so it calls [`Self::local_intervals`] once and
    /// indexes the result.
    #[must_use]
    pub fn at(&self, i: usize) -> i64 {
        self.at_with(i, &mut Vec::new())
    }

    /// Every local frame interval for a sequence of `n` photos, computed in one pass.
    ///
    /// Same values as calling [`Self::at`] for each `i`, but with one sample buffer for the whole
    /// shoot instead of one heap allocation and one sort per boundary — `at` is `O(n·w)` with an
    /// allocation inside the loop, and the batching pass runs it once per boundary.
    #[must_use]
    pub fn local_intervals(&self, n: usize) -> Vec<i64> {
        let mut sample = Vec::with_capacity(2 * LOCAL_RADIUS_FALLBACK + 1);
        (0..n).map(|i| self.at_with(i, &mut sample)).collect()
    }

    /// [`Self::at`] with the caller's buffer, so a caller asking for many boundaries allocates
    /// once.
    fn at_with(&self, i: usize, sample: &mut Vec<i64>) -> i64 {
        // No gaps means no rate to estimate: every window is empty, so indexing one would run off
        // the end of the slice (a one-photo folder reaches this).
        if self.gaps.is_empty() {
            return self.default;
        }
        for radius in [LOCAL_RADIUS, LOCAL_RADIUS_FALLBACK] {
            let lo = i.saturating_sub(radius);
            let hi = (i + radius).min(self.gaps.len().saturating_sub(1));
            sample.clear();
            sample.extend(
                self.gaps[lo..=hi]
                    .iter()
                    .copied()
                    .filter(|&g| g > 0 && g < LOCAL_WINDOW_MAX_MS),
            );
            if !sample.is_empty() {
                sample.sort_unstable();
                return median(sample);
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
    use crate::batch::view::mock::MockPhoto as M;

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
    fn an_empty_gap_list_asks_for_no_rate_and_gets_the_default() {
        // A one-photo folder reaches this. `at` used to slice `gaps[0..=0]` on an empty vector.
        let fi = FrameIntervals::new(Vec::new(), 100);
        assert_eq!(fi.at(0), 100);
    }

    #[test]
    fn the_batched_local_intervals_are_the_ones_at_returns() {
        // `local_intervals` exists only to save an allocation per boundary, so every value has to
        // be the one `at` would have computed, including the widened window and the fallbacks.
        let gaps = vec![
            0, 90, 90, 90, 1_000_000, 90, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25,
        ];
        let fi = FrameIntervals::new(gaps.clone(), 90);
        let all = fi.local_intervals(gaps.len());
        assert_eq!(all.len(), gaps.len());
        for (i, &f) in all.iter().enumerate() {
            assert_eq!(f, fi.at(i), "boundary {i}");
        }
        assert!(
            all.contains(&25),
            "the 40 fps stretch must survive: {all:?}"
        );
    }

    #[test]
    fn a_fallback_timestamp_never_makes_a_hard_decision() {
        // REV-63 as a promise: two frames whose times are two seconds apart *because both came from
        // the file system* must not be joined or split on that gap alone. Coarse mtimes (exFAT's
        // 2 s granularity) make exactly this Δt look like a re-press. With real capture times the
        // same pair is a hard split, which is what makes the contrast the assertion.
        let from_mtime = (
            M::frame(0, 0).without_capture_time().with_mtime(0),
            M::frame(1, 0).without_capture_time().with_mtime(2_000),
        );
        let sigs = PairSignals::compute(&from_mtime.0, &from_mtime.1);
        assert_eq!(sigs.time_quality, TimeQuality::Fallback);
        assert_eq!(sigs.dt_ms, 2_000);
        assert!(
            !sigs.has_exif_time(),
            "an mtime is not EXIF, whatever the field it arrives in"
        );
        let t = Thresholds {
            frame_interval_ms: 90,
            join_ms: 250,
            split_ms: 1_500,
        };
        assert_eq!(
            sigs.decide(t),
            Decision::Ambiguous,
            "a fallback Δt goes to the ambiguous zone so signatures decide"
        );

        let from_exif = (M::frame(0, 0), M::frame(1, 0).at(2_000));
        let sigs = PairSignals::compute(&from_exif.0, &from_exif.1);
        assert_eq!(sigs.time_quality, TimeQuality::Exif);
        assert_eq!(sigs.decide(t), Decision::Split);
    }

    #[test]
    fn a_timestamp_at_the_integer_limits_does_not_overflow_the_gap() {
        // A corrupt mtime of `i64::MIN + 1` against a real 2023 instant is a gap no `i64` holds.
        // Plain subtraction panicked in debug and wrapped to a negative number in release, which
        // then read as a backwards pair — a hard split nobody could revisit (REV-63).
        let real = 1_700_000_000_000;
        let corrupt = M::frame(0, 0).at(i64::MIN + 1);

        let forwards = PairSignals::compute(&corrupt, &M::frame(1, 0).at(real));
        assert_eq!(
            forwards.dt_ms,
            i64::MAX,
            "the gap saturates rather than wrapping negative"
        );
        assert!(
            forwards.dt_ms > 0,
            "a huge positive gap must not read as a negative one"
        );
        assert_ne!(
            forwards.time_quality,
            TimeQuality::Backwards,
            "the later frame really is later"
        );

        let backwards = PairSignals::compute(&M::frame(0, 0).at(real), &corrupt);
        assert_eq!(backwards.time_quality, TimeQuality::Backwards);
        assert_eq!(backwards.dt_ms, i64::MIN);
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
