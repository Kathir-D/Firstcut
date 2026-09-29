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
│   │   └── Shared/           # infra: CoreTypes.swift stand-ins until UniFFI bindings replace them
│   ├── Resources/            # ui: Assets; app-logic: DefaultKeymap.json; infra: Info.plist
│   ├── Generated/            # infra: UniFFI Swift bindings (git-ignored, built)
│   └── Tests/
│       ├── Unit/<Area>/      # each agent owns the tests for its own area
│       ├── Integration/      # qa
│       └── Performance/      # qa
├── core/
│   ├── Cargo.toml            # infra (workspace)
│   ├── firstcut-core/src/
│   │   ├── lib.rs            # infra (module wiring + UniFFI setup only)
│   │   ├── scan/ meta/ formats/   # core-meta
│   │   ├── order/ batch/          # core-batch
│   │   ├── store/ xmp/ fileops/ session.rs   # core-store
│   │   └── ffi.rs            # infra (exports; each owner requests its additions)
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
| Swift module for the Rust core | `FirstcutCore` (UniFFI) |
| App target / scheme / bundle | `Firstcut` / `Firstcut` / `com.kathird.firstcut` |
| Test photos env var | `FIRSTCUT_TEST_PHOTOS` (default `~/Documents/testing`) |

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
| Build the Rust core + bindings | `scripts/build-core.sh` |
| Generate the Xcode project | `xcodegen` (run after pulling; `Firstcut.xcodeproj` is git-ignored) |
| Build the app | `scripts/build-app.sh` → `dist/Firstcut.app` |
| Swift tests | `xcodebuild -project Firstcut.xcodeproj -scheme Firstcut -destination 'platform=macOS,arch=arm64' test` |

## Git workflow for parallel agents

- **One worktree and one branch per agent, already created.** Work only in yours:
  `~/Documents/projects/Firstcut-wt/<agent>` on branch `agent/<agent>` (upstream set). Don't create
  worktrees. The main checkout (`~/Documents/projects/Firstcut`) stays on `main` and isn't used for work.
- **Reading other agents' files**: always read the live copy from the owner's worktree,
  `~/Documents/projects/Firstcut-wt/<owner>/<path>`, e.g. `.../Firstcut-wt/senior-dev/docs/review.md`
  or `.../Firstcut-wt/core-store/docs/contracts/session-api.md`. Your own checkout only has what's been
  merged to `main`. Committed state is also available via `git show agent/<owner>:<path>`.
- **Commit often, push right after every commit** (`git push -u origin agent/<agent>` the first time).
- **Stay current**: `git merge origin/main` into your branch. Don't rebase shared branches.
  **Never force-push.**
- **Landing work**: open a PR from `agent/<agent>` to `main` (`gh pr create`) and merge it once CI is
  green and senior-dev hasn't requested changes (a P0 finding in your area blocks merging). Ownership is disjoint, so conflicts should be rare. If you hit one in a file you don't own,
  stop and file a request rather than resolving it yourself.
- **Until infra's CI workflow exists**, "CI green" means you ran the checks locally in your worktree
  before merging: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (in `core/`), then
  `xcodegen` and the Swift test command above. Say so in the PR description.
- **Keep `main` green**: after merging, pull `main` and re-run the checks. If `main` is broken and your
  merge caused it, fix it or revert your merge (`git revert`, never a force-push) right away, and note
  it in your status file. If it isn't yours, tell the owner through a request and tell senior-dev.
- Commit messages: `<agent>: <what>` (for example `core-batch: adaptive frame interval`).

## Changelog

- v0.1: initial draft.
