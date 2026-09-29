//! Errors from the session store.

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("session database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{context}: {source}")]
    Db {
        context: String,
        #[source]
        source: rusqlite::Error,
    },

    /// The database was written by a newer Firstcut. Refused instead of downgraded.
    #[error("session database is schema version {found}, this build only understands up to {max}")]
    NewerSchema { found: i64, max: i64 },

    /// A migration is newer than `SCHEMA_VERSION`, i.e. the two constants disagree.
    #[error("migration {version} is newer than the declared schema version {target}")]
    MigrationOutOfOrder { version: i64, target: i64 },

    #[error("no such folder: {0}")]
    FolderNotFound(PathBuf),

    #[error("not a folder: {0}")]
    NotAFolder(PathBuf),

    #[error("no home directory, cannot locate the Firstcut application support folder")]
    NoHomeDir,

    #[error("session database is corrupt: {0}")]
    Corrupt(String),
}

impl StoreError {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> StoreError {
        StoreError::Io {
            context: context.into(),
            source,
        }
    }

    pub(crate) fn db(context: impl Into<String>, source: rusqlite::Error) -> StoreError {
        StoreError::Db {
            context: context.into(),
            source,
        }
    }
}
