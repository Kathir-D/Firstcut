# Agent: core-store

> **Charter.** Fixed; edit only with the owner's approval. The **Live status** section at the bottom is
> yours to update. Protocol: [task.md §0.4](../../task.md#04-protocol).

## Mission

Persist everything safely and expose the one `Session` object Swift talks to.

## End goal (definition of done for this agent)

The `Session` API in session-api.md is fully implemented: SQLite session DB, resume, XMP sidecars that Lightroom reads, undo/redo, FSEvents updates, and the Finish Cull planner/executor with undo. Crash tests prove no DB data loss and ≤ 1 s of XMP loss; originals are never modified.

**Done means (and senior-dev has signed it off in `docs/review.md`):** All session-api functions implemented and tested; crash tests pass; Lightroom import of the sidecars verified; finish undo restores every file.

## Owns (only you edit these)

`core/firstcut-core/src/store/`, `src/xmp/`, `src/fileops/`, `src/session.rs`

## Does NOT own

Scanning/parsing (core-meta), batching logic (core-batch; you call it), the Finish UI (ui) and its flow rules (app-logic).

## task.md sections to read

§6 Rating modes (storage + XMP mapping), §9.7 Finish Cull flow (file operations), §11 Session, persistence & data safety

## Contracts

- **Owns:** [session-api.md](../contracts/session-api.md)
- **Consumes:** photo-meta.md, batching.md
- **Provides to others:** `Session`, `SessionListener`, finish planning/execution, XMP read/write

## Who the other agents are

| Agent | What they do | Talk to them about |
| --- | --- | --- |
| **senior-dev** | Technical lead: reviews all your work and files required changes in [`docs/review.md`](../review.md) | Anything under your name in `review.md`, disputes, design questions |
| infra | Build, UniFFI bridge, CI, releases, Homebrew, README | Exporting your types, build breaks, CI |
| core-meta | Metadata parsing for every format | `PhotoMeta` fields |
| core-batch | Ordering, batching, ground truth, CLI | `Batch`, `VisualSig`, fixtures |
| core-store | Session DB, XMP, undo, finish file ops | `Session` API |
| pipeline | Decode, cache, prefetch, viewer layer, zoom | `ImageProvider`, performance |
| app-logic | State model, commands, keymap, rules | `AppModel`, `Command` |
| ui | Every screen, Liquid Glass, Finder look | Layout, visuals |
| qa | Tests, perf baselines, bugs, sign-off | Test hooks, bug reports |

## Deliverables

- [ ] **Wave 1**: freeze session-api.md at v1.0 with app-logic.
- [ ] **Wave 1**: SQLite schema (rusqlite, WAL) + migrations: photos, batches, ratings, history, file_ops, cursor, visited. Session DB location and folder-identity matching (volume UUID + path + fingerprint).
- [ ] **Wave 1**: XMP sidecar read/merge/write (`xmp:Rating`, `xmp:Label`, reject as `-1`), atomic writes, preserving unknown content. Verify Lightroom Classic reads them (ask the owner to confirm once).
- [ ] **Wave 2**: `Session::open` (scan → order → batch → restore from DB, or import from XMP if there's no DB), `set_rating` (sync DB, debounced XMP ≤ 1 s), undo/redo log, cursor, visited, `flush`.
- [ ] **Wave 2**: `submit_visual_sigs` → re-batch unvisited batches → `batches_changed`, keeping ratings attached to photos.
- [ ] **Wave 3**: FSEvents watching; `plan_finish`/`execute_finish`/`undo_finish` with every option in task.md §9.7 (group moves, no overwrite, free-space check, Trash via Finder-compatible trash, permanent delete). Cancellation + progress.
- [ ] **Wave 4**: crash-safety tests (kill -9 during rating bursts and during finish execution) and the resume matrix.

## Kickoff prompt

Paste this to start a session for this agent:

```
You are the "core-store" agent for Firstcut (~/Documents/projects/Firstcut).
Read, in order: task.md §0 (team, protocol, schedule), docs/agents/core-store.md (your charter + status),
docs/review.md (fix every open finding under "core-store" and "All agents" first, highest severity first),
docs/contracts/build.md, the contracts listed under "Contracts" in your file, the task.md sections listed
in your file, and the "Requests to others" sections of every other file in docs/agents/.
Work only in your own worktree, ~/Documents/projects/Firstcut-wt/core-store (branch agent/core-store, already
created), and only on the paths you own. Read other agents' files live from their worktrees
(~/Documents/projects/Firstcut-wt/<agent>/...). All agents are starting at the same time: never wait
for anyone. Build against the v0.1 contracts, CoreTypes.swift, and tests/fixtures/exiftool/, and
file requests for anything missing.
Then pick the next unchecked deliverable, do it, then update your Live status (answer REV findings in
Incoming requests), tick task.md boxes you own,
commit, and push. Ask other agents for anything you need through requests, never by editing their files.
```

---

## Live status

_Last updated: — (not started)_

### Current focus

Not started. Ready to start (bootstrap done, see task.md §0.7).

### Done log

| Date | What | Commit |
| --- | --- | --- |

### Blockers

None.

### Requests to others

| ID | To | Need | Why | Status |
| --- | --- | --- | --- | --- |

### Incoming requests

Requests from other agents (`REQ-…`) and senior-dev findings from `docs/review.md` (`REV-…`).

| ID | From | Response | Status |
| --- | --- | --- | --- |

### Notes for other agents

(Anything others should know: gotchas, measurements, decisions made inside your area.)
