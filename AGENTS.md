# Firstcut: agent notes

- Work is split across 9 parallel agents. Start with `task.md` §0 (roster, protocol, schedule), then your own `docs/agents/<agent>.md`, then `docs/review.md` (senior-dev's required changes).
- `task.md` is the source of truth for scope, decisions, and progress. Read it before starting work, and update it (check off tasks, adjust decisions) in the same change.
- Test photos live in `~/Documents/testing` (Canon R8 C-RAW, 4 games, 42 GB). Never copy them into the repo; reference them via `FIRSTCUT_TEST_PHOTOS`.
- Batching changes must be checked against the ground truth in `tests/fixtures/ground-truth/`.
- Performance claims need a measurement (see task.md §7.3), not an estimate.
