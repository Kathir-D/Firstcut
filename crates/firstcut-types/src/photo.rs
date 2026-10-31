//! The read-only view of a photo that ordering and batching need.
//!
//! `firstcut-meta` owns the real [`PhotoMeta`](docs/contracts/photo-meta.md) type (in v1 the
//! batcher read it through this trait with a single `impl` block), so `order()` and `batch()`
//! work against the real struct without either side depending on the other's internals — and
//! against test fixtures and the CLI's exiftool adapter in the meantime. Moved verbatim from
//! v1 `batch::view` (minus its test-only fixture, which stayed with the batcher) at the M1
//! port; this is the seam that breaks the order↔batch↔meta module cycle.

use crate::ids::PhotoId;

/// Everything the batcher reads off a photo.
///
/// Deliberately small: these are the fields named in todo.md §5.2 as burst-boundary signals. A
/// method that allocates would be wasteful in the inner loop, so `&str` is used instead of `String`.
pub trait Photo {
    fn id(&self) -> PhotoId;
    fn rel_path(&self) -> &str;

    fn camera_serial(&self) -> Option<&str>;

    /// Capture time in Unix milliseconds, sub-seconds already folded in, offset already applied.
    fn capture_unix_ms(&self) -> Option<i64>;
    /// Resolution the sub-second field is recorded at: 10 for Canon R8, 1000 when absent.
    fn subsec_resolution_ms(&self) -> u16;
    /// Filesystem modification time, the fallback when there is no usable capture time (§5.1).
    fn file_mtime_ms(&self) -> Option<i64>;

    fn shutter_count(&self) -> Option<u64>;
    fn file_number(&self) -> Option<u32>;

    fn focal_length_mm(&self) -> Option<f32>;
    fn exposure_time_s(&self) -> Option<f32>;
    fn f_number(&self) -> Option<f32>;
    fn iso(&self) -> Option<u32>;
    /// EXIF orientation, 1..=8.
    fn orientation(&self) -> u8;

    /// True when the effective time is a guess the file system made, not a time the camera
    /// recorded (todo.md §5.1), and so must never produce a hard decision (REV-63).
    ///
    /// The default covers the shape where the fallback lives in its own field: no capture time,
    /// but an mtime. An implementation that folds the mtime *into* the capture-time field — as
    /// `PhotoMeta` does, keeping `source: FileModified` on it — has to override this,
    /// because there `capture_unix_ms()` is `Some` and the default would hand the batcher an
    /// mtime wearing EXIF's name.
    fn time_source_is_fallback(&self) -> bool {
        self.capture_unix_ms().is_none() && self.file_mtime_ms().is_some()
    }
}

impl<T: Photo + ?Sized> Photo for &T {
    fn id(&self) -> PhotoId {
        (**self).id()
    }
    fn rel_path(&self) -> &str {
        (**self).rel_path()
    }
    fn camera_serial(&self) -> Option<&str> {
        (**self).camera_serial()
    }
    fn capture_unix_ms(&self) -> Option<i64> {
        (**self).capture_unix_ms()
    }
    fn subsec_resolution_ms(&self) -> u16 {
        (**self).subsec_resolution_ms()
    }
    fn file_mtime_ms(&self) -> Option<i64> {
        (**self).file_mtime_ms()
    }
    fn shutter_count(&self) -> Option<u64> {
        (**self).shutter_count()
    }
    fn file_number(&self) -> Option<u32> {
        (**self).file_number()
    }
    fn focal_length_mm(&self) -> Option<f32> {
        (**self).focal_length_mm()
    }
    fn exposure_time_s(&self) -> Option<f32> {
        (**self).exposure_time_s()
    }
    fn f_number(&self) -> Option<f32> {
        (**self).f_number()
    }
    fn iso(&self) -> Option<u32> {
        (**self).iso()
    }
    fn orientation(&self) -> u8 {
        (**self).orientation()
    }
    fn time_source_is_fallback(&self) -> bool {
        (**self).time_source_is_fallback()
    }
}

/// The timestamp `order()` sorts on: capture time when present, file mtime otherwise.
///
/// `None` only when the file has neither, in which case it sorts after everything that has a time.
#[must_use]
pub fn effective_time_ms<P: Photo + ?Sized>(p: &P) -> Option<i64> {
    p.capture_unix_ms().or_else(|| p.file_mtime_ms())
}

/// True when the timestamp is a fallback and must be flagged in the log (todo.md §5.1).
///
/// Routed through [`Photo::time_source_is_fallback`] rather than worked out here: `meta`'s
/// `PhotoMeta` reports no `file_mtime_ms` at all and keeps the mtime inside `capture_time` with
/// `source: FileModified`, so the two-field rule below is false for exactly the photos that are
/// guesses, and every mtime-derived gap would have been scored as EXIF-quality.
#[must_use]
pub fn time_is_fallback<P: Photo + ?Sized>(p: &P) -> bool {
    p.time_source_is_fallback()
}
