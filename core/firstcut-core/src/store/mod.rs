//! Session persistence (todo.md §11). Owner: core-store.
//!
//! One SQLite database per shoot, holding everything needed to resume: which files are in the
//! shoot, how they were batched, every rating, the undo log, the cursor, which batches were seen,
//! and every file operation the Finish step performed. Ratings are mirrored into XMP sidecars by
//! [`crate::xmp`]; this module is the source of truth for the app, and the sidecars are the
//! interoperability copy.
//!
//! Layout:
//!
//! * [`db`] — opening the right database for a folder, and the pragmas that keep it safe.
//! * [`identity`] — volume + path + fingerprint, i.e. what makes a folder "the same shoot".
//! * [`schema`] — the SQL and the forward-only migrations.
//! * [`records`] — typed rows and the only statements in the codebase.
//! * [`rating`] — ratings, flags, labels, tiers and the two rating modes (todo.md §6).
//!
//! Nothing here touches the photo files themselves: the session database lives in Application
//! Support, and no function in this module opens a file inside the shoot.

pub mod db;
pub mod error;
pub mod identity;
pub mod rating;
pub mod records;
pub mod schema;

pub use db::{Db, MatchKind, sessions_dir};
pub use error::{Result, StoreError};
pub use identity::{Fingerprint, FolderIdentity, VolumeIdentity};
pub use rating::{ColorLabel, Flag, Rating, RatingMode, Tier};

use std::time::{SystemTime, UNIX_EPOCH};

/// Unix milliseconds, the timestamp unit used in every column.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_ms_is_a_plausible_timestamp() {
        let now = now_ms();
        // After 2020-01-01 and before 2100: catches seconds-vs-milliseconds mix-ups.
        assert!(now > 1_577_836_800_000, "{now} looks like seconds");
        assert!(now < 4_102_444_800_000, "{now} looks like microseconds");
    }
}
