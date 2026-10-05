//! Schema and migrations for the session database (todo.md §11).
//!
//! Migrations are a list of SQL files applied in order and recorded in SQLite's `user_version`.
//! Rules for adding one (see docs/contracts/session-api.md):
//!
//! * Append a new entry to [`MIGRATIONS`]; never edit an existing one. A user who already ran
//!   Firstcut must never see a different meaning for a column.
//! * Additive DDL only (`ALTER TABLE ... ADD COLUMN`, `CREATE TABLE`, `CREATE INDEX`). Rebuilding
//!   a table is fine inside one migration as long as it copies the data.
//! * Bump [`SCHEMA_VERSION`]. Opening a database written by a newer Firstcut is refused rather
//!   than guessed at, so a user who downgrades gets an error instead of silent data loss.

use rusqlite::Connection;

use super::error::{Result, StoreError};

/// The schema version this build expects.
pub const SCHEMA_VERSION: i64 = 2;

const V1: &str = include_str!("schema_v1.sql");
const V2: &str = include_str!("schema_v2.sql");

/// One forward-only step.
pub struct Migration {
    pub version: i64,
    pub sql: &'static str,
}

/// Every migration, in order. The last entry's version must equal [`SCHEMA_VERSION`].
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: V1,
    },
    Migration {
        version: 2,
        sql: V2,
    },
];

/// Brings `conn` up to [`SCHEMA_VERSION`]. A no-op when it is already there.
///
/// Every step runs in its own transaction that also records the new `user_version`, so an
/// interrupted migration leaves the database on the previous version rather than half-migrated.
pub fn migrate(conn: &Connection) -> Result<i64> {
    migrate_with(conn, MIGRATIONS, SCHEMA_VERSION)
}

/// [`migrate`] over an explicit list, which is what makes the two guards in it *testable*:
/// `MigrationOutOfOrder` is unreachable through [`MIGRATIONS`] (whose newest entry is checked
/// against `SCHEMA_VERSION` by construction), so the only way to prove the guard fires is to hand
/// `migrate` a list that disagrees with the declared version.
fn migrate_with(conn: &Connection, migrations: &[Migration], target: i64) -> Result<i64> {
    let mut version = user_version(conn)?;

    if version > target {
        return Err(StoreError::NewerSchema {
            found: version,
            max: target,
        });
    }

    for migration in migrations {
        if migration.version <= version {
            continue;
        }
        if migration.version > target {
            return Err(StoreError::MigrationOutOfOrder {
                version: migration.version,
                target,
            });
        }
        apply(conn, migration)?;
        version = migration.version;
    }

    debug_assert_eq!(version, target);
    Ok(version)
}

fn apply(conn: &Connection, migration: &Migration) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = conn
        .execute_batch(migration.sql)
        .and_then(|()| set_user_version(conn, migration.version));
    match result {
        Ok(()) => {}
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(err.into());
        }
    }
    conn.execute_batch("COMMIT")?;
    Ok(())
}

pub fn user_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?)
}

fn set_user_version(conn: &Connection, version: i64) -> rusqlite::Result<()> {
    // PRAGMA does not take bound parameters.
    conn.execute_batch(&format!("PRAGMA user_version = {version}"))
}

/// `PRAGMA quick_check`, for the resume path: a database that fails this is not worth reading.
pub fn quick_check(conn: &Connection) -> Result<bool> {
    let result: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    Ok(result == "ok")
}

/// Every table this build expects to find, for tests and for the resume path's sanity check.
/// Sorted, so it can be compared with what `sqlite_master` reports.
pub const TABLES: &[&str] = &[
    "batch_photos",
    "batches",
    "cursor",
    "file_ops",
    "history",
    "photos",
    "ratings",
    "session",
    "visited",
];

pub fn table_names(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::Db;
    use crate::store::records;

    #[test]
    fn fresh_database_gets_the_full_schema() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(table_names(&conn).unwrap(), TABLES);
        assert!(quick_check(&conn).unwrap());
    }

    #[test]
    fn migrating_twice_changes_nothing() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute("INSERT INTO cursor (id, updated_at_ms) VALUES (1, 7)", [])
            .unwrap();
        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM cursor", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1, "re-running migrations must not drop data");
    }

    #[test]
    fn a_zero_byte_file_is_a_valid_empty_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.sqlite");
        std::fs::write(&path, b"").unwrap();

        let db = Db::open_existing_at(&path).unwrap();
        assert_eq!(db.user_version().unwrap(), SCHEMA_VERSION);
        assert!(records::photos_count(&db).unwrap() == 0);
    }

    #[test]
    fn a_newer_database_is_refused() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!("PRAGMA user_version = {}", SCHEMA_VERSION + 5))
            .unwrap();
        let err = migrate(&conn).unwrap_err();
        assert!(matches!(err, StoreError::NewerSchema { .. }), "{err}");
    }

    #[test]
    fn migrations_are_ordered_and_complete() {
        assert_eq!(
            MIGRATIONS.last().map(|m| m.version),
            Some(SCHEMA_VERSION),
            "the newest migration must match SCHEMA_VERSION"
        );
        let mut previous = 0;
        for migration in MIGRATIONS {
            assert!(
                migration.version > previous,
                "migration versions must increase"
            );
            previous = migration.version;
        }
    }

    #[test]
    fn every_table_in_the_list_exists_in_the_sql() {
        let sql = MIGRATIONS[0].sql;
        for table in TABLES {
            assert!(
                sql.contains(&format!("CREATE TABLE {table} (")),
                "{table} is listed in TABLES but not created by the migration"
            );
        }
    }

    /// The check the substring search above cannot make: it only ever looks at `MIGRATIONS[0]`, so
    /// it would happily pass while a *later* migration dropped a table, and a `CREATE TABLE`
    /// substring is not the same thing as a table the live database actually has. `PRAGMA
    /// table_info` is the schema itself answering.
    #[test]
    fn every_listed_table_has_the_columns_the_code_reads() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let expected: Vec<(&str, &[&str])> = vec![
            (
                "photos",
                &[
                    "id",
                    "rel_path",
                    "file_size",
                    "device",
                    "ino",
                    "shutter_count",
                ],
            ),
            (
                "ratings",
                &["photo_id", "stars", "keep", "rev", "xmp_pending"],
            ),
            (
                "history",
                &["seq", "photo_id", "before_stars", "after_stars", "undone"],
            ),
            ("batches", &["id", "idx", "provisional"]),
            ("batch_photos", &["batch_id", "photo_id", "pos"]),
            (
                "file_ops",
                &["finish_id", "seq", "kind", "src", "dst", "status"],
            ),
            ("cursor", &["batch_id", "photo_id"]),
            ("visited", &["batch_id", "batch_index"]),
            ("session", &["folder_path", "fingerprint", "rating_mode"]),
        ];
        for (table, columns) in expected {
            let found = column_names(&conn, table);
            assert!(!found.is_empty(), "{table} is not in the migrated schema");
            for column in columns {
                assert!(
                    found.iter().any(|name| name == column),
                    "{table} has no column {column}; it has {found:?}"
                );
            }
        }
    }

    /// `MigrationOutOfOrder` guards against a `MIGRATIONS` entry newer than `SCHEMA_VERSION`, which
    /// `MIGRATIONS` itself can never contain — so the only way to prove the guard fires is to hand
    /// `migrate` a list that disagrees.
    #[test]
    fn a_migration_newer_than_the_declared_version_is_refused() {
        let conn = Connection::open_in_memory().unwrap();
        let err = migrate_with(
            &conn,
            &[Migration {
                version: SCHEMA_VERSION + 1,
                sql: "SELECT 1",
            }],
            SCHEMA_VERSION,
        )
        .unwrap_err();
        match err {
            StoreError::MigrationOutOfOrder { version, target } => {
                assert_eq!(version, SCHEMA_VERSION + 1);
                assert_eq!(target, SCHEMA_VERSION);
            }
            other => panic!("expected MigrationOutOfOrder, got {other:?}"),
        }
        // And it did not apply anything: the database is untouched, not half-migrated.
        assert_eq!(user_version(&conn).unwrap(), 0);
    }

    /// A migration that fails leaves the database on the previous version with its data, so a
    /// user's session survives a bad upgrade rather than opening empty.
    #[test]
    fn a_failing_migration_leaves_the_previous_version_and_its_data() {
        let conn = Connection::open_in_memory().unwrap();
        // Exactly what version 1 was, applied the way `apply` would.
        conn.execute_batch(MIGRATIONS[0].sql).unwrap();
        set_user_version(&conn, 1).unwrap();
        conn.execute(
            "INSERT INTO photos (id, rel_path, group_key, file_size, first_seen_at_ms, last_seen_at_ms)
             VALUES (1, 'IMG_0001.CR3', 'IMG_0001', 16, 1, 1)",
            [],
        )
        .unwrap();
        // Now break version 2's own statement, so the migration fails after the database has data
        // a user would lose. The column it adds is added *already*, so its `ALTER TABLE` fails.
        conn.execute_batch("ALTER TABLE photos ADD COLUMN shutter_count INTEGER")
            .unwrap();

        let err = migrate(&conn).unwrap_err();
        assert!(matches!(err, StoreError::Sqlite(_)), "{err}");
        assert_eq!(
            user_version(&conn).unwrap(),
            1,
            "a failed migration must not record its version"
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM photos", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1,
            "and must not have lost what was already there"
        );
    }

    /// Version 2 on its own, against a database written by version 1: the shutter-count column the
    /// rename reconcile reads must appear, and everything version 1 stored must still be there.
    #[test]
    fn version_two_adds_the_capture_column_without_disturbing_anything_else() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0].sql).unwrap();
        set_user_version(&conn, 1).unwrap();
        conn.execute(
            "INSERT INTO photos (id, rel_path, group_key, file_size, device, ino, meta_json,
                                  first_seen_at_ms, last_seen_at_ms)
             VALUES (7, 'IMG_0001.CR3', 'IMG_0001', 4096, 16777220, 99, '{\"shutterCount\":1234}',
                     1, 1)",
            [],
        )
        .unwrap();

        assert_eq!(migrate(&conn).unwrap(), SCHEMA_VERSION);
        assert!(
            column_names(&conn, "photos")
                .iter()
                .any(|name| name == "shutter_count"),
            "the reconcile's column must exist after the migration"
        );
        let row: (String, i64, i64, Option<i64>) = conn
            .query_row(
                "SELECT rel_path, device, ino, shutter_count FROM photos WHERE id = 7",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, "IMG_0001.CR3");
        assert_eq!((row.1, row.2), (16777220, 99), "the stat identity survived");
        assert_eq!(
            row.3, None,
            "an existing row has no value in the new column until the next scan fills it in; \
             meta_json still carries it, which is why the reconcile falls back to it"
        );
        // And the index that makes the capture lookup cheap is there, so the column is not just
        // present but usable.
        let indexes: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'photos' ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert!(
            indexes.iter().any(|name| name == "photos_capture_identity"),
            "{indexes:?}"
        );
    }

    fn column_names(conn: &Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("table_info");
        stmt.query_map([], |row| row.get::<_, String>(1))
            .expect("table_info runs")
            .map(|row| row.expect("a column"))
            .collect()
    }

    /// `NoHomeDir` had no test at all, and the only thing that can produce it is a `HOME` that is
    /// not set — which a test cannot arrange without unsetting the environment for every other
    /// test in the binary. `sessions_dir` therefore takes the home directory as an argument and
    /// the exported one reads the environment, so the failure is reachable without touching it.
    #[test]
    fn no_home_directory_is_a_named_error_not_a_guess() {
        let missing = crate::store::db::sessions_dir_in(None).unwrap_err();
        assert!(matches!(missing, StoreError::NoHomeDir), "{missing}");
        assert!(
            missing.to_string().contains("home directory"),
            "the message has to say what is missing: {missing}"
        );

        let home = std::path::Path::new("/tmp/firstcut-home");
        assert_eq!(
            crate::store::db::sessions_dir_in(Some(home.to_path_buf())).unwrap(),
            crate::store::db::default_sessions_dir(home),
            "and a home directory gives the documented layout under it"
        );
    }
}
