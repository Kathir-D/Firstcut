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
│   │   └── Render/           # pipeline: IOSurface/Metal viewer layer, zoom/pan, overlays
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
└── docs/                     # see docs/README.md
```

## Names

| Thing | Name |
| --- | --- |
| Rust crate (library) | `firstcut-core` → `firstcut_core` |
| Rust CLI binary | `firstcut` (crate `firstcut-cli`) |
| Swift module for the Rust core | `FirstcutCore` (UniFFI) |
| App target / scheme / bundle | `Firstcut` / `Firstcut` / `com.kathird.firstcut` |
| Test photos env var | `FIRSTCUT_TEST_PHOTOS` (default `~/Documents/testing`) |

## Build & test commands

| What | Command |
| --- | --- |
| Rust tests | `cargo test --manifest-path core/Cargo.toml` |
| Rust lint | `cargo fmt --check` and `cargo clippy -- -D warnings` |
| Build the Rust core + bindings | `scripts/build-core.sh` |
| Generate the Xcode project | `xcodegen` |
| Build the app | `scripts/build-app.sh` → `dist/Firstcut.app` |
| Swift tests | `xcodebuild test -scheme Firstcut` |

## Git workflow for parallel agents

- **One worktree and one branch per agent**, so agents never trample each other's checkouts:
  ```sh
  cd ~/Documents/projects/Firstcut
  git worktree add ../Firstcut-wt/<agent> -b agent/<agent>
  ```
  The main checkout (`~/Documents/projects/Firstcut`) stays on `main` and is only used for merging.
- **Reading other agents' live status**: their working files are on disk at
  `~/Documents/projects/Firstcut-wt/<agent>/docs/agents/<agent>.md`, the freshest view. Committed
  state: `git show agent/<agent>:docs/agents/<agent>.md`, since all worktrees share one `.git`.
- **Commit often, push right after every commit** (`git push -u origin agent/<agent>` the first time).
- **Stay current**: `git merge origin/main` into your branch. Don't rebase shared branches.
  **Never force-push.**
- **Landing work**: open a PR from `agent/<agent>` to `main` (`gh pr create`) and merge it once CI is
  green. Ownership is disjoint, so conflicts should be rare. If you hit one in a file you don't own,
  stop and file a request rather than resolving it yourself.
- Commit messages: `<agent>: <what>` (for example `core-batch: adaptive frame interval`).

## Changelog

- v0.1: initial draft.
