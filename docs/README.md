# Firstcut multi-agent workspace

Firstcut is built by **8 agents working in parallel**. Each one owns a distinct part of the code with a
distinct end goal. They coordinate only through the Markdown files in this folder.

- [`../task.md`](../task.md) is the product plan: what to build and why.
- `docs/` is the coordination layer: who builds what, the interfaces between them, and live status.

> **Coding has not started.** Agents don't begin until the owner gives the go-ahead. Wave 1 in the
> [schedule](#schedule) is the starting point.

## Roster

| Agent | One-line mission | End goal (done when…) | Owns | Status file |
| --- | --- | --- | --- | --- |
| **infra** | Build system, the Rust↔Swift bridge, CI, releases, Homebrew, README | `scripts/build-app.sh` produces a signed `dist/Firstcut.app` that includes the Rust core; CI is green; a tagged release publishes the zip and updates the Homebrew cask | `core/Cargo.toml` (workspace), `project.yml`, `scripts/`, `.github/`, `Casks/`, `VERSION`, `README.md`, `LICENSE`, `.gitignore` | [agents/infra.md](agents/infra.md) |
| **core-meta** | Read every file's metadata fast and correctly | `scan_folder()` returns a complete `PhotoMeta` for every file in all four test games in < 3 s, matching exiftool on every field, and handles every format in task.md §8 | `core/firstcut-core/src/scan/`, `src/meta/`, `src/formats/` | [agents/core-meta.md](agents/core-meta.md) |
| **core-batch** | Turn photos into correct bursts | `order()` + `batch()` reach ≥ 98% boundary F1 against visually verified ground truth for all four games; deterministic; < 2 s for 1,500 files | `core/firstcut-core/src/order/`, `src/batch/`, `core/firstcut-cli/`, `tests/fixtures/` | [agents/core-batch.md](agents/core-batch.md) |
| **core-store** | Persist everything safely | The session DB, XMP sidecars, undo/redo log, and Finish Cull file operations all work, survive crashes, and never touch originals | `core/firstcut-core/src/store/`, `src/xmp/`, `src/fileops/`, `src/session.rs` | [agents/core-store.md](agents/core-store.md) |
| **pipeline** | Zero-wait image decoding and display | Holding → through a whole game never misses the cache in the previous/current/next batch; every §7.3 target is met and measured | `App/Sources/Pipeline/`, `App/Sources/Render/` | [agents/pipeline.md](agents/pipeline.md) |
| **app-logic** | The app's brain: state, commands, keymap, rating rules | Every command in the command list works end to end through the state model, with undo, both rating modes, auto-advance, and remappable keys, all covered by Swift tests | `App/Sources/Session/`, `App/Sources/Input/`, `App/Sources/Settings/Model/`, `App/Resources/DefaultKeymap.json` | [agents/app-logic.md](agents/app-logic.md) |
| **ui** | Make it look and feel like Finder with Liquid Glass | Every screen in task.md §9 exists, is driven by the app-logic model, and a side-by-side check against Finder on macOS 26 and 15 passes | `App/Sources/App/`, `App/Sources/Views/`, `App/Sources/Settings/Views/`, `App/Resources/Assets.xcassets`, `docs/ui/` | [agents/ui.md](agents/ui.md) |
| **qa** | Independent proof that it works | Test harnesses, performance baselines, the stress test and the manual QA checklist all exist and pass for all four games in both rating modes; every bug is filed and closed | `App/Tests/Integration/`, `App/Tests/Performance/`, `docs/qa/` | [agents/qa.md](agents/qa.md) |

## How the agents fit together

```
              core-meta ──PhotoMeta──▶ core-batch ──Batch──┐
                  │                                        ▼
                  └──────PhotoMeta──────────────────▶ core-store (Session API)
                                                           │  UniFFI (built by infra)
                                                           ▼
 pipeline ◀──focus / viewport── app-logic ◀──────── Session API
    │  VisualSig (thumbnail hashes) ──▶ core-batch (through the Session API)
    └──images──▶ ui ◀──state + commands── app-logic

 qa: tests everyone, independently.     infra: builds and ships everyone.
```

## Contracts

A contract is the agreed interface between two or more agents. Each one has exactly **one owner**, who
is the only agent allowed to edit it. Consumers build against the contract, using a mock until the real
implementation lands, so nobody waits on anyone else.

| Contract | Owner | Consumers |
| --- | --- | --- |
| [contracts/build.md](contracts/build.md): repo layout, module names, how to build/test, git workflow | infra | everyone |
| [contracts/photo-meta.md](contracts/photo-meta.md): `PhotoMeta`, `scan_folder()` | core-meta | core-batch, core-store, pipeline, app-logic, ui |
| [contracts/batching.md](contracts/batching.md): `order()`, `batch()`, `Batch`, `VisualSig` algorithm | core-batch | core-store, pipeline, app-logic |
| [contracts/session-api.md](contracts/session-api.md): the Swift-visible `Session` API (ratings, undo, cursor, finish) | core-store | app-logic, qa |
| [contracts/pipeline-api.md](contracts/pipeline-api.md): `ImageProvider`, focus/prefetch, overlays, stats | pipeline | app-logic, ui, qa |
| [contracts/app-model.md](contracts/app-model.md): `AppModel` state, `Command` list, default keymap | app-logic | ui, qa |

## Protocol (every agent follows this)

### 1. Start of every work session

1. Read [`../task.md`](../task.md) (the sections listed in your agent file), this README, and
   [contracts/build.md](contracts/build.md).
2. Read **every** file in `docs/agents/`, especially the **Requests to others** sections, looking for
   anything addressed to you.
3. Read the contracts you consume and check each one's changelog for changes since you last looked.

### 2. What you may edit

- ✅ Your own status file: `docs/agents/<you>.md`, **Live status** section only. The charter above it is
  changed only by the owner (the human) or by agreement recorded in both agents' files.
- ✅ Contracts you own.
- ✅ Code and test paths you own (the roster's **Owns** column).
- ✅ `task.md`: only to tick checkboxes for work you own.
- ❌ Anything else. If you need a change in someone else's file, code, or contract, **make a request**.

### 3. Requests between agents

- To ask another agent for something, add a row under **Requests to others** in *your own* file:
  `REQ-<you>-<n>` · to `<agent>` · what you need · why · status `open`.
- The receiving agent copies the ID into its **Incoming requests** table with its response
  (`accepted` / `done in <commit>` / `declined: reason`).
- The requester marks it `closed` once satisfied.
- If you're blocked, say so in **Blockers** with the request ID, then work on something else.
  Code against the contract with a mock rather than waiting.

### 4. Contract changes

- **Additive** changes (new field or function): the owner edits the contract, adds a changelog line,
  and ships.
- **Breaking** changes (rename, remove, change of meaning): the owner proposes the change in the
  contract under **Proposed changes**, and every consumer acknowledges it in their own file. Then the
  owner ships it and updates the changelog.
- Contract versions use `vMAJOR.MINOR` at the top of the file.

### 5. End of every work session (or every meaningful step)

- Update your **Live status**: current focus, a done-log entry with the commit hash, blockers, and any
  requests.
- Tick your finished boxes in `task.md`.
- Commit and push (see [contracts/build.md](contracts/build.md) for the git workflow).

### 6. Definition of done for any task

- Code plus tests merged to `main`, CI green, measurements recorded where task.md asks for them, the
  status file updated, and the relevant contract updated if its surface changed.
- Performance claims need a measurement, not an estimate.

## Schedule

The waves are built so that **every agent has real work from day one**, using mocks where needed.

| Wave | Goal | infra | core-meta | core-batch | core-store | pipeline | app-logic | ui | qa |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **1. Foundations** | Everything compiles; the contracts are frozen at v1.0 | Workspace, UniFFI "hello", XcodeGen, scripts, CI | CR3 parser + `scan_folder` | exiftool-JSON adapter, `order()`, first `batch()`, contact sheets | DB schema, XMP read/write | Standalone ImageIO decode + cache spike, benchmarks | `AppModel` + `Command` list against mock data | Window, toolbar, viewer, filmstrip with mock data | Test harness, fixture loader, perf baseline format |
| **2. Real data** | The CLI and a minimal app run on the real test games | Link the real core into the app | Canon fields verified against exiftool | Ground truth for all four games, F1 ≥ 98% | Session API, undo log, resume | Scheduler, memory budget, IOSurface renderer | Wire to the real Session + pipeline | Rating visuals, keep rings, HUD, batch navigation | Stress test, first perf run |
| **3. Features** | Feature complete | Release pipeline, Homebrew cask | Sony, then all other formats | Two-phase refinement with visual hashes | Finish Cull planner + executor + undo | Zoom (pinch/click), zoom lock, T3/T4, AF and clipping overlays | Finish flow logic, settings model, keymap editor model | Info panel, grid, compare, Finish sheet, Settings, welcome window | Manual QA checklist, both modes, all games |
| **4. Polish & ship** | v0.1.0 | README screenshots, release | Hardening | Tuning | Crash-safety tests | Performance tuning | Edge cases | Liquid Glass pass, macOS 15 fallback, accessibility, icon | Final sign-off |

## Folder layout

```
docs/
├── README.md           ← you are here (roster, protocol, schedule)
├── agents/<agent>.md   ← charter (fixed) + live status (the agent's own log)
├── contracts/*.md      ← interfaces between agents, one owner each
├── qa/                 ← qa-owned: perf baselines, QA checklist, bug list
└── ui/                 ← ui-owned: Finder reference screenshots, visual check notes
```
