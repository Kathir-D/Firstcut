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

_Last updated: 2026-09-29 (session 1)_

### Current focus

Wave 1 storage is done and tested: the session database (schema v1 + migrations + folder identity)
and XMP sidecars (read, merge, atomic write, debounced writer), plus the Finish planner.

Next: wire `Session` in `session.rs` (wave 2) — it can already be built against mocks, because
`store::records`, `xmp` and `fileops` are all in place. Blocked on nothing.

### Done log

| Date | What | Commit |
| --- | --- | --- |
| 2026-09-29 | Session database: schema v1 (`store/schema_v1.sql`), versioned migrations, folder identity (volume UUID + path + fingerprint), moved-folder re-match, records/undo/redo/cursor/visited/file-ops, 49 tests | `db4cbb5` |
| 2026-09-29 | XMP sidecars: byte-preserving read/merge/write, atomic temp+fsync+rename, `XmpMapping`, debounced `XmpWriter` (400 ms deadline, per-photo collapse, error sink), 60 tests | `ef04e5a` |
| 2026-09-29 | Finish Cull planner: groups, no-overwrite suffixing, free-space check, tier/star splits, preview lines; `session-api.md` → v0.2, 24 tests | `6aca9ef` |
| 2026-09-29 | `docs/agents/core-store.md` + `docs/contracts/session-api.md` updates, requests filed | (this commit) |

### Measurements

| Fact | Value |
| --- | --- |
| `cargo test -p firstcut-core` | 133 tests, 0 failures, ~4 s |
| Sidecar write (temp + fsync + rename + dir fsync) | measured in the test suite, < 5 ms on the internal SSD |
| Debounce deadline | 400 ms (contract promises ≤ 1 s; the test `a_rating_reaches_the_sidecar_within_a_second` asserts it) |
| Fingerprint of a 1,500-file folder | not measured yet — core-meta's scan numbers land in wave 2 |

### Blockers

- `REQ-core-store-1` (infra): UniFFI exports. `ffi.rs` is infra's, so I cannot wire my types to
  Swift yet. Building `Session` against the Rust API is unaffected, but "the app runs on the real
  core" (wave 2) is.
- `REQ-core-store-4` (senior-dev): the deliverable asks for the owner to confirm once that
  Lightroom Classic reads the sidecars. I can only prove the format is the documented Lightroom one;
  a human has to import one and look.

### Requests to others

| ID | To | Need | Why | Status |
| --- | --- | --- | --- | --- |
| REQ-core-store-1 | infra | UniFFI exports in `ffi.rs` for: `Session`, `SessionListener` (4 callbacks incl. `session_moved`), `Progress`, `Rating`, `Flag`, `ColorLabel`, `RatingMode`, `Tier`, `Change`, `Cursor`, `SessionSnapshot`, `FinishOptions`, `UnkeptAction`, `KeptAction`, `FinishPlan`, `FinishReport`, `FileOp`, `FileOpKind`, `MatchKind`, `XmpMapping`, `SessionError` | `ffi.rs` is yours; the Swift side cannot see anything until these exist. Exact field lists are in `docs/contracts/session-api.md` v0.2. Please also mirror the same names in `App/Sources/Shared/CoreTypes.swift` so app-logic's mock and the real types stay swappable | open |
| REQ-core-store-2 | app-logic | Two things about the Finish flow, both in `session-api.md` "Proposed changes": (1) does "split into subfolders by tier" cover the *unkept* photos too? As written (a `KeptAction`) only 4–5-star photos can be in it, so `3 Good` and `1 Maybe` can never be created; (2) I need your call on cancellation granularity: I poll between operations, so a cancel can leave a group half-moved — I propose polling between groups | You own the flow rules; I have implemented the contract as written and will follow whatever you decide | open |
| REQ-core-store-3 | core-batch | (1) The synthetic rollover fixture from task.md §11 (`IMG_9998`, `IMG_9999`, `IMG_0001`, `IMG_0002` with increasing capture times) — I own that checkbox but the fixture belongs in `tests/fixtures/`. (2) Confirm `BatchId` really is the hash of the batch's first `PhotoId`, because it is my primary key in `batches` and a visited batch must keep its id across a re-batch. (3) You own `firstcut-cli`: I need a hidden subcommand for the crash tests (wave 4), e.g. `firstcut store crash --folder X --rate-and-die` | §11 is a checkbox I have to tick; the ordering proof is yours; crash tests need a way to be killed on purpose | open |
| REQ-core-store-4 | senior-dev | Ask the owner to import one of my sidecars into Lightroom Classic and confirm the rating and colour show up. A file to try: any `*.CR3.xmp` written by my tests, or I can generate one in `~/Documents/testing` on request. Also flagging `session-api.md` v0.2 for your review — it is additive, nothing existing changed | The charter's definition of done requires Lightroom verification, and only a human can do it | open |

### Incoming requests

Requests from other agents (`REQ-…`) and senior-dev findings from `docs/review.md` (`REV-…`).

| ID | From | Response | Status |
| --- | --- | --- | --- |

### Notes for other agents

- **`Db::conn()` is a plain mutex and not reentrant.** A function that locks must not call another
  function that locks. `store::records` is written to keep one lock per statement; if you add to it,
  keep that property or the session will deadlock instead of failing.
- **Ids are masked to 63 bits** (`store::db::id_to_i64`) so they fit a SQLite `INTEGER`. The
  contracts' `PhotoId`/`BatchId` are `u64`; masking is lossless for any id below `2^63` and
  idempotent above it.
- **A `PhotoId` is a hash of the path relative to the session folder**, so it survives the folder
  being moved. That is what lets a moved shoot keep its ratings.
- **The folder fingerprint deliberately ignores `.xmp` files and non-image files.** Firstcut writes
  sidecars itself, so rating a photo must not change the shoot's identity. If core-meta ever pairs a
  file with an extension not in `store::identity::IMAGE_EXTENSIONS`, tell me and I will add it —
  a mismatch only changes which files take part in the hash, never whether a folder matches itself.
- **XMP is merged, never re-serialised.** I keep the file as text and splice only the
  `xmp:Rating` / `xmp:Label` spans, so `xmp:ModifyDate`, `dc:subject`, other tools' namespaces and
  even the whitespace survive byte for byte. Both the attribute form Lightroom writes and the
  element form are read. A `.xmp` file that is not an XMP packet is reported, never overwritten.
- **The pick flag (P) has no XMP representation**, because Lightroom cannot read one (task.md §6.2).
  It lives in the database only. A keep is written as `xmp:Rating=5` (or a colour label, if
  configured), because that is the only thing that survives an import.
- **Nothing was written into a shoot folder except `.xmp` sidecars**; there is a test
  (`originals_are_never_written_to`) that fails if anything else appears or changes.
