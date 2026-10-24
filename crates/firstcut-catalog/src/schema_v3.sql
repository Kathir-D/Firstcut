-- Firstcut session database, schema version 3.
--
-- One additive change: changes made by one action share a group, so undo takes them back together.
--
-- "Reject whole batch" (an opt-in setting, off by default) rejects every frame of a burst in one
-- action. Each frame is still its own history row — the sidecar and the rating are per photo — but
-- one ⌘Z has to undo the action the user took, not the last frame of it. `group_id` is the `seq`
-- of the group's first row; a row with no group is a change of its own, which is every row written
-- before this migration and every single-photo rating after it.

ALTER TABLE history ADD COLUMN group_id INTEGER;

CREATE INDEX history_group ON history (group_id) WHERE group_id IS NOT NULL;
