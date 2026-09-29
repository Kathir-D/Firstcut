# Agent: infra

> **Charter.** Fixed; edit only with the owner's approval. The **Live status** section at the bottom is
> yours to update. Protocol: [task.md §0.4](../../task.md#04-protocol).

## Mission

Own the build system, the Rust↔Swift bridge, CI, releases, the Homebrew cask, and the README. Everyone else builds on what you set up, so wave 1 is the most time-critical for you.

## End goal (definition of done for this agent)

`scripts/build-app.sh` produces an ad-hoc signed `dist/Firstcut.app` that includes the Rust core; CI is green on every PR; pushing a `v*` tag publishes `Firstcut-<version>.zip` to GitHub Releases and updates `Casks/firstcut.rb` in `Kathir-D/homebrew-tap`; the README stays accurate.

**Done means (and senior-dev has signed it off in `docs/review.md`):** Clean clone → `scripts/build-app.sh` → working app with no manual steps; CI green; a test tag produces a release and a working `brew install --cask firstcut`.

## Owns (only you edit these)

`core/Cargo.toml` (workspace), `core/firstcut-core/src/lib.rs` + `ffi.rs` (module wiring and UniFFI exports), `project.yml`, `scripts/`, `.github/`, `Casks/`, `VERSION`, `README.md`, `LICENSE`, `.gitignore`, `.gitattributes`, `.editorconfig`, `App/Sources/Shared/` (CoreTypes stand-ins), `App/Generated/`

## Does NOT own

Any feature code. Other agents' exports in `ffi.rs` are added **at their request**; you wire them, and they own the implementation.

## task.md sections to read

§2 Decisions, §4 Architecture (layout), §13 Build, CI & distribution, §14 M0 and M8

## Contracts

- **Owns:** [build.md](../contracts/build.md)
- **Consumes:** Every contract (to export it through UniFFI)
- **Provides to others:** A working workspace and app skeleton, `FirstcutCore` Swift module, build scripts, CI, release pipeline

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

- [x] **Bootstrap (done before kickoff)**: `xcodegen` + `swift-format` installed; Cargo workspace with every agent's module declared; `project.yml` with app + Unit/Integration/Performance targets (macOS 15, arm64, Swift 6 strict concurrency); `App/Sources/Shared/CoreTypes.swift` stand-ins; placeholder tests; per-agent worktrees.
- [ ] **Wave 1**: UniFFI with a `hello()` export; `scripts/build-core.sh` (→ `FirstcutCore.xcframework` + Swift bindings in `App/Generated/`); link it in `project.yml`; `scripts/build-app.sh`; `VERSION`; the app calls `hello()`. Plan the switch from `CoreTypes.swift` to generated types (same names) and announce it in Notes for other agents before doing it. Freeze `build.md` at v1.0.
- [ ] **Wave 1**: CI workflow: `cargo fmt --check`, `clippy -D warnings`, `cargo test`, build-core, xcodegen, `xcodebuild build test`. Cache cargo + DerivedData.
- [ ] **Wave 2**: export the real `Session` API (session-api.md) and `PhotoMeta`/`Batch`/`VisualSig` types; pre-build phase re-runs build-core when Rust changes.
- [ ] **Wave 3**: release workflow on `v*` tags (build-app, zip, SHA-256, GitHub Release); `Casks/firstcut.rb` modeled on `sonar.rb` in the tap (postflight clears quarantine); workflow bumps the cask in `Kathir-D/homebrew-tap`.
- [ ] **Wave 4**: README screenshots + GIF (ask ui for captures), version bump in the curl example, issue templates, `THIRD-PARTY-NOTICES.md` if needed, GPL-3.0 compatibility check of every crate. Tag v0.1.0.

## Kickoff prompt

Paste this to start a session for this agent:

```
You are the "infra" agent for Firstcut (~/Documents/projects/Firstcut).
Read, in order: task.md §0 (team, protocol, schedule), docs/agents/infra.md (your charter + status),
docs/review.md (fix every open finding under "infra" and "All agents" first, highest severity first),
docs/contracts/build.md, the contracts listed under "Contracts" in your file, the task.md sections listed
in your file, and the "Requests to others" sections of every other file in docs/agents/.
Work only in your own worktree, ~/Documents/projects/Firstcut-wt/infra (branch agent/infra, already
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
