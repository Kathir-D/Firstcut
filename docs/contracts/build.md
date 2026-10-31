# Contract: build, layout & git workflow

- **Owner:** infra
- **Consumers:** everyone
- **Version:** v2.1 (rewritten for the v2 crate ladder; the v1 Xcode/UniFFI build is gone)

## Repository layout

```
Firstcut/
├── crates/                    # the domain ladder (docs/v2-plan.md §4.1), one-way deps:
│   ├── firstcut-types/        #   ids, photo traits, sidecar paths, rating vocabulary, PhotoRow
│   ├── firstcut-meta/         #   scan/, meta/ (cr3.rs, exif.rs, mod.rs), formats/
│   ├── firstcut-xmp/          #   xmp/ reads & writes `<basename>.xmp` sidecars
│   ├── firstcut-fileops/      #   trash, volume identity, free space, reveal-in-Finder
│   ├── firstcut-batching/     #   order.rs + batch/ (metadata grain, visual signatures)
│   ├── firstcut-catalog/      #   the sessions SQLite store (db/records/identity/schema)
│   ├── firstcut-decode/       #   decode traits + the embedded-JPEG fast path (§4.2)
│   ├── firstcut-decode-libraw #   LibRaw bindings, the only crate allowed a [lints.rust] table (§4.5)
│   ├── firstcut-engine/       #   session.rs: the app's one long-lived conversation
│   └── firstcut-ui/           #   egui surfaces (lands milestone by milestone)
├── apps/
│   ├── firstcut/              # the native desktop shell (egui), per OS
│   └── firstcut-cli/          # the developer CLI (`firstcut …`; §5 of docs/v2-plan.md)
├── xtask/                     # `cargo xtask {ci,layers,smoke,assets}` — the gate (§9)
├── scripts/
│   └── test-with-photos.sh    # the opt-in real-photo runs; milestone-only (§9)
├── tests/fixtures/            # core-batch (ground truth, metadata dumps); core-meta (headers)
├── .github/workflows/v2-ci.yml  # the 3-OS runner (fmt, clippy, layers, tests; matrix)
├── Cargo.lock  VERSION?  README.md  LICENSE  THIRD-PARTY-NOTICES.md   # infra
├── todo.md                    # everyone may tick their own boxes; §0.5 is the handoff
└── docs/                      # contracts + docs/v2-plan.md (the branch's own contract)
```

There is no generated code in the tree: no UniFFI, no xcodegen output, nothing built into VCS.

## Names

| Thing | Name |
| --- | --- |
| Domain crates | `firstcut-types`, `firstcut-meta`, `firstcut-xmp`, `firstcut-fileops`, `firstcut-batching`, `firstcut-catalog`, `firstcut-decode`, `firstcut-decode-libraw`, `firstcut-engine`, `firstcut-ui` |
| CLI binary | `firstcut` (crate `apps/firstcut-cli`) |
| App binary | `firstcut` too — the GUI lives in `apps/firstcut`; the CLI is a separate package (§5 of docs/v2-plan.md) |
| Test photos env vars | `FIRSTCUT_TEST_PHOTOS` (default `~/Documents/testing`) and `FIRSTCUT_ALLOW_PHOTO_TESTS=1` — both required, per §9 |

## The ladder edge is one file

Every allowed crate-to-crate dependency lives in `xtask/src/layers.rs`. A crate may depend only
on crates above it; `cargo xtask layers` fails CI on any upward or sideways edge, or on any
workspace member the table does not know about. Add a new crate there first.

## Build & test commands

| What | Command |
| --- | --- |
| Per-commit gate | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, touched crates' tests, `cargo xtask layers` when crate edges moved |
| Milestone gate | `cargo xtask ci` (fmt, clippy, layers, assets, workspace tests; writes `target/ci/report.json` + `summary.md`) |
| One crate's tests | `cargo test -p firstcut-meta --lib` |
| Desktop shell | `cargo run -p firstcut` (or `-- --smoke`) |
| CLI | `cargo run -p firstcut-cli -- --help` |
| Opt-in photo tests | `scripts/test-with-photos.sh` (exports both env vars, runs `--include-ignored`) |
| Window smoke, 3 OSes | `cargo xtask smoke` (proven by CI, not by hand) |

Toolchain is pinned by `rust-toolchain.toml`; lint config is `rustfmt.toml` + `clippy.toml` +
the workspace `[lints]` in the root `Cargo.toml`. **Never crash outranks feature work** (§4.5):
`unwrap/expect/panic/todo/indexing` are denied in product code, tests are exempted by
`clippy.toml`, and the single `unsafe` carve-out is `firstcut-decode-libraw` (D3), enforced
mechanically by `cargo xtask layers`.

## CI

`.github/workflows/v2-ci.yml` runs the milestone gate on all three OSes (the window matrix is
where cross-OS reality is proven). It is the only workflow; v1's `ci/xcode26/release/screenshots`
lanes died with the Swift tree, and the release lanes (§6 of docs/v2-plan.md) come back for the
native app at the packaging milestone.

## Day-one bootstrap (in the tree already)

| Thing | Where | Owner |
| --- | --- | --- |
| Workspace scaffold + the empty crates of the ladder | root `Cargo.toml`, `crates/`, `apps/`, `xtask/` | infra |
| exiftool dumps of the test games | `tests/fixtures/exiftool/<game>.json` | core-batch (read-only for others) |
| Ground-truth batching files | `tests/fixtures/ground-truth/` | owner (never synthesised) |

- **Dependency rule**: new external deps go into `[workspace.dependencies]` in the root
  `Cargo.toml` (alphabetical); a crate then takes them via `{ workspace = true }`. New *internal*
  edges go into `xtask/src/layers.rs` — nowhere else.
- `Cargo.lock` conflicts: take the incoming version, then `cargo build`.

## Git workflow

- **Work on `v2`** (D9). Draft PR v2 → `main` stays a draft until the owner moves it; agents never
  merge into `main`. Read-only ops on `main` (`git fetch`, `git log main`, `git diff main…v2`)
  are fine; nothing else touches it.
- Commit small, imperative messages, **push right after every commit**. Never force-push; tags
  and releases are the owner's.
- Before any commit that moves symbols around, or before committing while a fresh index exists:
  GitNexus `detect_changes --scope all` (and impact analysis before editing moved-around symbols).
- **Keep the branch green**: a commit only goes up when the per-commit gate passed locally; a
  pushed-but-red state gets its own small fix commit right after.
- The §4.5 sweep style applies to anything ported later: checked reads, tested truncation.

## Changelog

- v0.1–v1.0: the v1 contract (Xcode app, UniFFI bridge, `core/` workspace, xcodegen, TCC-aware
  test hosts). Retired with the Swift tree; see `git log -- docs/contracts/build.md` under the
  v1 history for those sections.
- v2.0: rewritten for the v2 native-only workspace (this file's body); the UniFFI section, the
  `~/Documents` TCC rules, and the xcodegen/bootstrap paragraphs dropped with the build they
  described.
- v2.1: M1 port reality — the layout table now matches the tree after the port (v1's `core/` and
  the Xcode project are deleted in the same commit), `scripts/` is down to the retargeted
  `test-with-photos.sh`, and the remaining v1-isms in CI/commands are replaced by the §9 gates.
