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
pub const SCHEMA_VERSION: i64 = 1;

const V1: &str = include_str!("schema_v1.sql");

/// One forward-only step.
pub struct Migration {
    pub version: i64,
    pub sql: &'static str,
}

/// Every migration, in order. The last entry's version must equal [`SCHEMA_VERSION`].
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: V1,
}];

/// Brings `conn` up to [`SCHEMA_VERSION`]. A no-op when it is already there.
///
/// Every step runs in its own transaction that also records the new `user_version`, so an
/// interrupted migration leaves the database on the previous version rather than half-migrated.
pub fn migrate(conn: &Connection) -> Result<i64> {
    let mut version = user_version(conn)?;

    if version > SCHEMA_VERSION {
        return Err(StoreError::NewerSchema {
            found: version,
            max: SCHEMA_VERSION,
        });
    }

    for migration in MIGRATIONS {
        if migration.version <= version {
            continue;
        }
        if migration.version > SCHEMA_VERSION {
            return Err(StoreError::MigrationOutOfOrder {
                version: migration.version,
                target: SCHEMA_VERSION,
            });
        }
        apply(conn, migration)?;
        version = migration.version;
    }

    debug_assert_eq!(version, SCHEMA_VERSION);
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
}
