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
│   │   ├── Session/          # app-logic: AppModel, navigation, rating rules, undo bridging;
│   │   │                     #   CoreSessionBackend, UniFFICoreSession, CoreFirstPhoto are conversion layer
│   │   ├── Input/            # app-logic: Command, keymap, key routing
│   │   ├── Pipeline/         # pipeline: decoders, CacheManager, scheduler, ThumbnailStore, memory budget
│   │   ├── Render/           # pipeline: IOSurface/Metal viewer layer, zoom/pan, overlays
│   │   └── Shared/           # infra: CoreTypes.swift stand-ins until UniFFI bindings replace them;
│   │                         #   CoreTypeMapping.swift and CoreBridge.swift are conversion layer
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
├── todo.md                   # everyone may tick their own boxes
└── docs/                     # see todo.md §0.8
```

## Names

| Thing | Name |
| --- | --- |
| Rust crate (library) | `firstcut-core` → `firstcut_core` |
| Rust CLI binary | `firstcut` (crate `firstcut-cli`) |
| Swift module for the Rust core | `FirstcutCore` (UniFFI). Generated into `App/Generated/`; the C module it imports is `FirstcutCoreFFI`. Its types carry an `Ffi` prefix (`FfiPhotoMeta`, `FfiRating`, `FfiTier`). App code uses the types in `App/Sources`, and reaches the core through the conversion layer (below), never by importing `FirstcutCore` itself |
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
- **Calling it from Swift**: `import FirstcutCore` is confined to the **conversion layer**, which is
  these five files and no others:

  | File | Its job |
  | --- | --- |
  | `Shared/CoreTypeMapping.swift` | the type-by-type conversions between a generated `Ffi*` type and the app's own |
  | `Session/CoreSessionBackend.swift` | owns the session object and drives it |
  | `Session/UniFFICoreSession.swift` | the session's app-facing surface; also where the Finish conversions live |
  | `Session/CoreFirstPhoto.swift` | wraps the two first-photo exports into one call |
  | `Shared/CoreBridge.swift` | the top-level core functions, namespaced as `FirstcutCoreBridge` (version, greeting, capability checks, visual signature) |

  No view, model or view-state file may import it. Everywhere else calls the conversion layer, so
  the generated globals never land in app code.
- **The generated types keep their `Ffi` prefix** (`FfiPhotoMeta`, `FfiRating`, `FfiTier`) — that is
  the shipped design, not a temporary wart, and the conversion layer is the boundary between the two
  vocabularies. Conversions live in `CoreTypeMapping.swift`, type by type; the Finish ones live in
  `UniFFICoreSession.swift` instead, because they are not mechanical. No `Ffi*` name appears anywhere
  in `App/Sources` or `App/Tests` outside those five files.
  An earlier draft of this contract promised that the generated types would later take the plain
  names, which would have made every conversion deletable. That swap did not happen; the conversions
  are permanent, so write app code against the app types.
- **Errors**: `#[derive(uniffi::Error)]` enums, never `String`, so Swift can switch on them.
- **Staleness**: the app target's pre-build phase runs `scripts/build-core-if-stale.sh`, which
  rebuilds the core only when a Rust file is newer than the last successful build. After pulling,
  `scripts/generate-project.sh` does the same plus `xcodegen`.

## Build & test commands

Every command below is worth running through `scripts/with-timeout.sh <seconds> …` (exit 124 means
it was killed). A build or test run on this machine can otherwise block forever rather than fail —
see **Nothing in a test run may touch `~/Documents`** below.

| What | Command |
| --- | --- |
| Rust tests | `cargo test --manifest-path core/Cargo.toml` |
| Rust lint | `cargo fmt --check` and `cargo clippy -- -D warnings` |
| Build the Rust core + bindings | `scripts/build-core.sh` → `App/Generated/` |
| Get a worktree ready after a pull | `scripts/generate-project.sh` |
| Generate the Xcode project | `xcodegen` (run after pulling; `Firstcut.xcodeproj` is git-ignored) |
| Build the app | `scripts/build-app.sh` → `dist/Firstcut.app` (`--open` to launch it) |
| Swift tests | `xcodebuild -project Firstcut.xcodeproj -scheme Firstcut -destination 'platform=macOS,arch=arm64' test` |
| Run with a deadline | `scripts/with-timeout.sh 600 xcodebuild … test` |
| Everything CI runs | see the header of [.github/workflows/ci.yml](../../.github/workflows/ci.yml) |

## Nothing in a test run may touch `~/Documents`

The repository is under `~/Documents`. A test host is a GUI app, and the app is ad-hoc signed, so it
has a **new identity on every rebuild** — macOS therefore re-asks for Documents access every time and
blocks the run forever waiting for a click that nobody is there to give. A run that does this does
not fail, it just never finishes.

So the rules are:

- Everything a test reads is **bundled into the test target** (`project.yml`), and read from the
  app's own container. The fixtures land as flat `exiftool/` and `meta/` folders, because a folder
  reference keeps its own name; a lookup for `tests/fixtures/exiftool` will not find them.
- `build-core.sh` also writes `App/Generated/FirstcutCore.bindings.txt`, which is what the tests that
  assert on the export list read. Xcode will not copy a `.swift` file through Copy Bundle Resources.
- The **test photos** are read only when `FIRSTCUT_ALLOW_PHOTO_TESTS=1` is set. Not "the tests are
  skipped" — the lookup itself does nothing, so an innocent test cannot trigger it.
- `FixturePhotos`' repository fallback is disabled in any test process. It still exists for previews
  run from a checkout, which are not a test host.

`App/Tests/Integration/FixtureHarnessTests` has three tests that assert this, and they fail if a
lookup starts reaching a protected path again.

## CI

`.github/workflows/ci.yml` runs on every push and PR: `cargo fmt --check`, `cargo clippy -D
warnings`, `cargo test`, a release build of the core, then `scripts/build-core.sh`, `xcodegen`,
`xcodebuild build` and `xcodebuild test` on an arm64 `macos-15` runner. Cargo and DerivedData are
cached. The release workflow (`.github/workflows/release.yml`) runs on `v*` tags.

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
- v0.1.1: added **Nothing in a test run may touch `~/Documents`** (why the ad-hoc rebuild re-asks for
  Documents access and hangs the run, and the four lookups that were doing it), `scripts/with-timeout.sh`
  and a line in the commands table for it. `build-core.sh` now also emits
  `App/Generated/FirstcutCore.bindings.txt`, and `project.yml` bundles it into the test targets.
  Removed the duplicated **Build & test commands** and **CI** sections.
- v0.1.2: corrected **The Rust ↔ Swift bridge (UniFFI)**. `import FirstcutCore` is confined to the
  conversion layer and the five files that make it up are now named (it is no longer `CoreBridge.swift`
  alone), the generated types carry the `Ffi` prefix — the same-name swap this contract once promised
  never happened, so the conversions are permanent — and `session-api.md` records which session types
  are genuine app types that must not be aliased away, with the reason each one stays.
