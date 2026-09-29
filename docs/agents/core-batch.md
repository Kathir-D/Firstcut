# Agent: core-batch

> **Charter.** Fixed; edit only with the owner's approval. The **Live status** section at the bottom is
> yours to update. Protocol: [task.md §0.4](../../task.md#04-protocol).

## Mission

Turn a shoot into correct bursts. This is the heart of the app: a wrong merge hides photos, a wrong split slows culling.

## End goal (definition of done for this agent)

`order()` and `batch()` reach ≥ 98% boundary F1 and zero merges of clearly different plays against visually verified ground truth for all four test games; deterministic; < 2 s for 1,500 files; unvisited batches refine with visual signatures.

**Done means (and senior-dev has signed it off in `docs/review.md`):** `firstcut eval` ≥ 98% F1 on all four games in CI (from the committed dumps); qa signed off on the ground truth; timing recorded.

## Owns (only you edit these)

`core/firstcut-core/src/order/`, `src/batch/`, `core/firstcut-cli/` (the CLI crate), `tests/fixtures/meta/`, `tests/fixtures/ground-truth/`

## Does NOT own

Metadata parsing (core-meta), computing VisualSig from pixels in the app (pipeline), persistence of batches (core-store).

## task.md sections to read

§3 Measured facts, §5 Batching (all), §11 rollover fixture, §12 ground-truth testing

## Contracts

- **Owns:** [batching.md](../contracts/batching.md)
- **Consumes:** photo-meta.md (use your exiftool adapter until core-meta's parser lands, so you're never blocked)
- **Provides to others:** `order()`, `batch()`, `visual_sig()` reference implementation, the `firstcut` CLI (`dump-meta`, `batch`, `contact-sheet`, `eval`), meta JSON fixtures for everyone's mocks

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

- [ ] **Wave 1**: freeze batching.md at v1.0 (confirm VisualSig with pipeline, Batch with core-store/app-logic).
- [x] **Bootstrap (done before kickoff)**: raw exiftool dumps in `tests/fixtures/exiftool/<game>.json` (others mock from these now).
- [ ] **Wave 1**: `firstcut dump-meta --from-exiftool <folder>` → `tests/fixtures/meta/<game>.json` in exact `PhotoMeta` field names for all four games; announce it in Notes for other agents when it lands.
- [ ] **Wave 1**: `order()`, handling missing sub-seconds, shutter-count ties, multiple bodies, and file-name rollover (synthetic fixture `IMG_9998 → IMG_0002`).
- [ ] **Wave 1**: metadata-only `batch()` per task.md §5.3; `firstcut batch <folder>` prints batches.
- [ ] **Wave 2**: `firstcut contact-sheet <folder> --out <dir>`: one image per batch plus boundary pairs in the ambiguous zone, for **visual** review.
- [ ] **Wave 2**: ground truth for all four games in `tests/fixtures/ground-truth/<game>.json`, built by actually looking at the contact sheets (with extra care on `IMG_6117–6164` in Game1JENKS). qa independently spot-checks it.
- [ ] **Wave 2**: `firstcut eval`: precision/recall/F1, wrong merges, wrong splits; a CI test on the committed meta dumps.
- [ ] **Wave 3**: `visual_sig()` reference + two-phase refinement with frozen (visited) batches; F1 ≥ 98% with sigs; decide single-frame grouping (task.md §15) from the data and record why.
- [ ] **Wave 4**: tuning; record final thresholds and scores in task.md §5.

## Kickoff prompt

Paste this to start a session for this agent:

```
You are the "core-batch" agent for Firstcut (~/Documents/projects/Firstcut).
Read, in order: task.md §0 (team, protocol, schedule), docs/agents/core-batch.md (your charter + status),
docs/review.md (fix every open finding under "core-batch" and "All agents" first, highest severity first),
docs/contracts/build.md, the contracts listed under "Contracts" in your file, the task.md sections listed
in your file, and the "Requests to others" sections of every other file in docs/agents/.
Work only in your own worktree, ~/Documents/projects/Firstcut-wt/core-batch (branch agent/core-batch, already
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
