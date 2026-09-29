//! The debounced sidecar writer.
//!
//! A rating keystroke must return in well under a millisecond (docs/contracts/session-api.md), so
//! the sidecar is never written on the caller's thread. Instead the rating goes into a small map,
//! and a background thread writes whatever has been sitting there for [`DEADLINE`] (task.md §6.3:
//! "Each change writes to the DB immediately and to XMP on a debounced background queue (≤ 1 s),
//! flushed on batch change and on quit. A crash never loses more than ~1 s").
//!
//! Debouncing per photo, not per file: rating the same photo three times in a second writes one
//! sidecar, with the last value. The database keeps the exact state, and any sidecar write that had
//! not happened when the app died is still marked pending in the database, so the next session
//! re-queues it — a crash loses nothing at all, and at worst repeats a write.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::document::XmpValues;
use super::write_sidecar;

/// How long a pending sidecar write waits for more changes before it is written.
pub const DEADLINE: Duration = Duration::from_millis(400);

/// One photo's sidecar, and what should end up in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWrite {
    /// The photo's id, so an error can be reported against the photo the user is looking at.
    pub photo_id: u64,
    /// Absolute path of the sidecar to write.
    pub sidecar: PathBuf,
    pub values: XmpValues,
}

/// Where write failures go. Implemented by the session layer, which forwards to
/// `SessionListener::xmp_error` (docs/contracts/session-api.md).
pub trait ErrorSink: Send + Sync {
    fn xmp_error(&self, photo_id: u64, message: String);
}

impl<F> ErrorSink for F
where
    F: Fn(u64, String) + Send + Sync,
{
    fn xmp_error(&self, photo_id: u64, message: String) {
        self(photo_id, message)
    }
}

#[derive(Default)]
struct Queue {
    /// Photo id → the write and when it was queued.
    pending: HashMap<u64, (PendingWrite, Instant)>,
    stopped: bool,
    /// True while the writer thread holds a batch it has taken off the queue but not yet written.
    ///
    /// Without it `flush()` could see an empty queue in the window between taking the writes and
    /// writing them, and return before a single sidecar existed — which is exactly the promise
    /// `flush` makes on quit and on a batch change.
    writing: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    signal: Condvar,
    sink: Arc<dyn ErrorSink>,
    /// Set by `flush` to bring everything forward instead of waiting out the debounce.
    flushing: AtomicBool,
}

/// A background thread that writes sidecars a moment after they change.
pub struct XmpWriter {
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
}

impl XmpWriter {
    /// Starts a writer with the default deadline.
    pub fn new(sink: Arc<dyn ErrorSink>) -> XmpWriter {
        XmpWriter::with_deadline(DEADLINE, sink)
    }

    pub fn with_deadline(deadline: Duration, sink: Arc<dyn ErrorSink>) -> XmpWriter {
        assert!(
            deadline > Duration::ZERO,
            "the debounce deadline must be positive"
        );
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            signal: Condvar::new(),
            sink,
            flushing: AtomicBool::new(false),
        });
        let thread_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("firstcut-xmp".to_string())
            .spawn(move || run(thread_shared, deadline))
            .expect("the XMP writer thread could not be started");
        XmpWriter {
            shared,
            handle: Some(handle),
        }
    }

    /// Queues a sidecar write. Repeated calls for one photo collapse into the last value, and the
    /// deadline restarts, which is what debouncing means here.
    pub fn submit(&self, write: PendingWrite) {
        let due = Instant::now();
        {
            let mut queue = lock(&self.shared.queue);
            if queue.stopped {
                return;
            }
            queue.pending.insert(write.photo_id, (write, due));
        }
        self.shared.signal.notify_all();
    }

    /// How many sidecars are waiting to be written.
    pub fn pending(&self) -> usize {
        lock(&self.shared.queue).pending.len()
    }

    /// Writes everything that is waiting and returns when the queue is empty *and* nothing is
    /// mid-write. Used on batch change and on quit (task.md §6.3).
    pub fn flush(&self) {
        self.shared.flushing.store(true, Ordering::Release);
        self.shared.signal.notify_all();

        let mut queue = lock(&self.shared.queue);
        while (!queue.pending.is_empty() || queue.writing) && !queue.stopped {
            queue = self
                .shared
                .signal
                .wait(queue)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        self.shared.flushing.store(false, Ordering::Release);
    }

    /// Stops the thread, writing anything still pending. Called when the session closes.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        {
            let mut queue = lock(&self.shared.queue);
            if queue.stopped {
                return;
            }
            queue.stopped = true;
        }
        self.shared.flushing.store(true, Ordering::Release);
        self.shared.signal.notify_all();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for XmpWriter {
    fn drop(&mut self) {
        self.stop();
    }
}

impl std::fmt::Debug for XmpWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XmpWriter")
            .field("pending", &self.pending())
            .finish()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn run(shared: Arc<Shared>, deadline: Duration) {
    loop {
        let due;
        {
            let mut queue = lock(&shared.queue);
            loop {
                if queue.stopped {
                    // Write whatever is left, then leave.
                    let pending = all_pending(&queue);
                    let due = take(&mut queue, &pending);
                    drop(queue);
                    write_all(&shared, due);
                    return;
                }

                let flushing = shared.flushing.load(Ordering::Acquire);
                let wait = match earliest(&queue, deadline, flushing) {
                    Some(wait) => wait,
                    None => {
                        // Nothing pending: sleep until something is submitted.
                        queue = shared
                            .signal
                            .wait(queue)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        continue;
                    }
                };

                let (guard, timeout) = shared
                    .signal
                    .wait_timeout(queue, wait)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                queue = guard;
                if timeout.timed_out() || flushing {
                    let ready = due_ids(&queue, deadline, flushing);
                    due = take(&mut queue, &ready);
                    // Marked while the lock is still held, so a `flush` that wakes up next cannot
                    // see an empty queue and return before these have been written.
                    queue.writing = !due.is_empty();
                    break;
                }
            }
        }
        if !due.is_empty() {
            write_all(&shared, due);
            // Cleared after the writes, so `flush` is only released once the files exist.
            lock(&shared.queue).writing = false;
            shared.signal.notify_all();
        }
    }
}

/// How long to wait before the next write is due, or `None` when there is nothing pending.
fn earliest(queue: &Queue, deadline: Duration, flushing: bool) -> Option<Duration> {
    if queue.pending.is_empty() {
        return None;
    }
    if flushing {
        return Some(Duration::ZERO);
    }
    let now = Instant::now();
    queue
        .pending
        .values()
        .map(|(_, submitted)| deadline.saturating_sub(now.saturating_duration_since(*submitted)))
        .min()
}

/// Ids whose debounce window has expired, or all of them when quitting.
fn due_ids(queue: &Queue, deadline: Duration, flushing: bool) -> Vec<u64> {
    let now = Instant::now();
    queue
        .pending
        .iter()
        .filter(|(_, (_, queued))| flushing || now.saturating_duration_since(*queued) >= deadline)
        .map(|(photo_id, _)| *photo_id)
        .collect()
}

fn all_pending(queue: &Queue) -> Vec<u64> {
    queue.pending.keys().copied().collect()
}

fn take(queue: &mut Queue, photo_ids: &[u64]) -> Vec<PendingWrite> {
    let mut taken: Vec<(Instant, PendingWrite)> = photo_ids
        .iter()
        .filter_map(|photo_id| {
            queue
                .pending
                .remove(photo_id)
                .map(|(write, queued)| (queued, write))
        })
        .collect();
    // Oldest first, so sidecars are written in the order the user worked through the shoot.
    taken.sort_by_key(|(queued, _)| *queued);
    taken.into_iter().map(|(_, write)| write).collect()
}

fn write_all(shared: &Shared, writes: Vec<PendingWrite>) {
    for write in writes {
        // One failure must not stop the rest: a read-only card would otherwise hold up every
        // other photo's sidecar. The database still has the rating marked pending, so the next
        // session tries again.
        if let Err(err) = write_sidecar(&write.sidecar, &write.values) {
            shared.sink.xmp_error(write.photo_id, err.to_string());
        }
    }
    shared.signal.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[derive(Default)]
    struct Errors {
        seen: Mutex<Vec<(u64, String)>>,
    }

    impl ErrorSink for Errors {
        fn xmp_error(&self, photo_id: u64, message: String) {
            self.seen.lock().unwrap().push((photo_id, message));
        }
    }

    /// A writer and a handle on the errors it reported.
    fn writer(deadline: Duration) -> (XmpWriter, Arc<Errors>) {
        let errors = Arc::new(Errors::default());
        let writer = XmpWriter::with_deadline(deadline, Arc::clone(&errors) as Arc<dyn ErrorSink>);
        (writer, errors)
    }

    fn sidecar(dir: &Path, name: &str) -> PathBuf {
        dir.join(name)
    }

    fn write(photo_id: u64, sidecar: PathBuf, stars: i64) -> PendingWrite {
        PendingWrite {
            photo_id,
            sidecar,
            values: XmpValues::rating(stars),
        }
    }

    #[test]
    fn a_submitted_write_lands_after_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "IMG_0001.CR3.xmp");
        let (writer, errors) = writer(Duration::from_millis(30));

        writer.submit(write(1, path.clone(), 4));
        assert!(
            !path.exists(),
            "nothing should be written before the deadline"
        );

        writer.flush();
        assert!(path.exists());
        assert_eq!(
            crate::xmp::read_sidecar(&path).unwrap().unwrap().rating,
            Some(4)
        );
        assert_eq!(writer.pending(), 0);
        assert!(errors.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn rapid_changes_collapse_into_one_write_with_the_last_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "IMG_0001.CR3.xmp");
        let (writer, _errors) = writer(Duration::from_millis(40));

        for stars in 1..=5 {
            writer.submit(write(1, path.clone(), stars));
        }
        assert_eq!(writer.pending(), 1, "one photo, one pending write");
        writer.flush();

        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("xmp:Rating").count(), 1);
        assert!(text.contains("xmp:Rating=\"5\""), "the last value wins");
    }

    #[test]
    fn many_photos_are_all_written() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, errors) = writer(Duration::from_millis(10));

        for photo_id in 1..=25u64 {
            writer.submit(write(
                photo_id,
                sidecar(dir.path(), &format!("IMG_{photo_id:04}.CR3.xmp")),
                (photo_id % 5) as i64,
            ));
        }
        writer.flush();
        for photo_id in 1..=25u64 {
            let path = sidecar(dir.path(), &format!("IMG_{photo_id:04}.CR3.xmp"));
            let values = crate::xmp::read_sidecar(&path).unwrap().unwrap();
            assert_eq!(
                values.rating,
                Some((photo_id % 5) as i64),
                "photo {photo_id}"
            );
        }
        assert!(errors.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_bad_write_is_reported_and_does_not_stop_the_queue() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, errors) = writer(Duration::from_millis(10));

        // A directory where a sidecar should be: the write cannot succeed.
        let blocked = dir.path().join("blocked.xmp");
        fs::create_dir(&blocked).unwrap();
        writer.submit(write(1, blocked, 1));
        writer.submit(write(2, sidecar(dir.path(), "IMG_0002.CR3.xmp"), 2));
        writer.flush();

        let seen = errors.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "one failure reported: {seen:?}");
        assert_eq!(seen[0].0, 1);
        drop(seen);
        assert!(
            sidecar(dir.path(), "IMG_0002.CR3.xmp").exists(),
            "the rest of the queue must still be written"
        );
    }

    #[test]
    fn shutdown_writes_what_is_pending() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "IMG_0001.CR3.xmp");
        let (writer, _errors) = writer(Duration::from_secs(30));
        writer.submit(write(1, path.clone(), 5));
        // A 30 s deadline would never elapse inside the test, so quitting has to bring it forward.
        writer.shutdown();
        assert_eq!(
            crate::xmp::read_sidecar(&path).unwrap().unwrap().rating,
            Some(5)
        );
    }

    #[test]
    fn dropping_the_writer_stops_the_thread() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "IMG_0001.CR3.xmp");
        {
            let (writer, _errors) = writer(Duration::from_secs(30));
            writer.submit(write(1, path.clone(), 3));
        }
        assert!(path.exists());
    }

    #[test]
    fn submissions_after_shutdown_are_ignored_rather_than_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, _errors) = writer(Duration::from_millis(10));
        let shared = Arc::clone(&writer.shared);
        let mut writer = writer;
        writer.stop();
        let orphan = XmpWriter {
            shared,
            handle: None,
        };
        orphan.submit(write(1, sidecar(dir.path(), "late.xmp"), 1));
        assert_eq!(orphan.pending(), 0);
    }

    #[test]
    fn a_rating_reaches_the_sidecar_within_a_second() {
        // The promise in task.md §6.3.
        assert!(DEADLINE <= Duration::from_secs(1));
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "IMG_0001.CR3.xmp");
        let (writer, _errors) = writer(DEADLINE);

        let started = Instant::now();
        writer.submit(write(1, path.clone(), 4));
        while !path.exists() && started.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let elapsed = started.elapsed();
        assert!(path.exists(), "the sidecar was never written");
        assert!(elapsed < Duration::from_secs(1), "took {elapsed:?}");
    }

    #[test]
    fn the_writer_is_usable_from_several_threads() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, _errors) = writer(Duration::from_millis(10));
        let writer = Arc::new(writer);

        let threads: Vec<_> = (0..4)
            .map(|thread| {
                let writer = Arc::clone(&writer);
                let dir = dir.path().to_path_buf();
                std::thread::spawn(move || {
                    for photo_id in 0..25u64 {
                        let id = thread * 100 + photo_id;
                        writer.submit(write(
                            id,
                            dir.join(format!("IMG_{id:04}.CR3.xmp")),
                            (id % 6) as i64,
                        ));
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        writer.flush();
        assert_eq!(writer.pending(), 0);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 100);
    }

    #[test]
    fn the_queue_ignores_work_after_it_has_stopped() {
        let mut queue = Queue::default();
        queue
            .pending
            .insert(1, (write(1, PathBuf::from("a.xmp"), 1), Instant::now()));
        assert_eq!(all_pending(&queue), vec![1]);
        let pending = all_pending(&queue);
        let taken = take(&mut queue, &pending);
        assert_eq!(taken.len(), 1);
        assert!(queue.pending.is_empty());
    }
}
