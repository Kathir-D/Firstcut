//! The Welcome screen's recent shoots, read from the session databases themselves.
//!
//! There is no separate recents file: every shoot already has one database under the sessions
//! directory, and its `session` row carries the folder, the photo count and when it was last
//! opened. Listing those rows keeps one source of truth, and resuming is the same
//! `Session::open_in` path a folder chosen from the dialog takes, so the database's cursor comes
//! back with it (docs/contracts/session-api.md).
//!
//! The sweep is deliberately best-effort. A corrupt or half-written database must not stop the
//! Welcome screen from showing the shoots that are still readable, so per-entry failures are
//! skipped rather than returned.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

/// One shoot the Welcome screen can offer to reopen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentShoot {
    /// The folder path recorded in the database (canonical when the open canonicalized it).
    pub folder: PathBuf,
    /// The folder's display name at the last open.
    pub name: String,
    /// Photographs in the last scan.
    pub photo_count: u32,
    /// Unix milliseconds, written whenever the session row is touched.
    pub updated_at_ms: i64,
}

/// The most recent `limit` shoots whose folders still exist, newest first.
///
/// A session database whose folder has gone (an unmounted volume, a deleted shoot) is omitted:
/// offering a row that cannot open would only produce an error on click. Two databases that
/// record the same folder (a re-homed session left behind) collapse to the newest.
pub fn list(sessions_dir: &Path, limit: usize) -> Vec<RecentShoot> {
    if limit == 0 {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };
    let mut shoots = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("sqlite") {
            continue;
        }
        if let Some(shoot) = read_one(&path)
            && shoot.folder.is_dir()
        {
            shoots.push(shoot);
        }
    }
    shoots.sort_by(|left, right| right.updated_at_ms.cmp(&left.updated_at_ms).then_with(|| left.folder.cmp(&right.folder)));
    let mut seen = HashSet::new();
    shoots.retain(|shoot| seen.insert(shoot.folder.clone()));
    shoots.truncate(limit);
    shoots
}

/// One database's `session` row, or `None` when it is unreadable, not a session, or written by a
/// schema this build does not know. The list only needs columns that have existed since v1.
fn read_one(path: &Path) -> Option<RecentShoot> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    connection
        .query_row("SELECT folder_path, folder_name, photo_count, updated_at_ms FROM session WHERE id = 1", [], |row| {
            Ok(RecentShoot {
                folder: PathBuf::from(row.get::<_, String>(0)?),
                name: row.get(1)?,
                photo_count: u32::try_from(row.get::<_, i64>(2)?.max(0)).unwrap_or(u32::MAX),
                updated_at_ms: row.get(3)?,
            })
        })
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    fn sessions_with_shoots() -> (tempfile::TempDir, tempfile::TempDir, PathBuf, PathBuf) {
        let sessions = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("FirstShoot");
        let second = root.path().join("SecondShoot");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        (sessions, root, first, second)
    }

    #[test]
    fn the_newest_open_comes_first_and_the_folder_is_what_resumes() {
        let (sessions, _root, first, second) = sessions_with_shoots();
        let one = Db::open_in(sessions.path(), &first).unwrap();
        one.conn().execute("UPDATE session SET updated_at_ms = 100, photo_count = 7 WHERE id = 1", []).unwrap();
        drop(one);
        let two = Db::open_in(sessions.path(), &second).unwrap();
        two.conn().execute("UPDATE session SET updated_at_ms = 200, photo_count = 3 WHERE id = 1", []).unwrap();
        drop(two);

        let shoots = list(sessions.path(), 8);
        assert_eq!(shoots.len(), 2);
        assert_eq!(shoots[0].name, "SecondShoot");
        assert_eq!(shoots[0].photo_count, 3);
        assert_eq!(shoots[1].name, "FirstShoot");
        assert_eq!(shoots[1].photo_count, 7);
        assert_eq!(shoots[0].folder, std::fs::canonicalize(&second).unwrap_or(second));
    }

    #[test]
    fn the_limit_trims_the_tail_and_a_vanished_folder_is_not_offered() {
        let (sessions, _root, first, second) = sessions_with_shoots();
        let one = Db::open_in(sessions.path(), &first).unwrap();
        one.conn().execute("UPDATE session SET updated_at_ms = 100 WHERE id = 1", []).unwrap();
        drop(one);
        let two = Db::open_in(sessions.path(), &second).unwrap();
        two.conn().execute("UPDATE session SET updated_at_ms = 200 WHERE id = 1", []).unwrap();
        drop(two);
        std::fs::remove_dir_all(&second).unwrap();

        let shoots = list(sessions.path(), 1);
        assert_eq!(shoots.len(), 1, "the deleted folder is gone from the list");
        assert_eq!(shoots[0].name, "FirstShoot");
        assert!(list(sessions.path(), 0).is_empty(), "a zero limit asks for nothing");
    }

    #[test]
    fn an_unreadable_database_is_skipped_instead_of_failing_the_list() {
        let (sessions, _root, first, _second) = sessions_with_shoots();
        let one = Db::open_in(sessions.path(), &first).unwrap();
        one.conn().execute("UPDATE session SET updated_at_ms = 100 WHERE id = 1", []).unwrap();
        drop(one);
        std::fs::write(sessions.path().join("broken.sqlite"), b"not a database").unwrap();

        let shoots = list(sessions.path(), 8);
        assert_eq!(shoots.len(), 1, "the readable shoot still shows");
        assert_eq!(shoots[0].name, "FirstShoot");
    }

    #[test]
    fn a_missing_sessions_directory_is_an_empty_welcome() {
        let sessions = tempfile::tempdir().unwrap();
        let missing = sessions.path().join("not-created");
        assert!(list(&missing, 8).is_empty());
    }
}
