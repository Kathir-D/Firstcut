-- Firstcut session database, schema version 4.
--
-- One additive change: how this folder's photos are grouped. 'plays' merges neighbouring bursts
-- into plays; 'bursts' keeps the burst batcher's batches as they are. Plays is the default, so a
-- database written before this migration opens in Plays like a new one.

ALTER TABLE session ADD COLUMN batching_mode TEXT NOT NULL DEFAULT 'plays';
