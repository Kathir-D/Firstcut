//! Typed rows and the statements that move them in and out of the database.
//!
//! Every function takes the shared [`Db`], so all SQL lives in one place and the session layer
//! never writes a statement itself. Timestamps are Unix milliseconds, ids are masked to 63 bits
//! (see [`crate::store::db::id_to_i64`]).

use std::collections::{HashMap, HashSet};

use rusqlite::types::Value;
use rusqlite::{Row, params_from_iter};

use super::db::{Db, i64_to_id, id_to_i64};
use super::error::Result;
use super::rating::{ColorLabel, Flag, Rating, RatingMode};

/// One photo file group, as core-meta described it (docs/contracts/photo-meta.md).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhotoRow {
    pub id: u64,
    /// Primary file relative to the session folder, '/' separated.
    pub rel_path: String,
    /// Shared base name of the group; the XMP sidecar is named after it.
    pub group_key: String,
    /// RAW + paired JPEG/HEIF + any existing `.xmp`.
    pub companions: Vec<String>,
    pub file_size: u64,
    pub mtime_ms: Option<i64>,
    pub device: Option<i64>,
    pub ino: Option<i64>,
    /// `PhotoMeta` as JSON, opaque to this schema.
    pub meta_json: Option<String>,
    /// Position in capture order, set by core-batch.
    pub ordinal: Option<i64>,
    pub first_seen_at_ms: i64,
    pub last_seen_at_ms: i64,
    /// False once the file is gone; the row is kept so a rating survives a file coming back.
    pub present: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchRow {
    pub id: u64,
    pub index: u32,
    pub provisional: bool,
    pub first_ordinal: i64,
    pub last_ordinal: i64,
    /// Members in capture order.
    pub photo_ids: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RatingRow {
    pub photo_id: u64,
    pub rating: Rating,
    /// Bumped on every change, so the app can tell a stale UI value from a current one.
    pub rev: i64,
    /// True while the sidecar still has to be written.
    pub xmp_pending: bool,
    /// The exact `xmp:Rating` value to write, including -1 for reject (todo.md §6.2).
    pub xmp_rating: Option<i64>,
    /// The exact `xmp:Label` value to write.
    pub xmp_label: Option<String>,
    pub updated_at_ms: i64,
}

/// A rating change, with everything needed to undo it (todo.md §6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub seq: i64,
    pub photo_id: u64,
    pub batch_id: u64,
    /// Batch position at the time of the change, so undo can navigate back to it.
    pub batch_index: i64,
    pub before: Rating,
    pub after: Rating,
    pub at_ms: i64,
    /// True once undone: those rows are the redo stack.
    pub undone: bool,
}

/// One physical file operation from the Finish step (todo.md §9.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileOpRow {
    pub id: i64,
    /// Groups every operation from one Finish run.
    pub finish_id: i64,
    /// Execution order within the run; undo walks it backwards.
    pub seq: i64,
    /// `move` · `copy` · `trash` · `delete` · `mark_rejected` · `write_list`.
    pub kind: String,
    pub src: String,
    pub dst: Option<String>,
    pub size_bytes: u64,
    /// `done` · `failed` · `skipped`.
    pub status: String,
    pub error: Option<String>,
    pub at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorRow {
    pub batch_id: Option<u64>,
    pub photo_id: Option<u64>,
    pub batch_index: Option<i64>,
    /// Opaque view state (which view, zoom lock) owned by app-logic.
    pub view: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisitedRow {
    pub batch_id: u64,
    pub batch_index: i64,
    pub last_photo_id: Option<u64>,
    pub at_ms: i64,
}

// ---------------------------------------------------------------- photos

/// Inserts a photo, or refreshes the row for one already known.
///
/// `first_seen_at_ms` is only used on insert; `last_seen_at_ms` and `present` always come from the
/// caller, so a file that disappeared and came back keeps its ratings and its original first-seen
/// time.
pub fn upsert_photo(db: &Db, photo: &PhotoRow) -> Result<()> {
    db.conn().execute(
        "INSERT INTO photos (id, rel_path, group_key, companions, file_size, mtime_ms, device,
                             ino, meta_json, ordinal, first_seen_at_ms, last_seen_at_ms, present)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT (id) DO UPDATE SET
            rel_path   = excluded.rel_path,
            group_key  = excluded.group_key,
            companions = excluded.companions,
            file_size  = excluded.file_size,
            mtime_ms   = excluded.mtime_ms,
            device     = excluded.device,
            ino        = excluded.ino,
            meta_json  = COALESCE(excluded.meta_json, photos.meta_json),
            ordinal    = COALESCE(excluded.ordinal, photos.ordinal),
            last_seen_at_ms = excluded.last_seen_at_ms,
            present    = excluded.present",
        params_from_iter(photo_values(photo).iter()),
    )?;
    Ok(())
}

/// [`upsert_photo`] for a whole shoot, in one transaction. The scan writes 1,500 rows on open.
pub fn upsert_photos(db: &Db, photos: &[PhotoRow]) -> Result<usize> {
    let mut conn = db.conn();
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO photos (id, rel_path, group_key, companions, file_size, mtime_ms, device,
                                 ino, meta_json, ordinal, first_seen_at_ms, last_seen_at_ms, present)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT (id) DO UPDATE SET
                rel_path   = excluded.rel_path,
                group_key  = excluded.group_key,
                companions = excluded.companions,
                file_size  = excluded.file_size,
                mtime_ms   = excluded.mtime_ms,
                device     = excluded.device,
                ino        = excluded.ino,
                meta_json  = COALESCE(excluded.meta_json, photos.meta_json),
                ordinal    = COALESCE(excluded.ordinal, photos.ordinal),
                last_seen_at_ms = excluded.last_seen_at_ms,
                present    = excluded.present",
        )?;
        for photo in photos {
            stmt.execute(params_from_iter(photo_values(photo).iter()))?;
        }
    }
    let count = photos.len();
    tx.execute(
        "UPDATE session SET photo_count = ?1, updated_at_ms = ?2 WHERE id = 1",
        rusqlite::params![count as i64, super::now_ms()],
    )?;
    tx.commit()?;
    Ok(count)
}

fn opt_int(value: Option<i64>) -> Value {
    value.map_or(Value::Null, Value::Integer)
}

/// One row's worth of bound values, in the column order of the two photo statements.
///
/// Owned `Value`s rather than `&dyn ToSql`, so the same list can be built once and used by both
/// the single-row and the bulk statement without borrowing anything temporary.
fn photo_values(photo: &PhotoRow) -> [Value; 13] {
    [
        Value::Integer(id_to_i64(photo.id)),
        Value::Text(photo.rel_path.clone()),
        Value::Text(photo.group_key.clone()),
        Value::Text(photo.companions.join("\n")),
        Value::Integer(photo.file_size as i64),
        opt_int(photo.mtime_ms),
        opt_int(photo.device),
        opt_int(photo.ino),
        photo.meta_json.clone().map_or(Value::Null, Value::Text),
        opt_int(photo.ordinal),
        Value::Integer(photo.first_seen_at_ms),
        Value::Integer(photo.last_seen_at_ms),
        Value::Integer(i64::from(photo.present)),
    ]
}

fn photo_from_row(row: &Row<'_>) -> rusqlite::Result<PhotoRow> {
    let companions: String = row.get(3)?;
    Ok(PhotoRow {
        id: i64_to_id(row.get(0)?),
        rel_path: row.get(1)?,
        group_key: row.get(2)?,
        companions: companions
            .split('\n')
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect(),
        file_size: row.get::<_, i64>(4)?.max(0) as u64,
        mtime_ms: row.get(5)?,
        device: row.get(6)?,
        ino: row.get(7)?,
        meta_json: row.get(8)?,
        ordinal: row.get(9)?,
        first_seen_at_ms: row.get(10)?,
        last_seen_at_ms: row.get(11)?,
        present: row.get::<_, i64>(12)? != 0,
    })
}

const PHOTO_COLUMNS: &str = "id, rel_path, group_key, companions, file_size, mtime_ms, device, \
                             ino, meta_json, ordinal, first_seen_at_ms, last_seen_at_ms, present";

pub fn photo(db: &Db, id: u64) -> Result<Option<PhotoRow>> {
    let conn = db.conn();
    let sql = format!("SELECT {PHOTO_COLUMNS} FROM photos WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([id_to_i64(id)])?;
    match rows.next()? {
        Some(row) => Ok(Some(photo_from_row(row)?)),
        None => Ok(None),
    }
}

/// Every photo, present ones first, in capture order. Falls back to the path when a photo has no
/// ordinal yet (the first open, before core-batch has ordered anything).
pub fn photos_in_order(db: &Db) -> Result<Vec<PhotoRow>> {
    let conn = db.conn();
    let sql = format!(
        "SELECT {PHOTO_COLUMNS} FROM photos
          ORDER BY present DESC, ordinal IS NULL, ordinal, rel_path"
    );
    let mut stmt = conn.prepare(&sql)?;
    let photos = stmt
        .query_map([], photo_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(photos)
}

pub fn photos_count(db: &Db) -> Result<i64> {
    Ok(db
        .conn()
        .query_row("SELECT COUNT(*) FROM photos", [], |row| row.get(0))?)
}

/// Rows for files that are no longer there, so FSEvents can mark them missing.
pub fn photos_not_in(db: &Db, present_ids: &[u64]) -> Result<Vec<PhotoRow>> {
    let conn = db.conn();
    let sql = format!(
        "SELECT {PHOTO_COLUMNS} FROM photos
          WHERE present = 1 AND id NOT IN ({} )
          ORDER BY rel_path",
        present_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ")
    );
    if present_ids.is_empty() {
        // `NOT IN ()` is not valid SQL.
        let mut stmt = conn.prepare(&format!(
            "SELECT {PHOTO_COLUMNS} FROM photos WHERE present = 1 ORDER BY rel_path"
        ))?;
        return Ok(stmt
            .query_map([], photo_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?);
    }
    let ids: Vec<i64> = present_ids.iter().copied().map(id_to_i64).collect();
    let mut stmt = conn.prepare(&sql)?;
    let photos = stmt
        .query_map(params_from_iter(ids.iter()), photo_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(photos)
}

pub fn set_photo_present(db: &Db, id: u64, present: bool) -> Result<()> {
    db.conn().execute(
        "UPDATE photos SET present = ?2, last_seen_at_ms = ?3 WHERE id = ?1",
        rusqlite::params![id_to_i64(id), present as i64, super::now_ms()],
    )?;
    Ok(())
}

pub fn set_photo_ordinal(db: &Db, id: u64, ordinal: i64) -> Result<()> {
    db.conn().execute(
        "UPDATE photos SET ordinal = ?2 WHERE id = ?1",
        rusqlite::params![id_to_i64(id), ordinal],
    )?;
    Ok(())
}

pub fn set_photo_ordinals(db: &Db, ordinals: &[(u64, i64)]) -> Result<()> {
    let mut conn = db.conn();
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare_cached("UPDATE photos SET ordinal = ?2 WHERE id = ?1")?;
        for (id, ordinal) in ordinals {
            stmt.execute(rusqlite::params![id_to_i64(*id), *ordinal])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn set_photo_meta_json(db: &Db, id: u64, meta_json: Option<&str>) -> Result<()> {
    db.conn().execute(
        "UPDATE photos SET meta_json = ?2 WHERE id = ?1",
        rusqlite::params![id_to_i64(id), meta_json],
    )?;
    Ok(())
}

pub fn photo_meta_json(db: &Db, id: u64) -> Result<Option<String>> {
    let conn = db.conn();
    Ok(conn
        .query_row(
            "SELECT meta_json FROM photos WHERE id = ?1",
            [id_to_i64(id)],
            |row| row.get(0),
        )
        .unwrap_or(None))
}

// --------------------------------------------------------------- batches

/// Ids of the batches the user has already seen. These are frozen: a re-batch must not touch them.
pub fn visited_batch_ids(db: &Db) -> Result<HashSet<u64>> {
    let conn = db.conn();
    let ids = conn
        .prepare("SELECT batch_id FROM visited")?
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids.into_iter().map(i64_to_id).collect())
}

/// Replaces the unvisited batches with `batches`, leaving visited ones exactly as they are.
///
/// This is what `Session::submit_visual_sigs` calls when visual signatures improve the boundaries:
/// visited batches and every rating attached to a photo are untouched, and only the batches that
/// can still change are rewritten.
pub fn replace_unvisited_batches(db: &Db, batches: &[BatchRow]) -> Result<()> {
    let frozen = visited_batch_ids(db)?;

    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    if frozen.is_empty() {
        tx.execute("DELETE FROM batches", [])?;
    } else {
        let placeholders = frozen.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let ids: Vec<i64> = frozen.iter().copied().map(id_to_i64).collect();
        tx.execute(
            &format!("DELETE FROM batches WHERE id NOT IN ({placeholders})"),
            params_from_iter(ids.iter()),
        )?;
    }

    {
        let mut insert_batch = tx.prepare_cached(
            "INSERT INTO batches (id, idx, provisional, first_ordinal, last_ordinal)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (id) DO UPDATE SET
                idx = excluded.idx,
                provisional = excluded.provisional,
                first_ordinal = excluded.first_ordinal,
                last_ordinal = excluded.last_ordinal",
        )?;
        let mut insert_member = tx.prepare_cached(
            "INSERT INTO batch_photos (batch_id, photo_id, pos) VALUES (?1, ?2, ?3)
             ON CONFLICT (batch_id, photo_id) DO UPDATE SET pos = excluded.pos",
        )?;
        let mut clear_members =
            tx.prepare_cached("DELETE FROM batch_photos WHERE batch_id = ?1")?;

        for batch in batches {
            if frozen.contains(&batch.id) {
                continue;
            }
            insert_batch.execute(rusqlite::params![
                id_to_i64(batch.id),
                batch.index as i64,
                batch.provisional as i64,
                batch.first_ordinal,
                batch.last_ordinal,
            ])?;
            clear_members.execute([id_to_i64(batch.id)])?;
            for (pos, photo_id) in batch.photo_ids.iter().enumerate() {
                insert_member.execute(rusqlite::params![
                    id_to_i64(batch.id),
                    id_to_i64(*photo_id),
                    pos as i64
                ])?;
            }
        }
    }

    tx.execute(
        "UPDATE session SET updated_at_ms = ?1 WHERE id = 1",
        [super::now_ms()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Every batch with its members, in shoot order.
pub fn batches(db: &Db) -> Result<Vec<BatchRow>> {
    // The lock is taken per statement, not held across the loop: `Db::conn` hands out a plain
    // mutex guard and this function would deadlock on itself.
    let mut rows = {
        let conn = db.conn();
        let mut stmt = conn
            .prepare("SELECT id, idx, provisional, first_ordinal, last_ordinal FROM batches ORDER BY idx, id")?;
        stmt.query_map([], |row| {
            Ok(BatchRow {
                id: i64_to_id(row.get(0)?),
                index: row.get::<_, i64>(1)?.max(0) as u32,
                provisional: row.get::<_, i64>(2)? != 0,
                first_ordinal: row.get(3)?,
                last_ordinal: row.get(4)?,
                photo_ids: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };

    for batch in rows.iter_mut() {
        batch.photo_ids = batch_photo_ids(db, batch.id)?;
    }
    Ok(rows)
}

fn batch_photo_ids(db: &Db, batch_id: u64) -> Result<Vec<u64>> {
    let conn = db.conn();
    let ids = conn
        .prepare("SELECT photo_id FROM batch_photos WHERE batch_id = ?1 ORDER BY pos, photo_id")?
        .query_map([id_to_i64(batch_id)], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids.into_iter().map(i64_to_id).collect())
}

pub fn batch(db: &Db, batch_id: u64) -> Result<Option<BatchRow>> {
    Ok(batches(db)?.into_iter().find(|b| b.id == batch_id))
}

pub fn batches_count(db: &Db) -> Result<i64> {
    Ok(db
        .conn()
        .query_row("SELECT COUNT(*) FROM batches", [], |row| row.get(0))?)
}

// --------------------------------------------------------------- ratings

/// A rating change plus the XMP values it should end up as, written in one statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RatingWrite {
    pub photo_id: u64,
    pub rating: Rating,
    /// `xmp:Rating` to write, or None to leave the attribute alone.
    pub xmp_rating: Option<i64>,
    /// `xmp:Label` to write, or None to leave the attribute alone.
    pub xmp_label: Option<String>,
}

/// Reads a rating row by column name, so the two lists above can never drift apart.
fn rating_from_row(row: &Row<'_>) -> rusqlite::Result<RatingRow> {
    let label: Option<String> = row.get("label")?;
    Ok(RatingRow {
        photo_id: i64_to_id(row.get("photo_id")?),
        rating: Rating {
            stars: row
                .get::<_, i64>("stars")?
                .clamp(0, i64::from(Rating::MAX_STARS)) as u8,
            flag: Flag::from_db(row.get("flag")?),
            label: label.as_deref().and_then(|l| l.parse().ok()),
            keep: row.get::<_, i64>("keep")? != 0,
        },
        rev: row.get("rev")?,
        xmp_pending: row.get::<_, i64>("xmp_pending")? != 0,
        xmp_rating: row.get("xmp_rating")?,
        xmp_label: row.get("xmp_label")?,
        updated_at_ms: row.get("updated_at_ms")?,
    })
}

const RATING_COLUMNS: &str =
    "photo_id, stars, flag, label, keep, rev, xmp_pending, xmp_rating, xmp_label, updated_at_ms";

/// Writes the rating, marks the sidecar dirty, and bumps `rev`. Returns the row as stored.
pub fn write_rating(db: &Db, write: &RatingWrite) -> Result<RatingRow> {
    let now = super::now_ms();
    let rating = Rating::new(
        write.rating.stars,
        write.rating.flag,
        write.rating.label,
        write.rating.keep,
    );
    let label = rating.label.map(|label| label.as_str().to_string());
    db.conn().execute(
        "INSERT INTO ratings (photo_id, stars, flag, label, keep, rev, xmp_pending, xmp_rating,
                              xmp_label, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, 1, ?6, ?7, ?8)
         ON CONFLICT (photo_id) DO UPDATE SET
            stars = excluded.stars,
            flag = excluded.flag,
            label = excluded.label,
            keep = excluded.keep,
            rev = ratings.rev + 1,
            xmp_pending = 1,
            xmp_rating = COALESCE(excluded.xmp_rating, ratings.xmp_rating),
            xmp_label = COALESCE(excluded.xmp_label, ratings.xmp_label),
            updated_at_ms = excluded.updated_at_ms",
        rusqlite::params![
            id_to_i64(write.photo_id),
            i64::from(rating.stars),
            rating.flag.to_db(),
            label,
            rating.keep as i64,
            write.xmp_rating,
            write.xmp_label,
            now,
        ],
    )?;
    rating_of(db, write.photo_id)?.ok_or_else(|| {
        super::error::StoreError::Corrupt(format!("rating {} vanished", write.photo_id))
    })
}

pub fn rating_of(db: &Db, photo_id: u64) -> Result<Option<RatingRow>> {
    let conn = db.conn();
    let sql = format!("SELECT {RATING_COLUMNS} FROM ratings WHERE photo_id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([id_to_i64(photo_id)])?;
    match rows.next()? {
        Some(row) => Ok(Some(rating_from_row(row)?)),
        None => Ok(None),
    }
}

/// Every rating in the session, for the snapshot and for the Finish summary.
pub fn ratings(db: &Db) -> Result<HashMap<u64, Rating>> {
    let conn = db.conn();
    let sql = format!("SELECT {RATING_COLUMNS} FROM ratings");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], rating_from_row)?;
    let mut map = HashMap::new();
    for row in rows {
        let row = row?;
        map.insert(row.photo_id, row.rating);
    }
    Ok(map)
}

pub fn ratings_count(db: &Db) -> Result<i64> {
    Ok(db
        .conn()
        .query_row("SELECT COUNT(*) FROM ratings", [], |row| row.get(0))?)
}

/// Clears the dirty flag after the sidecar has been written. Returns whether there was work to do.
pub fn mark_xmp_written(db: &Db, photo_id: u64) -> Result<bool> {
    let changed = db.conn().execute(
        "UPDATE ratings SET xmp_pending = 0, xmp_rating = NULL, xmp_label = NULL
          WHERE photo_id = ?1 AND xmp_pending = 1",
        [id_to_i64(photo_id)],
    )?;
    Ok(changed > 0)
}

/// Ratings whose sidecar still has to be written, oldest first. On open this is the queue for the
/// writes a crash interrupted, so at most the debounce window is re-done rather than lost.
pub fn pending_xmp_writes(db: &Db) -> Result<Vec<RatingRow>> {
    let conn = db.conn();
    let sql = format!(
        "SELECT {RATING_COLUMNS} FROM ratings WHERE xmp_pending = 1 ORDER BY updated_at_ms, photo_id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], rating_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn count_by_tier(db: &Db, mode: RatingMode) -> Result<HashMap<super::rating::Tier, usize>> {
    let mut counts: HashMap<super::rating::Tier, usize> = super::rating::Tier::ALL
        .into_iter()
        .map(|tier| (tier, 0))
        .collect();
    for rating in ratings(db)?.values() {
        *counts.entry(rating.tier(mode)).or_insert(0) += 1;
    }
    Ok(counts)
}

// --------------------------------------------------------------- history

/// Appends a change to the undo log. Undo and redo are two views of this one table.
///
/// A new change discards whatever could still be redone, as in every editor: redoing a change made
/// before it would apply a stale rating over the new one.
pub fn push_history(db: &Db, entry: &HistoryEntry) -> Result<i64> {
    let conn = db.conn();
    if !entry.undone {
        conn.execute("DELETE FROM history WHERE undone = 1", [])?;
    }
    conn.execute(
        "INSERT INTO history (photo_id, batch_id, batch_index, before_stars, before_flag,
                              before_label, before_keep, after_stars, after_flag, after_label,
                              after_keep, at_ms, undone)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        rusqlite::params![
            id_to_i64(entry.photo_id),
            id_to_i64(entry.batch_id),
            entry.batch_index,
            i64::from(entry.before.stars),
            entry.before.flag.to_db(),
            entry.before.label.map(|label| label.as_str().to_string()),
            entry.before.keep as i64,
            i64::from(entry.after.stars),
            entry.after.flag.to_db(),
            entry.after.label.map(|label| label.as_str().to_string()),
            entry.after.keep as i64,
            entry.at_ms,
            entry.undone as i64,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Reads a history row by column name, for the same reason as [`rating_from_row`].
fn history_from_row(row: &Row<'_>) -> rusqlite::Result<HistoryEntry> {
    let stars = |column: &str| -> rusqlite::Result<u8> {
        Ok(row
            .get::<_, i64>(column)?
            .clamp(0, i64::from(Rating::MAX_STARS)) as u8)
    };
    let label = |column: &str| -> rusqlite::Result<Option<ColorLabel>> {
        Ok(row
            .get::<_, Option<String>>(column)?
            .and_then(|text| text.parse().ok()))
    };
    Ok(HistoryEntry {
        seq: row.get("seq")?,
        photo_id: i64_to_id(row.get("photo_id")?),
        batch_id: i64_to_id(row.get("batch_id")?),
        batch_index: row.get("batch_index")?,
        before: Rating {
            stars: stars("before_stars")?,
            flag: Flag::from_db(row.get("before_flag")?),
            label: label("before_label")?,
            keep: row.get::<_, i64>("before_keep")? != 0,
        },
        after: Rating {
            stars: stars("after_stars")?,
            flag: Flag::from_db(row.get("after_flag")?),
            label: label("after_label")?,
            keep: row.get::<_, i64>("after_keep")? != 0,
        },
        at_ms: row.get("at_ms")?,
        undone: row.get::<_, i64>("undone")? != 0,
    })
}

const HISTORY_COLUMNS: &str = "seq, photo_id, batch_id, batch_index, before_stars, before_flag, \
                               before_label, before_keep, after_stars, after_flag, after_label, \
                               after_keep, at_ms, undone";

/// The newest change that has not been undone: the next `undo()`.
pub fn last_undoable(db: &Db) -> Result<Option<HistoryEntry>> {
    history_one(
        db,
        "SELECT {HISTORY_COLUMNS} FROM history WHERE undone = 0 ORDER BY seq DESC LIMIT 1",
    )
}

/// The change undone most recently: the next `redo()`.
///
/// Undo walks back from the newest change, so the undone changes are always the newest ones
/// (`push_history` discards them when a new change arrives), and the last one undone is the
/// *oldest* of them. Taking the newest instead redid changes out of order: 3 stars then 5, undone
/// twice, redid 5 and then 3.
pub fn last_redoable(db: &Db) -> Result<Option<HistoryEntry>> {
    history_one(
        db,
        "SELECT {HISTORY_COLUMNS} FROM history WHERE undone = 1 ORDER BY seq ASC LIMIT 1",
    )
}

fn history_one(db: &Db, sql: &str) -> Result<Option<HistoryEntry>> {
    let conn = db.conn();
    let mut stmt = conn.prepare(&sql.replace("{HISTORY_COLUMNS}", HISTORY_COLUMNS))?;
    let mut rows = stmt.query([])?;
    match rows.next()? {
        Some(row) => Ok(Some(history_from_row(row)?)),
        None => Ok(None),
    }
}

pub fn set_history_undone(db: &Db, seq: i64, undone: bool) -> Result<()> {
    db.conn().execute(
        "UPDATE history SET undone = ?2 WHERE seq = ?1",
        rusqlite::params![seq, undone as i64],
    )?;
    Ok(())
}

pub fn history_len(db: &Db) -> Result<i64> {
    Ok(db
        .conn()
        .query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))?)
}

pub fn history_for_photo(db: &Db, photo_id: u64) -> Result<Vec<HistoryEntry>> {
    let conn = db.conn();
    let sql = format!("SELECT {HISTORY_COLUMNS} FROM history WHERE photo_id = ?1 ORDER BY seq");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([id_to_i64(photo_id)], history_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Drops the undo log. Only used when a session is reset, never as a side effect of anything else.
pub fn clear_history(db: &Db) -> Result<()> {
    db.conn().execute("DELETE FROM history", [])?;
    Ok(())
}

// -------------------------------------------------------------- file ops

/// Appends one file operation to a Finish run's log. Undo walks `file_ops` in reverse.
pub fn log_file_op(db: &Db, op: &FileOpRow) -> Result<i64> {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO file_ops (finish_id, seq, kind, src, dst, size_bytes, status, error, at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            op.finish_id,
            op.seq,
            op.kind,
            op.src,
            op.dst,
            op.size_bytes as i64,
            op.status,
            op.error,
            op.at_ms,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn set_file_op_status(db: &Db, id: i64, status: &str, error: Option<&str>) -> Result<()> {
    db.conn().execute(
        "UPDATE file_ops SET status = ?2, error = ?3 WHERE id = ?1",
        rusqlite::params![id, status, error],
    )?;
    Ok(())
}

/// Every operation of one Finish run, in execution order.
pub fn file_ops(db: &Db, finish_id: i64) -> Result<Vec<FileOpRow>> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT id, finish_id, seq, kind, src, dst, size_bytes, status, error, at_ms
           FROM file_ops WHERE finish_id = ?1 ORDER BY seq, id",
    )?;
    let rows = stmt
        .query_map([finish_id], |row| {
            Ok(FileOpRow {
                id: row.get(0)?,
                finish_id: row.get(1)?,
                seq: row.get(2)?,
                kind: row.get(3)?,
                src: row.get(4)?,
                dst: row.get(5)?,
                size_bytes: row.get::<_, i64>(6)?.max(0) as u64,
                status: row.get(7)?,
                error: row.get(8)?,
                at_ms: row.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The most recent Finish run, i.e. the one `undo_finish` would reverse.
pub fn last_finish_id(db: &Db) -> Result<Option<i64>> {
    let conn = db.conn();
    let id = conn.query_row("SELECT MAX(finish_id) FROM file_ops", [], |row| {
        row.get::<_, Option<i64>>(0)
    })?;
    Ok(id)
}

pub fn clear_file_ops(db: &Db, finish_id: i64) -> Result<()> {
    db.conn()
        .execute("DELETE FROM file_ops WHERE finish_id = ?1", [finish_id])?;
    Ok(())
}

// ------------------------------------------------------- cursor, visited

pub fn set_cursor(db: &Db, cursor: &CursorRow) -> Result<()> {
    db.conn().execute(
        "INSERT INTO cursor (id, batch_id, photo_id, batch_index, view, updated_at_ms)
         VALUES (1, ?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (id) DO UPDATE SET
            batch_id = excluded.batch_id,
            photo_id = excluded.photo_id,
            batch_index = excluded.batch_index,
            view = excluded.view,
            updated_at_ms = excluded.updated_at_ms",
        rusqlite::params![
            cursor.batch_id.map(id_to_i64),
            cursor.photo_id.map(id_to_i64),
            cursor.batch_index,
            cursor.view,
            cursor.updated_at_ms,
        ],
    )?;
    Ok(())
}

pub fn cursor(db: &Db) -> Result<Option<CursorRow>> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT batch_id, photo_id, batch_index, view, updated_at_ms FROM cursor WHERE id = 1",
    )?;
    let mut rows = stmt.query([])?;
    match rows.next()? {
        Some(row) => Ok(Some(CursorRow {
            batch_id: row.get::<_, Option<i64>>(0)?.map(i64_to_id),
            photo_id: row.get::<_, Option<i64>>(1)?.map(i64_to_id),
            batch_index: row.get(2)?,
            view: row.get(3)?,
            updated_at_ms: row.get(4)?,
        })),
        None => Ok(None),
    }
}

/// Marks a batch as seen, which also freezes it against re-batching.
pub fn mark_visited(
    db: &Db,
    batch_id: u64,
    batch_index: i64,
    last_photo_id: Option<u64>,
) -> Result<()> {
    db.conn().execute(
        "INSERT INTO visited (batch_id, batch_index, last_photo_id, at_ms)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (batch_id) DO UPDATE SET
            batch_index = excluded.batch_index,
            last_photo_id = COALESCE(excluded.last_photo_id, visited.last_photo_id),
            at_ms = excluded.at_ms",
        rusqlite::params![
            id_to_i64(batch_id),
            batch_index,
            last_photo_id.map(id_to_i64),
            super::now_ms(),
        ],
    )?;
    Ok(())
}

pub fn set_visited_last_photo(db: &Db, batch_id: u64, photo_id: u64) -> Result<()> {
    db.conn().execute(
        "UPDATE visited SET last_photo_id = ?2 WHERE batch_id = ?1",
        rusqlite::params![id_to_i64(batch_id), id_to_i64(photo_id)],
    )?;
    Ok(())
}

pub fn visited(db: &Db) -> Result<Vec<VisitedRow>> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT batch_id, batch_index, last_photo_id, at_ms FROM visited ORDER BY batch_index",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(VisitedRow {
                batch_id: i64_to_id(row.get(0)?),
                batch_index: row.get(1)?,
                last_photo_id: row.get::<_, Option<i64>>(2)?.map(i64_to_id),
                at_ms: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn visited_count(db: &Db) -> Result<i64> {
    Ok(db
        .conn()
        .query_row("SELECT COUNT(*) FROM visited", [], |row| row.get(0))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::{Db, MatchKind};
    use crate::store::rating::{RatingMode, Tier};
    use std::fs;
    use std::path::Path;

    struct Shoot {
        _sessions: tempfile::TempDir,
        _files: tempfile::TempDir,
        db: Db,
    }

    impl Shoot {
        fn new(photo_count: usize) -> Shoot {
            let sessions = tempfile::tempdir().unwrap();
            let files = tempfile::tempdir().unwrap();
            for i in 1..=photo_count {
                fs::write(files.path().join(format!("IMG_{i:04}.CR3")), vec![0u8; 8]).unwrap();
            }
            let db = Db::open_in(sessions.path(), files.path()).unwrap();
            assert_eq!(db.matched(), &MatchKind::Created);
            Shoot {
                _sessions: sessions,
                _files: files,
                db,
            }
        }

        fn folder(&self) -> &Path {
            self._files.path()
        }
    }

    fn photo_row(id: u64, index: u64) -> PhotoRow {
        PhotoRow {
            id,
            rel_path: format!("IMG_{index:04}.CR3"),
            group_key: format!("IMG_{index:04}"),
            companions: Vec::new(),
            file_size: 8,
            mtime_ms: Some(1_700_000_000_000),
            device: Some(16777220),
            ino: Some(1000 + index as i64),
            meta_json: None,
            ordinal: Some(index as i64),
            first_seen_at_ms: 1,
            last_seen_at_ms: 1,
            present: true,
        }
    }

    fn batch_row(id: u64, index: u32, photo_ids: Vec<u64>) -> BatchRow {
        BatchRow {
            id,
            index,
            provisional: false,
            first_ordinal: 0,
            last_ordinal: photo_ids.len() as i64,
            photo_ids,
        }
    }

    fn write(db: &Db, photo_id: u64, rating: Rating, xmp_rating: Option<i64>) -> RatingRow {
        write_rating(
            db,
            &RatingWrite {
                photo_id,
                rating,
                xmp_rating,
                xmp_label: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn photos_round_trip_through_the_database() {
        let shoot = Shoot::new(3);
        upsert_photos(
            &shoot.db,
            &[photo_row(1, 1), photo_row(2, 2), photo_row(3, 3)],
        )
        .unwrap();
        assert_eq!(photos_count(&shoot.db).unwrap(), 3);
        assert_eq!(shoot.db.photo_count().unwrap(), 3);

        let mut stored = photo_row(1, 1);
        stored.companions = vec!["IMG_0001.xmp".to_string(), "IMG_0001.CR3".to_string()];
        stored.meta_json = Some("{\"iso\":800}".to_string());
        upsert_photo(&shoot.db, &stored).unwrap();

        let row = photo(&shoot.db, 1).unwrap().unwrap();
        assert_eq!(row.companions, stored.companions);
        assert_eq!(row.meta_json.as_deref(), Some("{\"iso\":800}"));
        assert!(row.present);

        // Ordinals order the shoot; rows without one come last.
        let mut unordered = photo_row(4, 4);
        unordered.ordinal = None;
        upsert_photo(&shoot.db, &unordered).unwrap();
        let ids: Vec<u64> = photos_in_order(&shoot.db)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_photo_that_disappears_keeps_its_row_and_its_rating() {
        let shoot = Shoot::new(3);
        upsert_photos(
            &shoot.db,
            &[photo_row(1, 1), photo_row(2, 2), photo_row(3, 3)],
        )
        .unwrap();
        write(&shoot.db, 2, Rating::stars(4), Some(4));

        let missing = photos_not_in(&shoot.db, &[1, 3]).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].id, 2);

        set_photo_present(&shoot.db, 2, false).unwrap();
        assert!(!photo(&shoot.db, 2).unwrap().unwrap().present);
        assert_eq!(ratings(&shoot.db).unwrap().get(&2), Some(&Rating::stars(4)));

        // Coming back marks it present again.
        set_photo_present(&shoot.db, 2, true).unwrap();
        assert!(photo(&shoot.db, 2).unwrap().unwrap().present);
        assert!(photos_not_in(&shoot.db, &[1, 2, 3]).unwrap().is_empty());
    }

    #[test]
    fn photos_not_in_handles_an_empty_folder() {
        let shoot = Shoot::new(2);
        upsert_photos(&shoot.db, &[photo_row(1, 1), photo_row(2, 2)]).unwrap();
        assert_eq!(photos_not_in(&shoot.db, &[]).unwrap().len(), 2);
    }

    #[test]
    fn ratings_are_versioned_and_mark_the_sidecar_dirty() {
        let shoot = Shoot::new(1);
        upsert_photos(&shoot.db, &[photo_row(1, 1)]).unwrap();

        let first = write(&shoot.db, 1, Rating::stars(3), Some(3));
        assert_eq!(first.rev, 1);
        assert!(first.xmp_pending);

        let second = write(&shoot.db, 1, Rating::stars(5), Some(5));
        assert_eq!(second.rev, 2, "rev must increase on every change");
        assert_eq!(second.rating.stars, 5);
        assert_eq!(second.xmp_rating, Some(5));

        assert!(mark_xmp_written(&shoot.db, 1).unwrap());
        assert!(
            !mark_xmp_written(&shoot.db, 1).unwrap(),
            "nothing left to write"
        );
        let row = rating_of(&shoot.db, 1).unwrap().unwrap();
        assert!(!row.xmp_pending);
        assert_eq!(row.xmp_rating, None);
    }

    #[test]
    fn pending_writes_are_queued_for_a_resume() {
        let shoot = Shoot::new(3);
        upsert_photos(
            &shoot.db,
            &[photo_row(1, 1), photo_row(2, 2), photo_row(3, 3)],
        )
        .unwrap();
        write(&shoot.db, 1, Rating::stars(3), Some(3));
        write(
            &shoot.db,
            2,
            Rating::new(0, Flag::Reject, None, false),
            Some(-1),
        );
        mark_xmp_written(&shoot.db, 2).unwrap();

        let pending = pending_xmp_writes(&shoot.db).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].photo_id, 1);
        assert_eq!(pending[0].xmp_rating, Some(3));
    }

    #[test]
    fn every_rating_field_survives_a_round_trip() {
        let shoot = Shoot::new(1);
        upsert_photos(&shoot.db, &[photo_row(1, 1)]).unwrap();
        let rating = Rating::new(4, Flag::Pick, Some(ColorLabel::Green), true);
        write(&shoot.db, 1, rating, Some(4));
        assert_eq!(ratings(&shoot.db).unwrap()[&1], rating);
    }

    #[test]
    fn tiers_are_counted_per_mode() {
        let shoot = Shoot::new(4);
        upsert_photos(
            &shoot.db,
            &[
                photo_row(1, 1),
                photo_row(2, 2),
                photo_row(3, 3),
                photo_row(4, 4),
            ],
        )
        .unwrap();
        write(&shoot.db, 1, Rating::stars(5), Some(5));
        write(&shoot.db, 2, Rating::stars(3), Some(3));
        write(&shoot.db, 3, Rating::stars(1), Some(1));
        write(
            &shoot.db,
            4,
            Rating::new(0, Flag::Reject, None, false),
            Some(-1),
        );

        let counts = count_by_tier(&shoot.db, RatingMode::Stars).unwrap();
        assert_eq!(counts[&Tier::Keep], 1);
        assert_eq!(counts[&Tier::Good], 1);
        assert_eq!(counts[&Tier::Maybe], 1);
        assert_eq!(counts[&Tier::Rejected], 1);
        assert_eq!(counts[&Tier::Unrated], 0);
        assert_eq!(counts.values().sum::<usize>(), 4);
    }

    /// What the session layer does for one keystroke: write the rating, then log the change so it
    /// can be undone.
    fn rate(
        db: &Db,
        photo_id: u64,
        batch_id: u64,
        batch_index: i64,
        before: Rating,
        after: Rating,
    ) {
        write(db, photo_id, after, Some(i64::from(after.stars)));
        push_history(
            db,
            &HistoryEntry {
                seq: 0,
                photo_id,
                batch_id,
                batch_index,
                before,
                after,
                at_ms: crate::store::now_ms(),
                undone: false,
            },
        )
        .unwrap();
    }

    #[test]
    fn undo_and_redo_walk_one_log() {
        let shoot = Shoot::new(1);
        upsert_photos(&shoot.db, &[photo_row(1, 1)]).unwrap();
        rate(&shoot.db, 1, 10, 7, Rating::neutral(), Rating::stars(1));
        rate(&shoot.db, 1, 10, 7, Rating::stars(1), Rating::stars(4));

        let entry = last_undoable(&shoot.db).unwrap().unwrap();
        assert_eq!(entry.after, Rating::stars(4));
        assert_eq!(entry.before, Rating::stars(1));
        assert_eq!(entry.batch_index, 7);
        set_history_undone(&shoot.db, entry.seq, true).unwrap();

        assert_eq!(
            last_undoable(&shoot.db).unwrap().unwrap().after,
            Rating::stars(1)
        );
        let redo = last_redoable(&shoot.db).unwrap().unwrap();
        assert_eq!(redo.seq, entry.seq);
        set_history_undone(&shoot.db, redo.seq, false).unwrap();
        assert!(last_redoable(&shoot.db).unwrap().is_none());
        assert_eq!(history_len(&shoot.db).unwrap(), 2);

        assert_eq!(history_for_photo(&shoot.db, 1).unwrap().len(), 2);
        clear_history(&shoot.db).unwrap();
        assert!(last_undoable(&shoot.db).unwrap().is_none());
    }

    #[test]
    fn batches_are_replaced_but_visited_ones_are_frozen() {
        let shoot = Shoot::new(6);
        upsert_photos(
            &shoot.db,
            &[
                photo_row(1, 1),
                photo_row(2, 2),
                photo_row(3, 3),
                photo_row(4, 4),
                photo_row(5, 5),
                photo_row(6, 6),
            ],
        )
        .unwrap();
        replace_unvisited_batches(
            &shoot.db,
            &[
                batch_row(10, 0, vec![1, 2, 3]),
                batch_row(20, 1, vec![4, 5, 6]),
            ],
        )
        .unwrap();
        assert_eq!(batches_count(&shoot.db).unwrap(), 2);
        assert_eq!(
            batch(&shoot.db, 20).unwrap().unwrap().photo_ids,
            vec![4, 5, 6]
        );

        // The user has seen the first batch, and rated inside it.
        mark_visited(&shoot.db, 10, 0, Some(3)).unwrap();
        write(&shoot.db, 2, Rating::stars(5), Some(5));
        let before = batches(&shoot.db).unwrap();
        assert_eq!(before[0].photo_ids, vec![1, 2, 3]);

        // Visual signatures re-batch the rest, and merge the first one differently.
        replace_unvisited_batches(
            &shoot.db,
            &[
                batch_row(10, 0, vec![1, 2, 3, 7]),
                batch_row(30, 1, vec![4, 5, 6]),
            ],
        )
        .unwrap();

        let after = batches(&shoot.db).unwrap();
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].id, 10);
        assert_eq!(
            after[0].photo_ids,
            vec![1, 2, 3],
            "a visited batch must not change"
        );
        assert_eq!(after[1].id, 30, "the unvisited batch is replaced");
        assert_eq!(
            ratings(&shoot.db).unwrap()[&2],
            Rating::stars(5),
            "ratings follow photos"
        );
        assert_eq!(visited_batch_ids(&shoot.db).unwrap(), HashSet::from([10]));
    }

    #[test]
    fn replacing_batches_from_scratch_drops_the_old_ones() {
        let shoot = Shoot::new(4);
        upsert_photos(
            &shoot.db,
            &[
                photo_row(1, 1),
                photo_row(2, 2),
                photo_row(3, 3),
                photo_row(4, 4),
            ],
        )
        .unwrap();
        replace_unvisited_batches(
            &shoot.db,
            &[batch_row(10, 0, vec![1, 2]), batch_row(20, 1, vec![3, 4])],
        )
        .unwrap();
        replace_unvisited_batches(&shoot.db, &[batch_row(99, 0, vec![1, 2, 3, 4])]).unwrap();

        let after = batches(&shoot.db).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, 99);
        assert_eq!(after[0].photo_ids, vec![1, 2, 3, 4]);
    }

    #[test]
    fn visited_batches_remember_where_the_user_was() {
        let shoot = Shoot::new(2);
        mark_visited(&shoot.db, 7, 3, Some(11)).unwrap();
        mark_visited(&shoot.db, 8, 4, None).unwrap();
        assert_eq!(visited_count(&shoot.db).unwrap(), 2);

        // Coming back to a batch the user already left keeps the newer position.
        mark_visited(&shoot.db, 7, 3, Some(12)).unwrap();
        set_visited_last_photo(&shoot.db, 8, 22).unwrap();

        let rows = visited(&shoot.db).unwrap();
        assert_eq!(rows[0].batch_id, 7);
        assert_eq!(rows[0].batch_index, 3);
        assert_eq!(rows[0].last_photo_id, Some(12));
        assert_eq!(rows[1].last_photo_id, Some(22));
    }

    #[test]
    fn the_cursor_is_a_single_row_that_always_exists() {
        let shoot = Shoot::new(1);
        assert!(cursor(&shoot.db).unwrap().is_none());
        set_cursor(
            &shoot.db,
            &CursorRow {
                batch_id: Some(5),
                photo_id: Some(6),
                batch_index: Some(2),
                view: Some("loupe:zoom-locked".to_string()),
                updated_at_ms: 42,
            },
        )
        .unwrap();
        set_cursor(
            &shoot.db,
            &CursorRow {
                batch_id: Some(5),
                photo_id: Some(9),
                batch_index: Some(2),
                view: Some("grid".to_string()),
                updated_at_ms: 43,
            },
        )
        .unwrap();
        let conn = shoot.db.conn();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM cursor", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        drop(conn);
        let row = cursor(&shoot.db).unwrap().unwrap();
        assert_eq!(row.photo_id, Some(9));
        assert_eq!(row.view.as_deref(), Some("grid"));
    }

    #[test]
    fn file_ops_are_logged_per_finish_run() {
        let shoot = Shoot::new(2);
        let op = |seq: i64, kind: &str, src: &str, dst: Option<&str>| FileOpRow {
            id: 0,
            finish_id: 1,
            seq,
            kind: kind.to_string(),
            src: src.to_string(),
            dst: dst.map(str::to_string),
            size_bytes: 100,
            status: "done".to_string(),
            error: None,
            at_ms: 1,
        };
        let first = log_file_op(
            &shoot.db,
            &op(0, "move", "IMG_0001.CR3", Some("kept/IMG_0001.CR3")),
        )
        .unwrap();
        log_file_op(&shoot.db, &op(1, "trash", "IMG_0002.CR3", None)).unwrap();
        assert_eq!(last_finish_id(&shoot.db).unwrap(), Some(1));

        set_file_op_status(&shoot.db, first, "failed", Some("read-only volume")).unwrap();
        let ops = file_ops(&shoot.db, 1).unwrap();
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].status, "failed");
        assert_eq!(ops[0].error.as_deref(), Some("read-only volume"));
        assert_eq!(ops[1].kind, "trash");

        log_file_op(
            &shoot.db,
            &op(0, "move", "IMG_0003.CR3", Some("kept/IMG_0003.CR3")),
        )
        .unwrap();
        assert_eq!(last_finish_id(&shoot.db).unwrap(), Some(1));

        clear_file_ops(&shoot.db, 1).unwrap();
        assert!(last_finish_id(&shoot.db).unwrap().is_none());
    }

    #[test]
    fn meta_json_is_kept_when_a_rescan_does_not_carry_it() {
        let shoot = Shoot::new(1);
        upsert_photo(&shoot.db, &photo_row(1, 1)).unwrap();
        set_photo_meta_json(&shoot.db, 1, Some("{\"iso\":6400}")).unwrap();
        assert_eq!(
            photo_meta_json(&shoot.db, 1).unwrap().as_deref(),
            Some("{\"iso\":6400}")
        );

        upsert_photo(&shoot.db, &photo_row(1, 1)).unwrap();
        assert_eq!(
            photo_meta_json(&shoot.db, 1).unwrap().as_deref(),
            Some("{\"iso\":6400}"),
            "a rescan without metadata must not wipe what we had"
        );

        set_photo_ordinals(&shoot.db, &[(1, 5)]).unwrap();
        assert_eq!(photo(&shoot.db, 1).unwrap().unwrap().ordinal, Some(5));
    }

    #[test]
    fn reopening_a_session_restores_everything() {
        let sessions = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        for i in 1..=4 {
            fs::write(files.path().join(format!("IMG_{i:04}.CR3")), vec![0u8; 8]).unwrap();
        }

        {
            let db = Db::open_in(sessions.path(), files.path()).unwrap();
            upsert_photos(
                &db,
                &[
                    photo_row(1, 1),
                    photo_row(2, 2),
                    photo_row(3, 3),
                    photo_row(4, 4),
                ],
            )
            .unwrap();
            replace_unvisited_batches(
                &db,
                &[batch_row(10, 0, vec![1, 2]), batch_row(20, 1, vec![3, 4])],
            )
            .unwrap();
            write(&db, 1, Rating::stars(4), Some(4));
            write(&db, 3, Rating::keep(), Some(5));
            push_history(
                &db,
                &HistoryEntry {
                    seq: 0,
                    photo_id: 1,
                    batch_id: 10,
                    batch_index: 0,
                    before: Rating::neutral(),
                    after: Rating::stars(4),
                    at_ms: 1,
                    undone: false,
                },
            )
            .unwrap();
            mark_visited(&db, 10, 0, Some(2)).unwrap();
            set_cursor(
                &db,
                &CursorRow {
                    batch_id: Some(20),
                    photo_id: Some(3),
                    batch_index: Some(1),
                    view: Some("loupe".to_string()),
                    updated_at_ms: 7,
                },
            )
            .unwrap();
            db.set_rating_mode(RatingMode::KeepNotKeep).unwrap();
        }

        let db = Db::open_in(sessions.path(), files.path()).unwrap();
        assert_eq!(db.matched(), &MatchKind::Exact);
        assert_eq!(photos_count(&db).unwrap(), 4);
        assert_eq!(batches(&db).unwrap().len(), 2);
        assert_eq!(ratings(&db).unwrap()[&1], Rating::stars(4));
        assert!(ratings(&db).unwrap()[&3].keep);
        assert!(last_undoable(&db).unwrap().is_some());
        assert_eq!(visited(&db).unwrap().len(), 1);
        assert_eq!(cursor(&db).unwrap().unwrap().photo_id, Some(3));
        assert_eq!(db.rating_mode().unwrap(), RatingMode::KeepNotKeep);
        assert_eq!(
            pending_xmp_writes(&db).unwrap().len(),
            2,
            "ratings written but not yet flushed to XMP are re-queued"
        );
        assert!(shoot_folder_still_untouched(&files));
    }

    #[test]
    fn originals_are_never_written_to() {
        let shoot = Shoot::new(2);
        let before = fs::metadata(shoot.folder().join("IMG_0001.CR3")).unwrap();
        upsert_photos(&shoot.db, &[photo_row(1, 1), photo_row(2, 2)]).unwrap();
        write(&shoot.db, 1, Rating::stars(5), Some(5));
        shoot.db.checkpoint().unwrap();
        let after = fs::metadata(shoot.folder().join("IMG_0001.CR3")).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        let names: Vec<String> = fs::read_dir(shoot.folder())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names.len(),
            2,
            "the session must not add files to the shoot folder"
        );
    }

    fn shoot_folder_still_untouched(files: &tempfile::TempDir) -> bool {
        fs::read_dir(files.path()).unwrap().count() == 4
    }
}
