-- Firstcut session database, schema version 2.
--
-- One additive change: the capture identity the rename reconcile needs, as a column of its own.
--
-- `reconcile` recognises a renamed file by (device, ino) or, failing that, by the camera's
-- ShutterCount plus the file size. `device` and `ino` were columns; the shutter count was only
-- inside `meta_json`, which is the whole `PhotoMeta` as JSON — kilobytes per row. Reading it meant
-- parsing every photo's metadata twice per scan (once for the swap check, once for the rename
-- match), several megabytes of JSON on a 1,500-frame shoot, all before the first photo is on
-- screen against a "< 1 s to first photo" target (todo.md §7.3).
--
-- The column is additive and nullable: a row written before this migration has no shutter count
-- here, and `meta_json` still carries it, so nothing is lost by the upgrade and the first scan
-- after it fills the column in for every photo it sees. The reconcile therefore falls back to
-- `meta_json` only for rows the new column has nothing to say about — see
-- `records::photo_identity_from_row`.

ALTER TABLE photos ADD COLUMN shutter_count INTEGER;

-- The capture fallback's lookup: "which rows have this shutter count *and* this size". Sized for
-- the capture-number range of a real shoot, not for a lookup table.
CREATE INDEX photos_capture_identity ON photos (shutter_count, file_size);