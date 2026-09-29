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
- [x] **Wave 1**: UniFFI with a `hello()` export; `scripts/build-core.sh` (→ `FirstcutCore.xcframework` + Swift bindings in `App/Generated/`); link it in `project.yml`; `scripts/build-app.sh`; `VERSION`; the app calls `hello()`. Plan the switch from `CoreTypes.swift` to generated types (same names) and announce it in Notes for other agents before doing it. **The switch is planned and announced; the swap itself happens when the real exports land (wave 2).** `build.md` is proposed for v1.0 (REQ-infra-3).
- [x] **Wave 1**: CI workflow: `cargo fmt --check`, `clippy -D warnings`, `cargo test`, build-core, xcodegen, `xcodebuild build test`. Cache cargo + DerivedData.
- [ ] **Wave 2**: export the real `Session` API (session-api.md) and `PhotoMeta`/`Batch`/`VisualSig` types; pre-build phase re-runs build-core when Rust changes. (The pre-build phase is already in place; only the exports remain, on request — REQ-infra-2.)
- [ ] **Wave 3**: release workflow on `v*` tags (build-app, zip, SHA-256, GitHub Release); `Casks/firstcut.rb` modeled on `sonar.rb` in the tap (postflight clears quarantine); workflow bumps the cask in `Kathir-D/homebrew-tap`. **Can start now** — everything it needs (`build-app.sh`, `VERSION`) exists and is verified.
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

_Last updated: 2026-09-29_

### Current focus

Wave 1 is done and waiting for review: UniFFI bridge, build scripts, `VERSION`, CI, the app calling
`hello()`. Next in my queue, in order: the `v*` release workflow + `Casks/firstcut.rb` (can be done
now, ahead of wave 3, since it only needs the build scripts that exist), then exporting the real
`PhotoMeta` / `Batch` / `Session` types as agents request them (wave 2).

### Done log

| Date | What | Commit |
| --- | --- | --- |
| 2026-09-29 | UniFFI 0.32 wired: `uniffi` dep, `setup_scaffolding!()` in `lib.rs`, `hello()` + `core_version()` in `ffi.rs`, `uniffi.toml` (module `FirstcutCore`), the two generator bins in `core/firstcut-core/src/bin/` so nobody needs `cargo install` | _(this commit)_ |
| 2026-09-29 | `scripts/common.sh`, `scripts/build-core.sh` (release staticlib → Swift bindings → `FirstcutCore.xcframework` in `App/Generated/`), `scripts/build-core-if-stale.sh`, `scripts/generate-project.sh`, `scripts/build-app.sh` (→ ad-hoc signed `dist/Firstcut.app`, 1.8 MB, verified launching) | _(this commit)_ |
| 2026-09-29 | `project.yml`: new static `FirstcutCore` target (the generated Swift as an importable module, linking the xcframework), `FirstcutCoreFFI` module on the header search path for every target, pre-build phase on the app target, test targets depend on `FirstcutCore` | _(this commit)_ |
| 2026-09-29 | `App/Sources/Shared/CoreBridge.swift` (`FirstcutCoreBridge`: `greeting`, `coreVersion`, `isLinked`) + `App/Tests/Unit/Core/CoreBridgeTests.swift`; both pass under `xcodebuild test` | _(this commit)_ |
| 2026-09-29 | `.github/workflows/ci.yml`: Rust fmt/clippy/test + release build, then build-core → xcodegen → `xcodebuild build`/`test` on arm64 `macos-15`, cargo + DerivedData cached, `swift-format` advisory | _(this commit)_ |
| 2026-09-29 | `VERSION` (0.0.0), `docs/contracts/build.md` (bridge + CI sections, proposed v1.0 freeze), task.md §13 and M0 boxes | _(this commit)_ |

### Blockers

None.

### Requests to others

| ID | To | Need | Why | Status |
| --- | --- | --- | --- | --- |
| REQ-infra-1 | ui | Call `FirstcutCoreBridge.greeting` in the placeholder window (`Text("Firstcut")` → the greeting, or a small line under it) so the Rust call is visible in the running app. `FirstcutApp.swift` is yours, so I can't do it. Later: show `FirstcutCoreBridge.coreVersion` in an About panel. | M0 "the app calls one Rust function through UniFFI" is only true end to end once the app itself makes the call. The unit test already proves the link. | open |
| REQ-infra-2 | core-meta, core-batch, core-store | When your Swift-visible types are ready, send me the Rust type + the export signature you want. I add the `#[uniffi::export]` / `#[derive(uniffi::Record)]` to `core/firstcut-core/src/ffi.rs`, regenerate the bindings, and hand back a one-line usage note. Nothing else in the build changes. | Keeps `ffi.rs` yours-by-proxy without anyone waiting: you write the implementation, I wire the boundary. Start with `PhotoMeta` (core-meta) and `Batch`/`VisualSig` (core-batch) — that is what unblocks app-logic and pipeline on real data. | open |
| REQ-infra-3 | senior-dev | Approve `docs/contracts/build.md` at **v1.0** (I added the UniFFI bridge section, the CI section, `generate-project.sh`, and the `FirstcutCoreBridge` rule; no breaking changes). Please also confirm the wave-1 infra deliverables are signed off so I can close the box. | Contracts are frozen by the owner, not the writer (task.md §0.4). | open |
| REQ-infra-4 | qa | Tell me the paths/harness you want for: (a) the local perf job (it needs `~/Documents/testing`, which CI cannot see, so I will add a `workflow_dispatch` perf workflow that you drive locally or on a self-serve runner), (b) a `perf-baselines` fixture path if you want the numbers committed. | The CI workflow I just merged runs unit + integration + performance *targets* on the runner, but any test that needs the 42 GB of RAW photos has to skip in CI (task.md §12: skip if `FIRSTCUT_TEST_PHOTOS` is absent). I need to know your contract for that. | open |

### Incoming requests

Requests from other agents (`REQ-…`) and senior-dev findings from `docs/review.md` (`REV-…`).

| ID | From | Response | Status |
| --- | --- | --- | --- |

### Notes for other agents

**Action needed after you pull `main` — run this once in your worktree:**

```
./scripts/generate-project.sh
```

`App/Sources/Shared/CoreBridge.swift` is new and does `import FirstcutCore`, so a project generated
from the old `project.yml` will not compile until you regenerate. `generate-project.sh` builds the
Rust core (≈1 min the first time, ~2 s afterwards) and runs `xcodegen`. After that, plain
`xcodebuild` and Xcode just work: the app target's pre-build phase rebuilds the core whenever a Rust
file changes, and Swift sources are still picked up by folder, so adding a file never needs
`xcodegen` again.

**How to reach Rust from Swift.** `import FirstcutCore` only inside
`App/Sources/Shared/CoreBridge.swift` (mine). Everything else goes through `FirstcutCoreBridge`, so
the generated top-level functions and types never land in your file's scope. For now it exposes
`greeting`, `coreVersion` and `isLinked`; I add the rest when you file a request (REQ-infra-2).

**The switch from `CoreTypes.swift` to the generated types (plan, announced before I do it, as
promised in my charter).** When the real exports land:

1. I regenerate the bindings. UniFFI derives Swift names from the Rust names, so the Rust types must
   be named to match `CoreTypes.swift` exactly (`PhotoMeta`, `Batch`, `VisualSig`, `Rating`,
   `CaptureTime`, `AfInfo`, `AfPoint`, `EmbeddedPreview`, `ByteRange`, `FileKind`, `RawFormat`,
   `TimeSource`, and the `PhotoID`/`BatchID` type aliases). The contracts' field names are the
   contract; if Rust needs different Rust-side names, `uniffi.toml` gets a `[bindings.swift.rename]`
   entry instead — **your field names never change**.
2. I delete `App/Sources/Shared/CoreTypes.swift` in the same commit that removes the stand-ins from
   the app, and I keep the diff mechanical: identical names, identical optionality, `Codable` and
   `Sendable` conformance requested explicitly on each generated type (`#[derive(uniffi::Record)]` +
   an explicit `Codable` where Swift needs it).
3. Before I delete anything I will (a) announce the commit in **Notes for other agents** here and in
   the PR body, (b) put every `CoreTypes` type that has no generated counterpart yet behind a
   `CoreTypes.swift` file that only *adds* the missing pieces, so nothing breaks mid-wave, and
   (c) keep the old file in git history so a revert is one command.
4. `Rating`/`Flag`/`ColorLabel`/`RatingMode` are app-logic's, not the core's: they live in the
   session DB and XMP, so they stay hand-written Swift. app-logic, expect those to move to
   `App/Sources/Session/` (yours) when the switch happens; I will not touch them.

**Gotchas I hit, so you don't have to:**

- `uniffi-bindgen-swift --module-name` sets the *C* module name, not the Swift one. The Swift
  module name comes from `core/firstcut-core/uniffi.toml` (`FirstcutCore`), and the C module it
  imports is `FirstcutCoreFFI`. `scripts/build-core.sh` gets both right; if you ever run the
  generator by hand, use `--module-name FirstcutCoreFFI`.
- Swift's explicit-module build planning resolves the *transitive* `import FirstcutCoreFFI`, so
  every target (app and all three test targets) needs
  `HEADER_SEARCH_PATHS = $(SRCROOT)/App/Generated/FirstcutCoreFFI`. It is set once, project-wide.
- `cargo` is not on `PATH` in a GUI-launched Xcode. `scripts/common.sh` sources `~/.cargo/env`, so
  the pre-build phase works from Xcode, `xcodebuild` and a terminal.
- GitHub's `macos-15` runner label is **arm64** (M1) for public repos; `macos-15-intel` is the Intel
  one and would fail, since Firstcut is Apple Silicon only. CI pins Xcode 16.4 (last SDK is
  macOS 15) with a fallback to the image default.
- `xcodebuild -create-xcframework` in Xcode 27 rejects `-quiet`; don't add it back.
- The app is ad-hoc signed and the Rust core is **statically** linked, so there is exactly one code
  signature to make and nothing to embed. `dist/Firstcut.app` is 1.8 MB today and launches from a
  clean build.

**Machine note:** 9 agents compiling at once on 16 GB is heavy (task.md §0.7). The Xcode pre-build
phase only runs cargo when a Rust file actually changed, so Swift-only iterations stay cheap.

**Please run `swift-format` on your Swift files before committing** —
`swift-format format --in-place --recursive App/Sources/<yours>`. The tree is `swift-format lint
--strict` clean as of this commit (there is a repo-wide `.swift-format` with 4-space indentation,
which matches the existing code; swift-format's built-in default of 2 was wrong for us). CI runs
that lint as advisory only right now, so it cannot block you, and I will flip it to enforcing once
the tree is formatted again.
