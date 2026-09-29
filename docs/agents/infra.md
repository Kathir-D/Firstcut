# Agent: infra

> **Charter.** Fixed; edit only with the owner's approval. The **Live status** section at the bottom is
> yours to update. Protocol: [docs/README.md](../README.md#protocol-every-agent-follows-this).

## Mission

Own the build system, the Rust↔Swift bridge, CI, releases, the Homebrew cask, and the README. Everyone else builds on what you set up, so wave 1 is the most time-critical for you.

## End goal (definition of done for this agent)

`scripts/build-app.sh` produces an ad-hoc signed `dist/Firstcut.app` that includes the Rust core; CI is green on every PR; pushing a `v*` tag publishes `Firstcut-<version>.zip` to GitHub Releases and updates `Casks/firstcut.rb` in `Kathir-D/homebrew-tap`; the README stays accurate.

**Done means:** Clean clone → `scripts/build-app.sh` → working app with no manual steps; CI green; a test tag produces a release and a working `brew install --cask firstcut`.

## Owns (only you edit these)

`core/Cargo.toml` (workspace), `core/firstcut-core/src/lib.rs` + `ffi.rs` (module wiring and UniFFI exports), `project.yml`, `scripts/`, `.github/`, `Casks/`, `VERSION`, `README.md`, `LICENSE`, `.gitignore`, `.gitattributes`, `.editorconfig`, `App/Resources/Info.plist`, `App/Generated/`

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
| infra | Build, UniFFI bridge, CI, releases, Homebrew, README | Exporting your types, build breaks, CI |
| core-meta | Metadata parsing for every format | `PhotoMeta` fields |
| core-batch | Ordering, batching, ground truth, CLI | `Batch`, `VisualSig`, fixtures |
| core-store | Session DB, XMP, undo, finish file ops | `Session` API |
| pipeline | Decode, cache, prefetch, viewer layer, zoom | `ImageProvider`, performance |
| app-logic | State model, commands, keymap, rules | `AppModel`, `Command` |
| ui | Every screen, Liquid Glass, Finder look | Layout, visuals |
| qa | Tests, perf baselines, bugs, sign-off | Test hooks, bug reports |

## Deliverables

- [ ] **Wave 1**: install `xcodegen` + `swift-format`; create the Cargo workspace (`firstcut-core`, `firstcut-cli`) with empty modules for every agent (`scan/ meta/ formats/ order/ batch/ store/ xmp/ fileops/ session.rs`); set up UniFFI with a `hello()` export; write `project.yml` (App target, Unit/Integration/Performance test targets, macOS 15 deployment target, arm64 only, Swift 6 strict concurrency); `scripts/build-core.sh` (→ `FirstcutCore.xcframework` + Swift bindings in `App/Generated/`); `scripts/build-app.sh`; `VERSION`; a minimal app that calls `hello()`. Freeze `build.md` at v1.0.
- [ ] **Wave 1**: CI workflow: `cargo fmt --check`, `clippy -D warnings`, `cargo test`, build-core, xcodegen, `xcodebuild build test`. Cache cargo + DerivedData.
- [ ] **Wave 1**: each agent's worktree set up per build.md, or instructions verified to work.
- [ ] **Wave 2**: export the real `Session` API (session-api.md) and `PhotoMeta`/`Batch`/`VisualSig` types; pre-build phase re-runs build-core when Rust changes.
- [ ] **Wave 3**: release workflow on `v*` tags (build-app, zip, SHA-256, GitHub Release); `Casks/firstcut.rb` modeled on `sonar.rb` in the tap (postflight clears quarantine); workflow bumps the cask in `Kathir-D/homebrew-tap`.
- [ ] **Wave 4**: README screenshots + GIF (ask ui for captures), version bump in the curl example, issue templates, `THIRD-PARTY-NOTICES.md` if needed, GPL-3.0 compatibility check of every crate. Tag v0.1.0.

## Kickoff prompt

Paste this to start a session for this agent:

```
You are the "infra" agent for Firstcut (~/Documents/projects/Firstcut).
Read, in order: docs/README.md, docs/agents/infra.md (your charter + status), docs/contracts/build.md,
the contracts listed under "Contracts" in your file, the task.md sections listed in your file, and the
"Requests to others" sections of every other file in docs/agents/.
Work only in your own worktree (../Firstcut-wt/infra, branch agent/infra) and only on the paths you own.
Pick the next unchecked deliverable, do it, then update your Live status, tick task.md boxes you own,
commit, and push. Ask other agents for anything you need through requests, never by editing their files.
```

---

## Live status

_Last updated: — (not started)_

### Current focus

Not started. Waiting for the owner's go-ahead.

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

### Notes for other agents

(Anything others should know: gotchas, measurements, decisions made inside your area.)
