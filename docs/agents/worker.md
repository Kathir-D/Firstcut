# Agent: worker

> **Charter.** Fixed except with the owner's approval. The **Live status** section at the bottom is
> yours to update. Protocol: [task.md §0.4](../../task.md#04-protocol).
>
> You are the only person writing code in this project. That is a real advantage: no handoff, no
> duplicated interface, no waiting for anyone. It is also a real risk — one context carrying nine
> former areas. `task.md` §0.9 and [`docs/review.md`](../review.md) are your memory; if you lose the
> thread, rebuild it from those two files rather than from a summary of a summary.

## Mission

Ship Firstcut v0.1.0: a hyper-fast manual photo culler for macOS on Apple Silicon. Open a folder of
500–1500 RAW files, get them split into bursts, arrow through each burst, rate what you want, and at
the end decide what happens to everything you did not keep. Non-destructive until you explicitly say
otherwise, always dark, indistinguishable from a first-party Apple app, and it never waits.

## End goal

Every deliverable in task.md §0.5 is ticked, all four wave gates are signed off in
[`docs/review.md`](../review.md), and a `v*` tag has published a working, ad-hoc-signed
`Firstcut.app` to GitHub Releases with a live Homebrew cask.

## Owns

Everything except `docs/review.md`, `docs/agents/senior-dev.md` and the merge to `main`. Concretely:
`core/`, `App/`, `scripts/`, `project.yml`, `.github/`, `Casks/`, `tests/fixtures/`, `docs/contracts/`,
`docs/qa/`, `docs/ui/`, `README.md`, `task.md` (its own boxes), and `docs/agents/worker.md`.

## Does NOT own

Merging to `main`, and `docs/review.md`. If you need a change in either, file a request.

## task.md sections to read

All of it, and especially: §2 decisions (locked), §3 measured facts, §5 batching, §6 rating modes,
§7 pipeline and performance, §8 formats, §9 UI, §10 shortcuts, §11 data safety, §12 testing,
§13 build and distribution.

## Contracts

- **Owns:** all six — [build](../contracts/build.md), [photo-meta](../contracts/photo-meta.md),
  [batching](../contracts/batching.md), [session-api](../contracts/session-api.md),
  [pipeline-api](../contracts/pipeline-api.md), [app-model](../contracts/app-model.md).
- **Approver:** senior-dev, for every v1.0 freeze and every breaking change.
- Add a changelog line when a contract's surface changes. Never ship a breaking change unacknowledged.

## The nine areas you now hold

The work is not nine jobs, it is one product. These are the areas, with what each must end up doing.

| Area | Must end up doing | Code |
| --- | --- | --- |
| **Build & infra** | `scripts/build-app.sh` → signed `dist/Firstcut.app`; CI green on every push; a `v*` tag publishes a zip with its SHA-256 and bumps the cask | `scripts/`, `.github/`, `project.yml`, `Casks/`, `VERSION`, `core/*/Cargo.toml`, `ffi.rs` |
| **Metadata** | `scan_folder()` returns a complete `PhotoMeta` for all 2,880 test files in < 3 s, matching exiftool field for field; every §8 format parses from spec | `core/…/{scan,meta,formats}/` |
| **Batching** | `order()` + `batch()` reach ≥ 98% boundary F1 against visually verified ground truth, deterministic, < 2 s for 1,500 files | `core/…/{order,batch}/`, `firstcut-cli/` |
| **Store** | Session DB, XMP sidecars, undo/redo, resume, Finish Cull file ops. Never touch an original | `core/…/{store,xmp,fileops}/`, `session.rs` |
| **Pipeline** | Holding → through a whole game never misses the cache in the previous/current/next batch; every §7.3 target met **and measured** | `App/Sources/{Pipeline,Render}/` |
| **App logic** | Every command works end to end through `AppModel` with undo, both rating modes, auto-advance, remappable keys, all unit-tested | `App/Sources/{Session,Input,Settings/Model}/` |
| **UI** | Every §9 screen exists, driven only by `AppModel`, and passes a side-by-side check with Finder on macOS 26 and 15 | `App/Sources/{App,Views,Settings/Views}/`, `docs/ui/` |
| **QA** | Integration, performance and stress suites for all four games in both modes; baselines recorded; no open P0/P1 bugs | `App/Tests/{Integration,Performance}/`, `docs/qa/` |

## The state you inherited

Nine agents ran for one hour. Everything below is **on the branches, not on `main`**, and is
unverified. `main` contains only the foundation commit (infra's UniFFI bridge, build scripts and CI).

- `agent/infra` (4 commits) — the UniFFI → xcframework → static framework chain. **Verified working**:
  I ran `build-core.sh` → `xcodegen` → `xcodebuild test`, exit 0. Also has a release workflow and
  Homebrew cask with no PR against it yet.
- `agent/core-batch` (2) — `order()`, pair signals, the scorer, `visual_sig()`. Real tests, including
  the rollover fixture. One known bug, REV-63.
- `agent/core-store` (5) — DB + WAL, folder identity, rating/tier logic. 3,000 lines, tests, good
  reasoning. Open: REV-68, REV-69.
- `agent/app-logic` (1) — `AppModel`, the full command set, keymap, router, mock session. Caps Lock
  handled correctly. The types are renamed away from the contract (REV-73) and the pipeline types are
  mirrored (REV-72) — with one agent, both are just deletions.
- `agent/ui` (3) — window, toolbar, menus, filmstrip, HUD, welcome, behind a protocol. Best-structured
  work of the nine. Known bug: REV-75, the welcome view over the filmstrip.
- `agent/qa` (3) — harness, fixture loader, perf baseline format, QA checklist.
- `agent/pipeline` (1) — a decode spike measuring the right things (§7.2's preview-vs-RAW question,
  Lanczos vs vImage, sharpness, grain). No types declared yet.
- `agent/core-meta` (0) — nothing written. It is the critical path: nothing real can be measured
  until the CR3 parser matches exiftool.

`docs/agents/archive/` keeps the eight original charters and their live-status records.

## Deliverables

Ordered. Each is done when senior-dev has reviewed it with no open P0/P1, not when the code compiles.

- [ ] **0. Rescue and consolidate.** Merge the eight branches into one, resolve the six shared-file
      conflicts, get `cargo test` and `xcodebuild test` green on the merged tree. Delete the
      stand-in and mirrored types so there is exactly one definition of each (REV-56, REV-72, REV-73).
- [ ] **1. Foundations gate.** Fix every open P0/P1 in `review.md`. Freeze the six contracts at v1.0.
      CI enforcing fmt, clippy, warnings-as-errors and a ratcheting `swift-format` (REV-12, REV-59).
- [ ] **2. CR3 parser** complete for §7.4, verified field-for-field against `exiftool` on all four
      games, `pread` headers only, `< 2 ms/file`. Fixtures for the other formats in §8, each failing
      gracefully rather than panicking.
- [ ] **3. Ordering and batching.** The REV-63 fix first: a backwards time gap must never be a hard
      join, and a pair with a fallback timestamp must never be a hard join. Then the §5.3 algorithm,
      `firstcut batch`, and contact sheets.
- [ ] **4. Ground truth and F1.** `tests/fixtures/ground-truth/<game>.json` for all four games, built
      by **looking at the photographs** — every ambiguous-zone boundary, especially
      `Game1JENKS IMG_6117–6164`. `firstcut eval` reporting precision/recall/F1, wrong merges, wrong
      splits, with a CI test on the committed dumps. Target ≥ 98% boundary F1, zero merges of
      clearly different plays. The rollover and scrambled-name regressions, because the test set
      cannot catch name-based ordering (REV-26).
- [ ] **5. Store.** `Session::open` end to end, XMP read/merge/write with atomic writes, undo/redo
      keyed to photos, resume, and the rename reconciliation (REV-68). Verify Lightroom reads the
      sidecars. Crash tests: no DB loss, ≤ 1 s of XMP loss, originals untouched.
- [ ] **6. Pipeline.** T0 thumbnails for the whole shoot at `.utility`, `visual_sig` called from the
      Rust reference (never reimplemented in Swift), T2 for the previous/current/next batch, the
      priority scheduler with instant re-prioritisation, the memory budget and pressure shedding.
      Then `PhotoViewerLayerView`. Resolve the display mechanism and the `Sendable` question (REV-37,
      REV-38) before writing the type.
- [ ] **7. App logic on the real thing.** `AppModel` against the real `Session` and the real pipeline.
      Both rating modes, current-batch-only rating, auto-advance, undo that navigates across batches,
      remappable keys with conflict detection, Settings persistence, the keymap editor, and the
      Finish flow state machine.
- [ ] **8. Every screen.** Finder gallery layout, unified toolbar with the ‹ › glass capsule,
      filmstrip, info panel, grid, compare, HUD, welcome, Finish sheet, Settings. **Screenshot the
      running app and look at it** — see below. Forced dark. Liquid Glass on 26+, the closest
      `NSVisualEffectView` fallback on 15.
- [ ] **9. Measured.** Every §7.3 row has a real number in `docs/qa/perf-baselines.md` and in task.md
      §3. Stress test: hold → through a whole game, zero focus misses, flat memory. No claim without
      a measurement.
- [ ] **10. Ship.** Accessibility, app icon, README screenshots, `v*` tag, cask, v0.1.0.

## Computer use — you can see the app, so look at it

Screen Recording and Accessibility are granted on this machine. This loop is verified working, and it
is how the UI gets reviewed rather than guessed at:

```sh
scripts/build-app.sh --open            # or xcodebuild … && open the .app
osascript -e 'tell application "System Events" to tell process "Firstcut" \
  to get {name, position, size} of window 1'          # -> Firstcut, 80, 40, 1120, 680
screencapture -x -o -R80,40,1120,680 docs/ui/shot-<what>.png
osascript -e 'tell application "System Events" to tell process "Firstcut" to key code 124'   # →
osascript -e 'tell application "System Events" to tell process "Firstcut" to keystroke "p"'
```

Then **read the PNG back as an image and actually look at it.** A screenshot nobody looks at is worth
nothing. Compare it with the Finder reference in `docs/ui/` and with task.md §9, then fix what is
wrong and look again. Save before/after pairs in `docs/ui/`.

This is not optional polish: REV-75, a welcome view composited over the filmstrip, was found this way
and is invisible in the source, the diff and any test.

## Kickoff prompt

Paste this to start a session:

```
You are the "worker" agent for Firstcut. You are the only person writing code in
this project. Work in ~/Documents/projects/Firstcut-wt/worker on branch
agent/worker — create it from agent/infra's branch state if it does not exist, and
merge the eight agent/* branches into it, resolving the conflicts in the six shared
files (core/Cargo.lock, core/*/Cargo.toml, App/Sources/Shared/CoreTypes.swift,
project.yml, docs/contracts/build.md, task.md). Do that FIRST, before any new code:
the goal is one branch containing everything the nine agents wrote, building and
testing green.

Read, in this order, from the LIVE paths (your own checkout's review.md is stale):
  1. ~/Documents/projects/Firstcut-wt/senior-dev/docs/review.md   <- 74 findings, fix P0/P1 first
  2. docs/agents/worker.md            <- your charter, deliverables, inherited state
  3. task.md §0 (protocol) then §0.9 (the critical path, top to bottom)
  4. docs/contracts/build.md, then the five contracts you now own
  5. docs/agents/archive/*.md         <- the nine original charters, for the detail

One branch, one PR, and never merge it yourself — senior-dev merges. Commit and push
often so integration never meets a week of unpushed work. Never force-push, never
rebase main; git merge origin/main into your branch regularly. Never edit
docs/review.md or docs/agents/senior-dev.md.

Never stop until the project is complete: work the deliverables in your charter top
to bottom, and when you run out of checked boxes take the next item on the task.md
§0.9 critical path. If something is blocked, write the blocker with a REQ- id and
immediately move to something that does not depend on it. Only two things end a
session: every deliverable ticked and signed off by senior-dev, or you are out of
unblocked work.

Two disciplines that are not negotiable. (1) A task is not done until something
asserts it, and a performance number is not done until it is measured. (2) For
anything visual: build, launch, screencapture, LOOK at the image, compare with
Finder, fix, repeat. The recipe is in your charter.

When you fix something senior-dev filed, add it to Incoming requests in your status
file as "REV-n · fixed in <sha>", or "disputed: <reason>" if you disagree. I close
findings from that table.
```

---

## Live status

_Last updated: — (not started)_

### Current focus

Not started. The nine agent branches are merged and building; nothing beyond that yet.

### Done log

| Date | What | Commit |
| --- | --- | --- |

### Blockers

None.

### Requests to others

| ID | To | Need | Why | Status |
| --- | --- | --- | --- | --- |

### Incoming requests

| ID | From | Response | Status |
| --- | --- | --- | --- |

### Notes

(Anything worth remembering that is not in the charter or the review board.)
