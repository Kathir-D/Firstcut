//! Opening the session database: where it lives, which one belongs to a folder, and the pragmas
//! that make it safe to write from a background thread (task.md §11).
//!
//! Layout: `~/Library/Application Support/Firstcut/Sessions/<hash>.sqlite`, one file per shoot.
//! See [`crate::store::identity`] for how the name is derived and how a moved folder is re-matched.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OpenFlags};

use super::error::{Result, StoreError};
use super::identity::{FolderIdentity, VolumeIdentity};
use super::schema;

/// `~/Library/Application Support/Firstcut/Sessions`, created on demand.
pub fn sessions_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or(StoreError::NoHomeDir)?;
    Ok(default_sessions_dir(&PathBuf::from(home)))
}

pub fn default_sessions_dir(home: &Path) -> PathBuf {
    home.join("Library")
        .join("Application Support")
        .join("Firstcut")
        .join("Sessions")
}

/// Photo ids and batch ids are `u64` in the contracts; SQLite integers are signed `i64`, and the
/// store must never write a negative one.
///
/// The conversion keeps the **top bit clear**, and that is only safe because every id is produced
/// by `batch::fnv1a64`, which masks to 63 bits. The two have to agree, and the two functions here
/// are the other half of that:
///
/// ```text
/// fnv1a64 -> & 0x7fff_ffff_ffff_ffff  (when the id is built)
/// id_to_i64   -> the same value, as a positive i64
/// i64_to_id   -> the same value back
/// ```
///
/// The bug this shape replaces masked on the way in and on the way out, which is **not** an
/// involution: an id whose top bit was set came back as a different number, so a rating written to
/// the database and read back in the next session was keyed to a photo that did not exist. It is
/// the worst possible failure for a data store -- the write succeeds, the read is confident, and
/// the user's rating is simply gone. Caught by `a_rating_survives_closing_and_reopening`.
pub const ID_MASK: u64 = 0x7fff_ffff_ffff_ffff;

pub fn id_to_i64(id: u64) -> i64 {
    (id & ID_MASK) as i64
}

pub fn i64_to_id(value: i64) -> u64 {
    (value as u64) & ID_MASK
}

/// How the session database that was opened relates to the folder that was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatchKind {
    /// No database existed; a new session was created.
    Created,
    /// The database for exactly this volume + path + fingerprint.
    Exact,
    /// The folder was moved or renamed: a session with the same fingerprint was found and
    /// re-associated. Carries the path the session was last seen at.
    Moved { from: PathBuf },
}

/// One shoot's database.
///
/// The connection is behind a mutex: `Session` is shared across the UI thread and the XMP writer,
/// and SQLite in WAL mode is happy to have one writer at a time.
pub struct Db {
    conn: Mutex<Connection>,
    path: PathBuf,
    identity: FolderIdentity,
    matched: MatchKind,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db")
            .field("path", &self.path)
            .field("matched", &self.matched)
            .finish()
    }
}

impl Db {
    /// Opens (or creates) the session for `folder`, in the user's sessions directory.
    pub fn open(folder: &Path) -> Result<Db> {
        Db::open_in(&sessions_dir()?, folder)
    }

    /// [`Db::open`] against an explicit sessions directory. Tests use this; so does the Finish
    /// step's undo, which has to find the same database again.
    pub fn open_in(sessions_dir: &Path, folder: &Path) -> Result<Db> {
        let identity = FolderIdentity::detect(folder)?;
        let path = identity.db_path(sessions_dir);
        Db::open_with_identity(&path, &identity, sessions_dir)
    }

    /// Opens a specific database file for a known folder.
    pub fn open_with_identity(
        path: &Path,
        identity: &FolderIdentity,
        sessions_dir: &Path,
    ) -> Result<Db> {
        std::fs::create_dir_all(sessions_dir).map_err(|err| {
            StoreError::io(
                format!("creating sessions folder {}", sessions_dir.display()),
                err,
            )
        })?;

        let (path, matched) = if path.exists() {
            (path.to_path_buf(), MatchKind::Exact)
        } else {
            match find_moved_session(sessions_dir, identity)? {
                Some((moved_path, from)) => {
                    // The file name is derived from the volume and the path, so a moved session
                    // has to be re-homed under the name its new location implies. Otherwise every
                    // open would go through the fingerprint scan again.
                    let new_path = identity.db_path(sessions_dir);
                    if moved_path != new_path {
                        if new_path.exists() {
                            return Err(StoreError::Corrupt(format!(
                                "cannot re-home the session for {}: {} already exists",
                                identity.folder.display(),
                                new_path.display()
                            )));
                        }
                        rename_session(&moved_path, &new_path)?;
                    }
                    (new_path, MatchKind::Moved { from })
                }
                None => (path.to_path_buf(), MatchKind::Created),
            }
        };

        Db::attach(path, identity, matched)
    }

    /// Opens an existing (possibly brand new, zero byte) database file and migrates it.
    pub fn open_existing_at(path: &Path) -> Result<Db> {
        let conn = connect(path)?;
        schema::migrate(&conn)?;
        let folder = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(Db {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
            identity: FolderIdentity {
                folder,
                volume: VolumeIdentity::default(),
                fingerprint: super::identity::Fingerprint::default(),
            },
            matched: MatchKind::Exact,
        })
    }

    fn attach(path: PathBuf, identity: &FolderIdentity, matched: MatchKind) -> Result<Db> {
        let conn = connect(&path)?;
        schema::migrate(&conn)?;
        let db = Db {
            conn: Mutex::new(conn),
            path,
            identity: identity.clone(),
            matched,
        };
        if let MatchKind::Moved { from } = &db.matched {
            db.reassociate(identity, from)?;
        } else {
            db.write_session_row(identity)?;
        }
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn identity(&self) -> &FolderIdentity {
        &self.identity
    }

    pub fn matched(&self) -> &MatchKind {
        &self.matched
    }

    /// The shared connection. Held only for the duration of a statement or a transaction.
    ///
    /// The mutex is not reentrant: a function that locks must not call another function that locks,
    /// or it will deadlock. [`crate::store::records`] is written to keep each lock to one
    /// statement.
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn user_version(&self) -> Result<i64> {
        schema::user_version(&self.conn())
    }

    /// Runs `f` inside an immediate transaction, so two writers cannot interleave.
    pub fn transaction<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let value = f(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    /// Writes the singleton `session` row: who we are, where we are, and which mode we are in.
    fn write_session_row(&self, identity: &FolderIdentity) -> Result<()> {
        let now = super::now_ms();
        self.conn().execute(
            "INSERT INTO session (id, schema_version, folder_path, folder_name, volume_uuid,
                                  volume_fsid, fingerprint, rating_mode, photo_count,
                                  created_at_ms, updated_at_ms)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, 'stars', 0, ?7, ?7)
             ON CONFLICT (id) DO UPDATE SET
                folder_path  = excluded.folder_path,
                folder_name  = excluded.folder_name,
                volume_uuid  = excluded.volume_uuid,
                volume_fsid  = excluded.volume_fsid,
                fingerprint  = excluded.fingerprint,
                updated_at_ms = excluded.updated_at_ms",
            rusqlite::params![
                schema::SCHEMA_VERSION,
                identity.folder.to_string_lossy(),
                identity.folder_name(),
                identity.volume.uuid,
                identity.volume.fsid,
                identity.fingerprint.hash,
                now,
            ],
        )?;
        Ok(())
    }

    /// Points an existing session at the folder's new path after a move or a rename.
    fn reassociate(&self, identity: &FolderIdentity, from: &Path) -> Result<()> {
        self.conn().execute(
            "UPDATE session
                SET folder_path = ?2,
                    folder_name = ?3,
                    volume_uuid = ?4,
                    volume_fsid = ?5,
                    fingerprint = ?6,
                    moved_from  = COALESCE(moved_from, ?7),
                    updated_at_ms = ?8
              WHERE id = 1",
            rusqlite::params![
                schema::SCHEMA_VERSION,
                identity.folder.to_string_lossy(),
                identity.folder_name(),
                identity.volume.uuid,
                identity.volume.fsid,
                identity.fingerprint.hash,
                from.to_string_lossy(),
                super::now_ms(),
            ],
        )?;
        Ok(())
    }

    pub fn folder_path(&self) -> Result<Option<PathBuf>> {
        let conn = self.conn();
        let path: Option<String> = conn
            .query_row("SELECT folder_path FROM session WHERE id = 1", [], |row| {
                row.get(0)
            })
            .ok();
        Ok(path.map(PathBuf::from))
    }

    pub fn moved_from(&self) -> Result<Option<PathBuf>> {
        let conn = self.conn();
        let path: Option<String> = conn
            .query_row("SELECT moved_from FROM session WHERE id = 1", [], |row| {
                row.get(0)
            })
            .ok();
        Ok(path.map(PathBuf::from))
    }

    pub fn rating_mode(&self) -> Result<super::rating::RatingMode> {
        let conn = self.conn();
        let mode: String = conn
            .query_row("SELECT rating_mode FROM session WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap_or_else(|_| "stars".to_string());
        Ok(super::rating::RatingMode::from_db(&mode))
    }

    pub fn set_rating_mode(&self, mode: super::rating::RatingMode) -> Result<()> {
        self.conn().execute(
            "UPDATE session SET rating_mode = ?1, updated_at_ms = ?2 WHERE id = 1",
            rusqlite::params![mode.to_db(), super::now_ms()],
        )?;
        Ok(())
    }

    pub fn photo_count(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT photo_count FROM session WHERE id = 1", [], |row| {
                row.get(0)
            })?)
    }

    /// A checkpoint: folds the WAL back into the database file. Called on quit and after the
    /// Finish step, so a session database is a single self-contained file to copy or back up.
    pub fn checkpoint(&self) -> Result<()> {
        self.conn()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }

    /// Durability proof used by the crash tests: everything committed is in the WAL file on disk.
    pub fn wal_bytes(&self) -> Result<u64> {
        let path = PathBuf::from(format!("{}-wal", self.path.display()));
        Ok(std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
    }
}

/// Opens a connection with the pragmas a session database always wants.
fn connect(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).map_err(|err| {
        StoreError::db(format!("opening session database {}", path.display()), err)
    })?;
    // WAL: the UI thread and the XMP writer never block each other, and a crash can never
    // leave a half-written transaction.
    //
    // synchronous = NORMAL is the right trade for WAL: a committed transaction survives the app
    // dying (the WAL is written to the OS), so "a crash loses zero DB writes" holds for
    // kill -9, and only a power loss could lose the last commits. FULL would fsync on every
    // rating and blow the < 1 ms budget in docs/contracts/session-api.md.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

/// Renames a session database together with its write-ahead log, which must travel with it or
/// SQLite would read a stale log against a new file.
fn rename_session(from: &Path, to: &Path) -> Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let source = PathBuf::from(format!("{}{suffix}", from.display()));
        if !source.exists() {
            continue;
        }
        let target = PathBuf::from(format!("{}{suffix}", to.display()));
        std::fs::rename(&source, &target).map_err(|err| {
            StoreError::io(
                format!(
                    "moving session {} to {}",
                    source.display(),
                    target.display()
                ),
                err,
            )
        })?;
    }
    Ok(())
}

/// Looks for a session whose fingerprint matches *and* whose folder is no longer where it was,
/// i.e. the same shoot in a folder that was moved or renamed.
///
/// The second condition is what separates a move from a copy: two cards holding the same shoot have
/// the same fingerprint, but a copy leaves the original in place, so it gets its own session.
/// Returns the database file and the path the session was last opened at.
fn find_moved_session(
    sessions_dir: &Path,
    identity: &FolderIdentity,
) -> Result<Option<(PathBuf, PathBuf)>> {
    let entries = match std::fs::read_dir(sessions_dir) {
        Ok(entries) => entries,
        // No sessions directory yet means no moved session either.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(StoreError::io(
                format!("reading sessions folder {}", sessions_dir.display()),
                err,
            ));
        }
    };

    let mut best: Option<(PathBuf, PathBuf, i64)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sqlite") {
            continue;
        }
        let Some((fingerprint, folder_path, updated_at)) = read_session_header(&path) else {
            continue;
        };
        if fingerprint != identity.fingerprint.hash {
            continue;
        }
        let previous = PathBuf::from(&folder_path);
        if previous == identity.folder || previous.exists() {
            // Either this session already belongs to the folder we are opening, or the folder is
            // still there, so what we have is a copy rather than a move.
            continue;
        }
        // Prefer the most recently used match; a user may have moved a shoot more than once.
        let better = best
            .as_ref()
            .is_none_or(|(_, _, best_updated)| updated_at >= *best_updated);
        if better {
            best = Some((path, previous, updated_at));
        }
    }

    Ok(best.map(|(path, from, _)| (path, from)))
}

/// Reads the singleton session row without migrating or writing anything.
fn read_session_header(path: &Path) -> Option<(String, String, i64)> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.query_row(
        "SELECT fingerprint, folder_path, updated_at_ms FROM session WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::records;
    use std::fs;

    fn shoot(folder: &Path, count: usize) {
        for i in 1..=count {
            fs::write(folder.join(format!("IMG_{i:04}.CR3")), vec![0u8; 16]).unwrap();
        }
    }

    fn new_sessions_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn ids_round_trip_through_sqlite_integers() {
        for id in [0u64, 1, 42, 0x7fff_ffff_ffff_fffe, 0x7fff_ffff_ffff_ffff] {
            assert_eq!(i64_to_id(id_to_i64(id)), id, "id {id} did not round trip");
            assert!(id_to_i64(id) >= 0, "ids must stay positive for SQLite");
        }
        // Only the top bit is ever lost, and masking is idempotent, so a hash that happens to use
        // it still maps to one stable id.
        assert_eq!(id_to_i64(u64::MAX), i64::MAX);
        assert_eq!(id_to_i64(0x8000_0000_0000_0000), 0);
        assert_eq!(i64_to_id(id_to_i64(u64::MAX)), u64::MAX >> 1);
    }

    #[test]
    fn opening_an_empty_folder_creates_a_session() {
        let sessions = new_sessions_dir();
        let shoot_dir = tempfile::tempdir().unwrap();
        shoot(shoot_dir.path(), 3);

        let db = Db::open_in(sessions.path(), shoot_dir.path()).unwrap();
        assert_eq!(db.matched(), &MatchKind::Created);
        assert_eq!(db.user_version().unwrap(), schema::SCHEMA_VERSION);
        assert_eq!(db.photo_count().unwrap(), 0);
        assert!(db.path().starts_with(sessions.path()));
        assert_eq!(
            db.folder_path().unwrap().unwrap(),
            fs::canonicalize(shoot_dir.path()).unwrap()
        );
    }

    #[test]
    fn reopening_the_same_folder_reuses_the_database() {
        let sessions = new_sessions_dir();
        let shoot_dir = tempfile::tempdir().unwrap();
        shoot(shoot_dir.path(), 3);

        let path = {
            let db = Db::open_in(sessions.path(), shoot_dir.path()).unwrap();
            records::upsert_photo(&db, &test_photo(1, "IMG_0001.CR3", 16)).unwrap();
            db.path().to_path_buf()
        };

        let db = Db::open_in(sessions.path(), shoot_dir.path()).unwrap();
        assert_eq!(db.matched(), &MatchKind::Exact);
        assert_eq!(db.path(), path);
        assert_eq!(
            records::photos_count(&db).unwrap(),
            1,
            "the session must survive a reopen"
        );
    }

    #[test]
    fn a_moved_folder_re_matches_its_session() {
        let sessions = new_sessions_dir();
        let parent = tempfile::tempdir().unwrap();
        let original = parent.path().join("Game1");
        fs::create_dir(&original).unwrap();
        shoot(&original, 4);

        let db = Db::open_in(sessions.path(), &original).unwrap();
        records::upsert_photo(&db, &test_photo(1, "IMG_0001.CR3", 16)).unwrap();
        drop(db);

        // The shoot folder is moved, as a drag in Finder would do it.
        let moved = parent.path().join("Game1 renamed");
        fs::rename(&original, &moved).unwrap();

        let db = Db::open_in(sessions.path(), &moved).unwrap();
        assert!(
            matches!(db.matched(), MatchKind::Moved { .. }),
            "expected a moved match, got {:?}",
            db.matched()
        );
        assert_eq!(
            records::photos_count(&db).unwrap(),
            1,
            "ratings and photos must come along"
        );
        assert_eq!(
            db.folder_path().unwrap().unwrap(),
            fs::canonicalize(&moved).unwrap()
        );
        assert!(db.moved_from().unwrap().is_some());

        // And from now on it opens by path.
        let again = Db::open_in(sessions.path(), &moved).unwrap();
        assert!(matches!(again.matched(), MatchKind::Exact));
    }

    #[test]
    fn a_reshoot_in_the_same_folder_gets_its_own_session() {
        let sessions = new_sessions_dir();
        let shoot_dir = tempfile::tempdir().unwrap();
        shoot(shoot_dir.path(), 2);

        let first = Db::open_in(sessions.path(), shoot_dir.path()).unwrap();
        records::upsert_photo(&first, &test_photo(1, "IMG_0001.CR3", 16)).unwrap();
        let first_path = first.path().to_path_buf();
        drop(first);

        // Same folder, different files: that is a different shoot, so the old session is left
        // alone rather than being overwritten.
        shoot(shoot_dir.path(), 5);
        let second = Db::open_in(sessions.path(), shoot_dir.path()).unwrap();
        assert_eq!(second.matched(), &MatchKind::Created);
        assert_ne!(second.path(), first_path);
        assert!(first_path.exists(), "the old session must not be touched");
    }

    #[test]
    fn two_copies_of_a_shoot_do_not_share_a_session() {
        let sessions = new_sessions_dir();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        shoot(a.path(), 3);
        shoot(b.path(), 3);

        let first = Db::open_in(sessions.path(), a.path()).unwrap();
        records::upsert_photo(&first, &test_photo(7, "IMG_0001.CR3", 16)).unwrap();
        let second = Db::open_in(sessions.path(), b.path()).unwrap();

        // Same names and sizes, so the fingerprint matches, but different folders. The path is
        // part of the lookup, and only a *moved* folder is re-matched, never a copy.
        assert_eq!(second.matched(), &MatchKind::Created);
        assert_ne!(second.path(), first.path());
    }

    #[test]
    fn wal_mode_is_on_and_commits_reach_the_file() {
        let sessions = new_sessions_dir();
        let shoot_dir = tempfile::tempdir().unwrap();
        shoot(shoot_dir.path(), 1);
        let db = Db::open_in(sessions.path(), shoot_dir.path()).unwrap();

        let mode: String = db
            .conn()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");

        records::upsert_photo(&db, &test_photo(1, "IMG_0001.CR3", 16)).unwrap();
        assert!(
            db.wal_bytes().unwrap() > 0,
            "the commit should be in the WAL"
        );

        db.checkpoint().unwrap();
        assert_eq!(
            db.wal_bytes().unwrap(),
            0,
            "a checkpoint folds the WAL away"
        );
        assert_eq!(records::photos_count(&db).unwrap(), 1);
    }

    #[test]
    fn rating_mode_is_persisted() {
        let sessions = new_sessions_dir();
        let shoot_dir = tempfile::tempdir().unwrap();
        shoot(shoot_dir.path(), 1);

        let db = Db::open_in(sessions.path(), shoot_dir.path()).unwrap();
        assert_eq!(db.rating_mode().unwrap(), crate::store::RatingMode::Stars);
        db.set_rating_mode(crate::store::RatingMode::KeepNotKeep)
            .unwrap();
        drop(db);

        let db = Db::open_in(sessions.path(), shoot_dir.path()).unwrap();
        assert_eq!(
            db.rating_mode().unwrap(),
            crate::store::RatingMode::KeepNotKeep
        );
    }

    fn test_photo(id: u64, rel_path: &str, size: u64) -> records::PhotoRow {
        records::PhotoRow {
            id,
            rel_path: rel_path.to_string(),
            group_key: rel_path.trim_end_matches(".CR3").to_string(),
            companions: Vec::new(),
            file_size: size,
            mtime_ms: None,
            device: None,
            ino: None,
            meta_json: None,
            ordinal: None,
            first_seen_at_ms: 0,
            last_seen_at_ms: 0,
            present: true,
        }
    }
}
