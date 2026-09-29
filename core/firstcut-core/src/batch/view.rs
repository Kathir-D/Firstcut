//! The read-only view of a photo that ordering and batching need.
//!
//! `core-meta` owns the real [`PhotoMeta`](docs/contracts/photo-meta.md) type. It implements this
//! trait with a single `impl` block, so `order()` and `batch()` work against the real struct without
//! either side depending on the other's internals — and against test fixtures and the CLI's
//! exiftool adapter in the meantime.

use crate::batch::PhotoId;

/// Everything the batcher reads off a photo.
///
/// Deliberately small: these are the fields named in task.md §5.2 as burst-boundary signals. A
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
}

/// The timestamp `order()` sorts on: capture time when present, file mtime otherwise.
///
/// `None` only when the file has neither, in which case it sorts after everything that has a time.
#[must_use]
pub fn effective_time_ms<P: Photo + ?Sized>(p: &P) -> Option<i64> {
    p.capture_unix_ms().or_else(|| p.file_mtime_ms())
}

/// True when the timestamp is a fallback and must be flagged in the log (task.md §5.1).
#[must_use]
pub fn time_is_fallback<P: Photo + ?Sized>(p: &P) -> bool {
    p.capture_unix_ms().is_none() && p.file_mtime_ms().is_some()
}

/// A `Photo` with every field settable, for the ordering and batching tests. Shared so the two test
/// modules cannot drift apart on what a photo looks like.
#[cfg(test)]
pub mod mock {
    use super::{Photo, PhotoId};
    use crate::batch::VisualSig;
    use std::collections::HashMap;

    #[derive(Clone)]
    pub struct MockPhoto {
        pub id: u64,
        pub path: String,
        pub capture_ms: Option<i64>,
        pub mtime_ms: Option<i64>,
        pub subsec_resolution_ms: u16,
        pub serial: Option<String>,
        pub shutter_count: Option<u64>,
        pub file_number: Option<u32>,
        pub focal_mm: Option<f32>,
        pub exposure_s: Option<f32>,
        pub aperture: Option<f32>,
        pub iso: Option<u32>,
        pub orientation: u8,
    }

    impl MockPhoto {
        /// One frame at an exact capture time, with every other field a typical Canon R8 value.
        /// Tests that care about Δt use this, so the time in the assertion is the time in the test.
        pub fn frame(id: u64, ms: i64) -> Self {
            Self::in_burst(id, ms, 0)
        }

        /// A frame `n` of a run that started at `start_ms`, `interval_ms` apart.
        pub fn in_burst(n: u64, start_ms: i64, interval_ms: i64) -> Self {
            Self {
                id: n,
                path: format!("IMG_{n:04}.CR3"),
                capture_ms: Some(start_ms + interval_ms * n as i64),
                mtime_ms: None,
                subsec_resolution_ms: 10,
                serial: Some("BODY1".into()),
                shutter_count: Some(1_000 + n),
                file_number: Some(1 + n as u32),
                focal_mm: Some(200.0),
                exposure_s: Some(0.000_5),
                aperture: Some(2.8),
                iso: Some(800),
                orientation: 1,
            }
        }

        pub fn named(mut self, path: &str) -> Self {
            self.path = path.into();
            self
        }

        pub fn at(mut self, ms: i64) -> Self {
            self.capture_ms = Some(ms);
            self
        }

        pub fn without_capture_time(mut self) -> Self {
            self.capture_ms = None;
            self
        }

        pub fn with_mtime(mut self, ms: i64) -> Self {
            self.mtime_ms = Some(ms);
            self
        }

        pub fn serial(mut self, serial: &str) -> Self {
            self.serial = Some(serial.into());
            self
        }

        pub fn shutter(mut self, n: u64) -> Self {
            self.shutter_count = Some(n);
            self
        }

        pub fn orientation(mut self, o: u8) -> Self {
            self.orientation = o;
            self
        }

        pub fn focal(mut self, mm: f32) -> Self {
            self.focal_mm = Some(mm);
            self
        }

        pub fn exposure(mut self, seconds: f32, aperture: f32) -> Self {
            self.exposure_s = Some(seconds);
            self.aperture = Some(aperture);
            self
        }

        pub fn iso(mut self, iso: u32) -> Self {
            self.iso = Some(iso);
            self
        }
    }

    impl Photo for MockPhoto {
        fn id(&self) -> PhotoId {
            PhotoId(self.id)
        }
        fn rel_path(&self) -> &str {
            &self.path
        }
        fn camera_serial(&self) -> Option<&str> {
            self.serial.as_deref()
        }
        fn capture_unix_ms(&self) -> Option<i64> {
            self.capture_ms
        }
        fn subsec_resolution_ms(&self) -> u16 {
            self.subsec_resolution_ms
        }
        fn file_mtime_ms(&self) -> Option<i64> {
            self.mtime_ms
        }
        fn shutter_count(&self) -> Option<u64> {
            self.shutter_count
        }
        fn file_number(&self) -> Option<u32> {
            self.file_number
        }
        fn focal_length_mm(&self) -> Option<f32> {
            self.focal_mm
        }
        fn exposure_time_s(&self) -> Option<f32> {
            self.exposure_s
        }
        fn f_number(&self) -> Option<f32> {
            self.aperture
        }
        fn iso(&self) -> Option<u32> {
            self.iso
        }
        fn orientation(&self) -> u8 {
            self.orientation
        }
    }

    /// Paths of a run of photos, capture-ordered, as a compact string for assertions.
    pub fn ordered_paths(photos: &[MockPhoto]) -> Vec<String> {
        crate::order::ordered_indices(photos)
            .into_iter()
            .map(|i| photos[i].path.clone())
            .collect()
    }

    /// Photo ids of each batch, as `Vec<Vec<u64>>`, which reads better in an assertion than ids.
    pub fn batch_ids(photos: &[MockPhoto]) -> Vec<Vec<u64>> {
        batch_ids_with(photos, &Default::default())
    }

    pub fn batch_ids_with(
        photos: &[MockPhoto],
        params: &crate::batch::BatchParams,
    ) -> Vec<Vec<u64>> {
        crate::batch::batch_with(photos, &std::collections::HashMap::new(), &[], *params)
            .batches
            .iter()
            .map(|b| b.photo_ids.iter().map(|id| id.0).collect())
            .collect()
    }

    /// Batch with signatures attached, the second phase of the two-phase pass.
    pub fn batch_ids_with_sigs(
        photos: &[MockPhoto],
        sigs: &HashMap<PhotoId, VisualSig>,
        params: &crate::batch::BatchParams,
    ) -> Vec<Vec<u64>> {
        crate::batch::batch_with(photos, sigs, &[], *params)
            .batches
            .iter()
            .map(|b| b.photo_ids.iter().map(|id| id.0).collect())
            .collect()
    }
}
