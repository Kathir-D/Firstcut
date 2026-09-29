-- Firstcut session database, schema version 1.
--
-- This file is the whole state needed to resume a shoot: which files are in it, how they were
-- batched, every rating, the undo/redo log, where the cursor was, which batches were seen, and
-- every file operation the Finish step performed. XMP sidecars are the interoperability copy; this
-- database is the source of truth for the app (task.md §11).
--
-- Conventions, so the Rust side stays simple:
--   * timestamps are Unix milliseconds in an INTEGER column;
--   * booleans are INT 0/1 (STRICT tables reject anything else);
--   * photo/batch ids are the u64 PhotoId/BatchId from the photo-meta and batching contracts
--     masked to 63 bits, so they fit an INTEGER and survive the round trip
--     (see store::db::id_to_i64 / i64_to_id);
--   * paths are relative to the session folder and always use '/';
--   * one row tables (session, cursor) use `id = 1` as a singleton key.

CREATE TABLE session (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    schema_version INTEGER NOT NULL,
    folder_path    TEXT    NOT NULL, -- canonical path when the session was last opened
    folder_name    TEXT    NOT NULL, -- last path component, for the UI
    volume_uuid    TEXT,            -- diskutil VolumeUUID; NULL when it cannot be read
    volume_fsid    TEXT,            -- fallback identity (statfs) when there is no UUID
    fingerprint    TEXT    NOT NULL, -- hash of names + sizes, see store::identity
    moved_from     TEXT,            -- set when a moved folder re-matched this session
    rating_mode    TEXT    NOT NULL DEFAULT 'stars',
    photo_count    INTEGER NOT NULL DEFAULT 0,
    created_at_ms  INTEGER NOT NULL,
    updated_at_ms  INTEGER NOT NULL
) STRICT;

-- One row per photo file group (RAW + companions), keyed by the id core-meta derives from the
-- path relative to the session folder.
CREATE TABLE photos (
    id               INTEGER PRIMARY KEY,
    rel_path         TEXT    NOT NULL UNIQUE, -- primary file, RAW when there is a pair
    group_key        TEXT    NOT NULL,        -- shared base name of the group
    companions       TEXT    NOT NULL DEFAULT '', -- '\n'-separated companion paths
    file_size        INTEGER NOT NULL,
    mtime_ms         INTEGER,
    device           INTEGER,                 -- st_dev, to notice a folder that was replaced
    ino              INTEGER,                 -- st_ino, to notice a rename
    meta_json        TEXT,                    -- PhotoMeta as JSON, opaque to this schema
    ordinal          INTEGER,                 -- position in capture order (core-batch)
    first_seen_at_ms INTEGER NOT NULL,
    last_seen_at_ms  INTEGER NOT NULL,
    present          INTEGER NOT NULL DEFAULT 1  -- 0 once the file goes missing (FSEvents)
) STRICT;

CREATE INDEX photos_present_ordinal ON photos (present, ordinal);
CREATE INDEX photos_group_key ON photos (group_key);

-- Batches, plus the membership table. `visited` batches are frozen: core-batch never changes
-- them, which is what makes undo and the cursor meaningful across a re-batch.
CREATE TABLE batches (
    id            INTEGER PRIMARY KEY,
    idx           INTEGER NOT NULL,  -- 0-based position in the shoot
    provisional   INTEGER NOT NULL DEFAULT 1,
    first_ordinal INTEGER NOT NULL DEFAULT 0,
    last_ordinal  INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE UNIQUE INDEX batches_idx ON batches (idx);

CREATE TABLE batch_photos (
    batch_id INTEGER NOT NULL REFERENCES batches (id) ON DELETE CASCADE,
    photo_id INTEGER NOT NULL REFERENCES photos (id) ON DELETE CASCADE,
    pos      INTEGER NOT NULL, -- position within the batch, capture order
    PRIMARY KEY (batch_id, photo_id)
) STRICT;

CREATE INDEX batch_photos_photo ON batch_photos (photo_id);

-- Current rating per photo, and what still has to reach the sidecar. `xmp_pending` is cleared
-- only after the sidecar has been written (or deliberately skipped), so a crash can lose at most
-- the writes still in the debounce window (task.md §6.3, §11).
CREATE TABLE ratings (
    photo_id      INTEGER PRIMARY KEY REFERENCES photos (id) ON DELETE CASCADE,
    stars         INTEGER NOT NULL DEFAULT 0 CHECK (stars BETWEEN 0 AND 5),
    flag          INTEGER NOT NULL DEFAULT 0,
    label         TEXT,
    keep          INTEGER NOT NULL DEFAULT 0,
    rev           INTEGER NOT NULL DEFAULT 0,     -- bumped on every change
    xmp_pending   INTEGER NOT NULL DEFAULT 0,
    xmp_rating    INTEGER,                       -- exact xmp:Rating value pending (-1 = reject)
    xmp_label     TEXT,                          -- exact xmp:Label value pending
    updated_at_ms INTEGER NOT NULL
) STRICT;

-- Undo/redo log. A row is a single rating change with the full before/after state. `undone = 1`
-- rows are the redo stack.
CREATE TABLE history (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id     INTEGER NOT NULL,
    batch_id     INTEGER NOT NULL,
    batch_index  INTEGER NOT NULL DEFAULT 0, -- so undo can navigate back to that batch
    before_stars INTEGER NOT NULL,
    before_flag  INTEGER NOT NULL,
    before_label TEXT,
    before_keep  INTEGER NOT NULL,
    after_stars  INTEGER NOT NULL,
    after_flag   INTEGER NOT NULL,
    after_label  TEXT,
    after_keep   INTEGER NOT NULL,
    at_ms        INTEGER NOT NULL,
    undone       INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX history_undone_seq ON history (undone, seq);
CREATE INDEX history_photo ON history (photo_id);

-- Every file operation the Finish step performed, grouped by run so one "Undo Finish" can walk it
-- back in reverse (task.md §9.7). Permanent deletes are logged with kind = 'delete' and are not
-- undoable.
CREATE TABLE file_ops (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    finish_id  INTEGER NOT NULL,
    seq        INTEGER NOT NULL, -- execution order within the run
    kind       TEXT    NOT NULL, -- move | copy | trash | delete | mark_rejected | write_list
    src        TEXT    NOT NULL,
    dst        TEXT,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    status     TEXT    NOT NULL, -- done | failed | skipped
    error      TEXT,
    at_ms      INTEGER NOT NULL
) STRICT;

CREATE INDEX file_ops_finish ON file_ops (finish_id, seq);

-- Where the user was (task.md §11: resume restores batch, photo and view).
CREATE TABLE cursor (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    batch_id       INTEGER,
    photo_id       INTEGER,
    batch_index    INTEGER,
    view           TEXT,   -- opaque UI state owned by app-logic, e.g. 'loupe' + zoom lock
    updated_at_ms  INTEGER NOT NULL
) STRICT;

-- Batches the user has seen. A visited batch is frozen for batching, and the session is "done"
-- when every batch is here.
CREATE TABLE visited (
    batch_id      INTEGER PRIMARY KEY,
    batch_index   INTEGER NOT NULL DEFAULT 0,
    last_photo_id INTEGER, -- resume position inside the batch
    at_ms         INTEGER NOT NULL
) STRICT;
