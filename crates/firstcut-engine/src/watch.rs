//! Live folder watching (docs/v2-plan.md §4.2, §4.4): `notify` tells the engine when a file
//! lands in or leaves the open shoot, and the app reconciles with [`crate::model::Model::rescan_folder`].
//!
//! The receiver carries a single "something changed" token, not the events themselves. Reconcile
//! is a full scan by identity, so batching events is both cheaper and more correct than trying to
//! apply a create/remove/rename stream (FSEvents coalesces and reorders them anyway). A bounded
//! channel means a burst of writes cannot grow a queue nobody is draining — one pending token is
//! all the next frame needs.

use std::path::Path;
use std::sync::mpsc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// Keeps the platform watcher alive for as long as the app holds it. Dropping it stops watching.
pub struct FolderWatcher {
    _watcher: RecommendedWatcher,
}

/// Watches `folder` recursively and reports "the file set may have changed".
pub fn watch(folder: &Path) -> Result<(FolderWatcher, mpsc::Receiver<()>), String> {
    let (sender, receiver) = mpsc::sync_channel::<()>(1);
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        // Errors are reported as a change too: a folder that became unreadable should reconcile
        // into "the photos are gone", not silently keep showing them.
        let _ = result;
        let _ = sender.try_send(());
    })
    .map_err(|err| format!("starting the folder watcher: {err}"))?;
    watcher.watch(folder, RecursiveMode::Recursive).map_err(|err| format!("watching {}: {err}", folder.display()))?;
    Ok((FolderWatcher { _watcher: watcher }, receiver))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_file_landing_in_the_folder_reports_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let (_watcher, events) = watch(dir.path()).expect("the platform watcher starts");
        // Let FSEvents/inotify register the watch before writing (macOS coalesces early events).
        std::thread::sleep(Duration::from_millis(150));
        std::fs::write(dir.path().join("IMG_0001.CR3"), b"not a raw, just a change").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = false;
        while Instant::now() < deadline {
            match events.recv_timeout(Duration::from_millis(100)) {
                Ok(()) => {
                    seen = true;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        assert!(seen, "a write inside the watched folder must wake the reconcile");
    }
}
