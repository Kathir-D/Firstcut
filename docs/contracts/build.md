# Contract: build, layout & git workflow

- **Owner:** infra
- **Consumers:** everyone
- **Version:** v0.1 (draft; frozen as v1.0 at the end of wave 1)

## Repository layout

```
Firstcut/
├── App/
│   ├── Sources/
│   │   ├── App/              # ui: @main, AppDelegate, window, toolbar, menus
│   │   ├── Views/            # ui: Viewer, Filmstrip, InfoPanel, Grid, Compare, HUD, Finish sheet, Welcome
│   │   ├── Settings/Views/   # ui: Settings window
│   │   ├── Settings/Model/   # app-logic: Settings model and persistence
│   │   ├── Session/          # app-logic: AppModel, navigation, rating rules, undo bridging
│   │   ├── Input/            # app-logic: Command, keymap, key routing
│   │   ├── Pipeline/         # pipeline: decoders, CacheManager, scheduler, ThumbnailStore, memory budget
│   │   ├── Render/           # pipeline: IOSurface/Metal viewer layer, zoom/pan, overlays
│   │   └── Shared/           # infra: CoreTypes.swift stand-ins until UniFFI bindings replace them;
│   │                         #   CoreBridge.swift is the only place that imports FirstcutCore
│   ├── Resources/            # ui: Assets; app-logic: DefaultKeymap.json; infra: Info.plist
│   ├── Generated/            # infra: UniFFI Swift bindings (git-ignored, built)
│   └── Tests/
│       ├── Unit/<Area>/      # tests grouped by area
│       ├── Integration/      # qa
│       └── Performance/      # qa
├── core/
│   ├── Cargo.toml            # infra (workspace)
│   ├── firstcut-core/src/
│   │   ├── lib.rs            # infra (module wiring + UniFFI setup only)
│   │   ├── scan/ meta/ formats/   # core-meta
│   │   ├── order/ batch/          # core-batch
│   │   ├── store/ xmp/ fileops/ session.rs   # core-store
│   │   ├── ffi.rs            # infra (exports; each owner requests its additions)
│   │   └── bin/              # infra (uniffi-bindgen, uniffi-bindgen-swift; no cargo install needed)
│   ├── firstcut-core/uniffi.toml  # infra (Swift module name)
│   └── firstcut-cli/         # core-batch (other agents add subcommands by request)
├── scripts/                  # infra
├── tests/fixtures/           # core-batch (ground truth, metadata dumps); core-meta (header byte fixtures in fixtures/headers/)
├── Casks/  .github/  project.yml  VERSION  README.md  LICENSE   # infra
├── task.md                   # everyone may tick their own boxes
└── docs/                     # see task.md §0.8
```

## Names

| Thing | Name |
| --- | --- |
| Rust crate (library) | `firstcut-core` → `firstcut_core` |
| Rust CLI binary | `firstcut` (crate `firstcut-cli`) |
| Swift module for the Rust core | `FirstcutCore` (UniFFI). Generated into `App/Generated/`; the C module it imports is `FirstcutCoreFFI`. Swift code reaches it through `FirstcutCoreBridge` in `App/Sources/Shared/CoreBridge.swift` |
| App target / scheme / bundle | `Firstcut` / `Firstcut` / `com.kathird.firstcut` |
| Test photos env var | `FIRSTCUT_TEST_PHOTOS` (default `~/Documents/testing`) |

## The Rust ↔ Swift bridge (UniFFI)

```
core/firstcut-core/src/ffi.rs      every #[uniffi::export]; infra wires them, the owning agent writes them
core/firstcut-core/src/lib.rs      uniffi::setup_scaffolding!() at the crate root
core/firstcut-core/uniffi.toml     module_name = "FirstcutCore"
                                   ↓ scripts/build-core.sh
App/Generated/FirstcutCore.swift         compiled into the FirstcutCore target (a static framework)
App/Generated/FirstcutCoreFFI/           FirstcutCoreFFI.h + module.modulemap; on HEADER_SEARCH_PATHS
App/Generated/FirstcutCore.xcframework   libfirstcut_core.a, linked, never embedded
```

- **Adding an export**: the owning agent writes the function in their own module and asks infra
  (`REQ-infra-n`) to add the `#[uniffi::export]` to `ffi.rs`. Nothing else in the build changes.
- **Calling it from Swift**: `import FirstcutCore` is only done in `CoreBridge.swift`. Everywhere
  else goes through `FirstcutCoreBridge`, so the generated globals never land in app code.
- **Errors**: `#[derive(uniffi::Error)]` enums, never `String`, so Swift can switch on them.
- **Staleness**: the app target's pre-build phase runs `scripts/build-core-if-stale.sh`, which
  rebuilds the core only when a Rust file is newer than the last successful build. After pulling,
  `scripts/generate-project.sh` does the same plus `xcodegen`.

## Build & test commands

| What | Command |
| --- | --- |
| Rust tests | `cargo test --manifest-path core/Cargo.toml` |
| Rust lint | `cargo fmt --check` and `cargo clippy -- -D warnings` |
| Build the Rust core + bindings | `scripts/build-core.sh` → `App/Generated/` |
| Get a worktree ready after a pull | `scripts/generate-project.sh` |
| Generate the Xcode project | `xcodegen` (run after pulling; `Firstcut.xcodeproj` is git-ignored) |
| Build the app | `scripts/build-app.sh` → `dist/Firstcut.app` (`--open` to launch it) |
| Swift tests | `xcodebuild -project Firstcut.xcodeproj -scheme Firstcut -destination 'platform=macOS,arch=arm64' test` |
| Everything CI runs | see the header of [.github/workflows/ci.yml](../../.github/workflows/ci.yml) |

## CI

`.github/workflows/ci.yml` runs on every push and PR: `cargo fmt --check`, `cargo clippy -D
warnings`, `cargo test`, a release build of the core, then `scripts/build-core.sh`, `xcodegen`,
`xcodebuild build` and `xcodebuild test` on an arm64 `macos-15` runner. Cargo and DerivedData are
cached. The release workflow (`.github/workflows/release.yml`) runs on `v*` tags.

Before CI existed, the local rule was to run the same checks yourself; that is no longer needed,
but it is still the fastest way to catch a break.


## Day-one bootstrap (already in the repo)

| Thing | Where | Owner |
| --- | --- | --- |
| Rust workspace, every module declared in `lib.rs` | `core/` | infra (each module: its agent) |
| XcodeGen project, app + 3 test targets | `project.yml` | infra |
| Placeholder app entry point | `App/Sources/App/FirstcutApp.swift` | ui (replace it) |
| Swift stand-ins for core types | `App/Sources/Shared/CoreTypes.swift` | infra (request changes; deleted when UniFFI lands) |
| Placeholder tests | `App/Tests/*/…PlaceholderTests.swift` | the owning agent may delete them |
| exiftool dumps of the test games | `tests/fixtures/exiftool/<game>.json` | core-batch (read-only for others) |

- Sources are included by **folder**, so adding Swift files never requires editing `project.yml`.
- **Dependency exception**: any agent may add its own crates to `[dependencies]` in
  `core/firstcut-core/Cargo.toml` (one line each, alphabetical). Swift packages: request them from infra.
- `Cargo.lock` conflicts: take `main`'s version, then `cargo build`.

## Build & test commands

| What | Command |
| --- | --- |
| Rust tests | `cargo test --manifest-path core/Cargo.toml` |
| Rust lint | `cargo fmt --check` and `cargo clippy -- -D warnings` |
| Build the Rust core + bindings | `scripts/build-core.sh` → `App/Generated/` |
| Get a worktree ready after a pull | `scripts/generate-project.sh` |
| Generate the Xcode project | `xcodegen` (run after pulling; `Firstcut.xcodeproj` is git-ignored) |
| Build the app | `scripts/build-app.sh` → `dist/Firstcut.app` (`--open` to launch it) |
| Swift tests | `xcodebuild -project Firstcut.xcodeproj -scheme Firstcut -destination 'platform=macOS,arch=arm64' test` |
| Everything CI runs | see the header of [.github/workflows/ci.yml](../../.github/workflows/ci.yml) |

## CI

`.github/workflows/ci.yml` runs on every push and PR: `cargo fmt --check`, `cargo clippy -D
warnings`, `cargo test`, a release build of the core, then `scripts/build-core.sh`, `xcodegen`,
`xcodebuild build` and `xcodebuild test` on an arm64 `macos-15` runner. Cargo and DerivedData are
cached. The release workflow (`.github/workflows/release.yml`) runs on `v*` tags.

Before CI existed, the rule was to run the same checks locally before merging; that is no longer
required, but it is still the fastest way to catch a break.

## Git workflow

- **One agent, work on `main`.** `git pull` before starting, commit small, **push right after every
  commit**. Short branches are fine; merge them straight into `main` (`gh pr create` + merge, or a local
  merge). **Never force-push, never rebase `main`.**
- **Checks before pushing** (CI runs the same): `cargo fmt --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test` (in `core/`), then `scripts/generate-project.sh` and the Swift test command above.
- **Keep `main` green**: if a push breaks it, fix it or `git revert` right away (never a force-push).
- Commit messages: say what changed and why, in plain prose (for example `Batching: never join a pair
  whose time runs backwards`).

## Changelog

- v0.1: initial draft.
- v0.1 (wave 1, infra): added **The Rust ↔ Swift bridge (UniFFI)** section (layout, how to add an
  export, how to call one from Swift), a **CI** section, `scripts/generate-project.sh`,
  `scripts/build-core.sh` output paths, and `CoreBridge.swift` as the single import site for
  `FirstcutCore`. `CoreTypes.swift` is unchanged. No breaking changes: Swift sources are still
  picked up by folder, and `App/Generated/` is optional in `project.yml` so a clean clone can run
  `xcodegen` before the first core build. **Proposed freeze at v1.0** — pending sign-off (see the old infra charter, `git show 4d4e43d:docs/agents/archive/infra.md`).
