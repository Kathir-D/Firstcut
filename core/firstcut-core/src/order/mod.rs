//! Capture order.
//!
//! Task.md §5.1. Ordering is the foundation of batching and of every downstream assumption
//! (`Batch.photo_ids` is "in capture order"), so the rule is deliberately conservative: **capture
//! time is the primary key and nothing else is.** File names never participate except as the final
//! tie-break between photos that are otherwise indistinguishable, which is what makes
//! `IMG_9999 → IMG_0001` rollover and renamed files irrelevant.

use std::cmp::Ordering;

use crate::batch::PhotoId;
use crate::batch::view::{Photo, effective_time_ms, time_is_fallback};

/// What `order()` had to do that is worth telling the user about (todo.md §5.1: files with no
/// usable timestamp must be flagged in the log).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderReport {
    /// Photos whose timestamp came from the file system because EXIF had none.
    pub fallback_times: Vec<String>,
    /// Photos with neither an EXIF time nor an mtime. They sort last, deterministically by path.
    pub no_time: Vec<String>,
}

/// Capture order. Never uses file names except as the final tie-breaker.
#[must_use]
pub fn order<P: Photo>(photos: &[P]) -> Vec<PhotoId> {
    order_with_report(photos).0
}

/// `order()`, plus the log entries the app shows the user.
#[must_use]
pub fn order_with_report<P: Photo>(photos: &[P]) -> (Vec<PhotoId>, OrderReport) {
    let indices = ordered_indices(photos);
    let mut report = OrderReport::default();
    for &i in &indices {
        let p = &photos[i];
        if time_is_fallback(p) {
            report.fallback_times.push(p.rel_path().to_string());
        } else if p.capture_unix_ms().is_none() && p.file_mtime_ms().is_none() {
            report.no_time.push(p.rel_path().to_string());
        }
    }
    (indices.iter().map(|&i| photos[i].id()).collect(), report)
}

/// Indices into `photos`, in capture order. `batch()` walks the same order.
#[must_use]
pub fn ordered_indices<P: Photo>(photos: &[P]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..photos.len()).collect();
    indices.sort_by(|&a, &b| compare(photos, a, b));
    indices
}

/// The sort key, one comparison at a time. Split out so the tests can assert each level on its own.
fn compare<P: Photo>(photos: &[P], a: usize, b: usize) -> Ordering {
    let (pa, pb) = (&photos[a], &photos[b]);
    cmp_optional_time(pa, pb)
        // Within the same millisecond the shutter count is the real sequence (todo.md §5.1).
        .then_with(|| cmp_optional_num(pa.shutter_count(), pb.shutter_count()))
        .then_with(|| cmp_optional_num(pa.file_number(), pb.file_number()))
        // Bodies in one folder interleave by time; the serial only orders photos that are
        // otherwise identical, and `batch()` keeps different serials apart regardless.
        .then_with(|| cmp_optional_str(pa.camera_serial(), pb.camera_serial()))
        .then_with(|| pa.rel_path().cmp(pb.rel_path()))
}

/// Photos with no usable time at all sort after every timed photo.
fn cmp_optional_time<P: Photo>(pa: &P, pb: &P) -> Ordering {
    match (effective_time_ms(pa), effective_time_ms(pb)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// A known value sorts before an unknown one, so a missing shutter count can never displace a
/// frame whose position we actually know.
fn cmp_optional_num<T: Ord>(a: Option<T>, b: Option<T>) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn cmp_optional_str(a: Option<&str>, b: Option<&str>) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::view::mock::{MockPhoto, ordered_paths};

    /// A frame with only the fields this test cares about; `MockPhoto` fills in the rest with
    /// realistic Canon R8 values that are constant, so they never affect ordering.
    fn frame(id: u64, ms: i64) -> MockPhoto {
        MockPhoto::in_burst(id, ms, 0)
    }

    #[test]
    fn sorts_by_capture_time_not_file_name() {
        let photos = vec![
            frame(1, 9_000).named("IMG_0009.CR3"),
            frame(2, 1_000).named("IMG_0001.CR3"),
            frame(3, 5_000).named("IMG_0005.CR3"),
        ];
        assert_eq!(
            ordered_paths(&photos),
            ["IMG_0001.CR3", "IMG_0005.CR3", "IMG_0009.CR3"]
        );
    }

    #[test]
    fn rolls_over_from_img_9999_to_img_0001() {
        // The shoot crosses the 9999 rollover: file names go backwards, times go forwards. Nothing
        // in ordering may notice.
        let photos = vec![
            frame(1, 4_000).named("IMG_9999.CR3"),
            frame(2, 5_000).named("IMG_0001.CR3"),
            frame(3, 6_000).named("IMG_0002.CR3"),
            frame(4, 3_000).named("IMG_9998.CR3"),
        ];
        assert_eq!(
            ordered_paths(&photos),
            [
                "IMG_9998.CR3",
                "IMG_9999.CR3",
                "IMG_0001.CR3",
                "IMG_0002.CR3"
            ]
        );
    }

    #[test]
    fn renamed_files_keep_capture_order() {
        let photos = vec![
            frame(1, 1_000).named("holiday.CR3"),
            frame(2, 2_000).named("IMG_0451.CR3"),
            frame(3, 3_000).named("DSC00002.CR3"),
        ];
        assert_eq!(
            ordered_paths(&photos),
            ["holiday.CR3", "IMG_0451.CR3", "DSC00002.CR3"]
        );
    }

    #[test]
    fn identical_timestamps_fall_back_to_shutter_count() {
        let photos = vec![
            frame(1, 1_000).named("b.CR3").shutter(50),
            frame(2, 1_000).named("a.CR3").shutter(49),
        ];
        assert_eq!(
            ordered_paths(&photos),
            ["a.CR3", "b.CR3"],
            "the shutter count knows the order; the name does not"
        );
    }

    #[test]
    fn sub_second_tenths_order_frames_inside_one_second() {
        // The 10 ms SubSecTimeOriginal is what separates these; without it they would tie.
        let photos = vec![
            frame(1, 1_090),
            frame(2, 1_000),
            frame(3, 1_040),
            frame(4, 1_010),
        ];
        let ordered = order(&photos);
        assert_eq!(
            ordered.iter().map(|id| id.0).collect::<Vec<_>>(),
            [2, 4, 3, 1]
        );
    }

    #[test]
    fn a_known_shutter_count_beats_a_missing_one_at_the_same_time() {
        let photos = vec![
            frame(1, 1_000).named("zzz.CR3").shutter(9),
            frame(2, 1_000).named("aaa.CR3"),
        ];
        assert_eq!(
            ordered_paths(&photos),
            ["zzz.CR3", "aaa.CR3"],
            "a missing value must not displace a frame whose position is known"
        );
    }

    #[test]
    fn missing_capture_time_falls_back_to_mtime_and_is_flagged() {
        let photos = vec![
            frame(1, 2_000)
                .named("IMG_0002.CR3")
                .without_capture_time()
                .with_mtime(2_000),
            frame(2, 1_000).named("IMG_0001.CR3"),
        ];
        let (_, report) = order_with_report(&photos);
        assert_eq!(
            ordered_paths(&photos),
            ["IMG_0001.CR3", "IMG_0002.CR3"],
            "the photo with a real capture time stays first"
        );
        assert_eq!(report.fallback_times, ["IMG_0002.CR3"]);
        assert!(report.no_time.is_empty());
    }

    #[test]
    fn photos_with_no_time_at_all_sort_last_by_path() {
        let photos = vec![
            frame(2, 0).named("zzz.CR3").without_capture_time(),
            frame(1, 0).named("aaa.CR3").without_capture_time(),
        ];
        let (ids, report) = order_with_report(&photos);
        assert_eq!(ids.iter().map(|id| id.0).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(report.no_time.len(), 2);
        assert!(report.fallback_times.is_empty());
    }

    #[test]
    fn two_bodies_interleave_by_time() {
        let photos = vec![
            frame(1, 1_000).named("a.CR3").serial("BODY1"),
            frame(2, 1_500).named("b.CR3").serial("BODY2"),
            frame(3, 2_000).named("c.CR3").serial("BODY1"),
        ];
        assert_eq!(ordered_paths(&photos), ["a.CR3", "b.CR3", "c.CR3"]);
    }

    #[test]
    fn ordering_does_not_depend_on_the_order_photos_arrive_in() {
        let forward: Vec<MockPhoto> = (0..50u64).map(|i| frame(i, (i * 90) as i64)).collect();
        let expected = ordered_paths(&forward);

        // Every rotation of the input must give the same answer: the scan's file order is arbitrary.
        for shift in 0..forward.len() {
            let mut rotated = forward.clone();
            rotated.rotate_left(shift);
            assert_eq!(ordered_paths(&rotated), expected, "shift {shift}");
        }
    }

    #[test]
    fn ordering_is_repeatable() {
        let photos: Vec<MockPhoto> = (0..50u64).map(|i| frame(i, (i * 90) as i64)).collect();
        let first = order(&photos);
        for _ in 0..50 {
            assert_eq!(order(&photos), first);
        }
    }
}
