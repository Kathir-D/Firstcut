# Firstcut v2 — agent notes

The pure-Rust rewrite. **`docs/v2-plan.md` is the contract for this branch** — every
decision is recorded there; when reality diverges, the file gets a note in the same
commit, not a quiet workaround. If a decision looks wrong, stop and ask the owner
instead of deviating.

## What runs where

- **Branch**: work on `v2` only. Draft PR v2 → main stays a draft until the owner
  moves it; agents never merge v2 into `main` (only the owner fast-forwards `main`
  after the merge gate in the plan's §6). Read-only ops on `main` (`git fetch`,
  `git log main`, `git diff main…v2`) are fine; nothing else touches it.
- **Push after every commit.** Small, single-purpose commits; imperative messages.
  Never force-push. Tags (`v2.0.0`) and Releases are the owner's.
- **No web/WASM.** The app is native-only on macOS, Windows and Linux
  (owner amendment 1). Do not shape crates for wasm and do not mention it.
- **Before you stop** for any reason (rate limit, context end, session end): rewrite
  `todo.md` §0.5 with exactly where you are, what's next, what's blocked, commit, push.
  Uncommitted work is lost on a container reclaim.

## Never crash outranks feature work (docs/v2-plan.md §4.5)

- Non-test code returns errors instead of panicking: **no `unwrap()`, `expect()`,
  `panic!`, `todo!`, unsafe**, and checked indexing on anything derived from input.
- Enforced mechanically: workspace lints `unsafe_code = "forbid"` plus
  `clippy::unwrap_used / expect_used / panic / todo / indexing_slicing = deny`
  (`-D warnings` on top). Tests are exempted by `clippy.toml`
  (`allow-unwrap-in-tests = true` …), which is reviewed config, not a loophole.
- **The single `unsafe` carve-out is `crates/firstcut-decode-libraw`** — the LibRaw
  (C++) bindings backend, approved by the owner (decision D3). Its own `[lints.rust]`
  table is the only one permitted in the workspace; `cargo xtask layers` fails CI if
  any other crate carries one, so the exception cannot spread.
- Every crash fix ships a regression test. Existing core code is swept to these lints
  as its modules are ported, not "later".

## Layered workspace (docs/v2-plan.md §4.1)

- One-way crate ladder. The allowed edges live in exactly one file,
  `xtask/src/layers.rs` — **it is the spec, not prose elsewhere**. A crate may depend
  only on crates above it; `cargo xtask layers` fails CI on any upward or sideways
  edge, and on any workspace member the table does not know about.
- The UI never mutates engine state except by dispatching commands (§4.3).
- `firstcut-decode` attaches backends, never bakes one in: the LibRaw backend is
  wired in by the apps, and the embedded-JPEG fast path stays `unsafe`-free.

## Tiered test discipline (docs/v2-plan.md §9, owner amendment 2)

- **Per commit gate**: `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, the touched crates' unit tests, and
  `cargo xtask layers` when crate edges could have moved. Nothing heavier.
- **Milestone gate, and only then**: the full `cargo xtask ci` (fmt, clippy, layers,
  assets, workspace tests; writes `target/ci/report.json` + `summary.md`). A
  milestone is not done until that passes on this machine, and the 3-OS workflow
  (`.github/workflows/v2-ci.yml`) is green.
- Real-photo integration tests are **milestone-only and opt-in**
  (`FIRSTCUT_TEST_PHOTOS`, `FIRSTCUT_ALLOW_PHOTO_TESTS=1`); they never enter the
  per-commit loop and photos never enter the repo.
- Never skip a failing test: diagnose before the next commit, or name it with the
  reason in the milestone report. A red fix is its own small commit, pushed
  immediately.

## Parity, not invention

v1's contract docs in `docs/contracts/` describe the behavior being ported
(batching, session, photo-meta semantics); the v1 feature list is the parity target
(§7's checklist is the merge gate). For any behavior question, port the v1 rule and
its tests — do not redesign.

- Boundary ground-truth data is **never synthesised** (todo.md §0.2; owner task).
- Performance claims are **measured** rows in the plan's §5 (mind the cold-file-cache
  trap: whichever decode runs first pays for the read). A number without a
  measurement is not a result.
- No proprietary assets, ever: every icon/font/image gets an `assets/ATTRIBUTION.md`
  entry in the commit that adds it (`cargo xtask assets` checks this).

## Local commands

| What | Command |
| --- | --- |
| Full gate (milestones) | `cargo xtask ci [--summary-md]` |
| Layer check | `cargo xtask layers` |
| Window smoke (all 3 OSes, proven by CI) | `cargo xtask smoke` |
| Format / lint everything | `cargo fmt --all` / `cargo clippy --workspace --all-targets -- -D warnings` |
| One crate's tests | `cargo test -p firstcut-meta --lib` |
| Desktop shell | `cargo run -p firstcut` (or `-- --smoke`) |
| CLI | `cargo run -p firstcut-cli -- --help` |

Toolchain is pinned by `rust-toolchain.toml` (1.99.0 stable); lint config is
`rustfmt.toml` (max 150, small heuristics max) + `clippy.toml` + the workspace
`[lints]` in the root `Cargo.toml`.

## GitNexus

Re-index after big file moves (M0's restructure landed one) with
`gitnexus analyze --embeddings --pdg` (≈4 min; `--embeddings` is what makes `query`
semantic, `--pdg` feeds `pdg_query`/`explain`/`impact(mode: "pdg")`). Check
freshness with `gitnexus status`; `.gitnexus/` is machine-local, never commit it.
MUST run impact analysis before editing moved-around symbols, and
`detect_changes --scope all` before committing while the index is fresh.

<!-- gitnexus:auto (regenerated by `gitnexus analyze`; keep the markers) -->

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **Firstcut** (12659 symbols, 31671 relationships, 901 execution flows).

> Index stale? Run `node .gitnexus/run.cjs analyze --index-only` from the project root — it auto-selects an available runner. No `.gitnexus/run.cjs` yet? Bootstrap with `npx`, `bunx`, or `pnpm dlx` — e.g. `bunx gitnexus@latest analyze` (npm 11 npx crash; #1939).

## Always Do

- **MUST run impact before editing.** Use `impact({target: "symbolName", direction: "upstream"})` or `node .gitnexus/run.cjs impact "symbolName" --direction upstream --repo .`; report callers, processes, and risk. Never substitute grep for graph analysis. For unified PDG impact, add `mode: "pdg"` with optional `line: <N>` — it returns statement-level `affectedStatements` over CDG + REACHING_DEF and inter-procedural symbols in `interproceduralByDepth`/`byDepth`; no-layer/degraded PDG results are UNKNOWN-risk notes (`--pdg` layer). CLI equivalent: `node .gitnexus/run.cjs impact "symbolName" --direction upstream --mode pdg --line <N> --repo .`.
- **MUST analyze graph changes before committing.** Use `detect_changes({scope: "all"})` (MCP) or `node .gitnexus/run.cjs detect-changes --scope all --repo .` (CLI fallback). `partial: true` or `truncated: true` is not a clean check — a zero means unseen, not unaffected; re-run it. For regression review: `detect_changes({scope: "compare", base_ref: "main"})` or `node .gitnexus/run.cjs detect-changes --scope compare --base-ref "main" --repo .`.
- MUST warn on HIGH/CRITICAL `risk` pre-edit; never use `riskSharedAxes` to waive a HIGH/CRITICAL `risk` warning. Compare File/symbol: MCP File omits axes; Graph-RAG expands File.
- **MUST treat `risk: UNKNOWN` as unresolved, not as low.** An empty caller set is not evidence the symbol is unused — it can also mean the callers are not resolvable by the index (plain-object property access, dynamic dispatch, cross-language calls). `impact` pairs `UNKNOWN` with a `riskNote` saying so. Confirm with a text search before treating the symbol as safe to change or delete; do not proceed on the strength of a zero.
- **MUST use `query({search_query: "concept"})` for concepts/flows, `context({name: "symbolName"})` for a named symbol, or `impact` for blast radius, on read-only callers, dependencies, imports, or execution flow.** Graph first; text search only for empty/`UNKNOWN`/literals.
- For security review, `explain({target: "fileOrSymbol"})` lists taint findings (source→sink flows; needs `analyze --pdg`).
- For control/data dependence, `pdg_query({mode: "controls", target: "fileOrSymbol"})` answers "under what condition does X run?" (CDG, incl. guard clauses) and `pdg_query({mode: "flows", target, variable})` traces "where does variable Y flow?" (REACHING_DEF). `--pdg` layer.

## Never Do

- NEVER edit a function, class, or method before MCP/CLI impact analysis.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis, and never read `UNKNOWN` as an all-clear — it means the walk could not answer, which is the one verdict that requires confirming by other means.
- NEVER rename symbols with find-and-replace — use `rename` which understands the call graph.
- NEVER commit before MCP/CLI graph change analysis.

## Resources

| Resource | Use for |
| --- | --- |
| `gitnexus://repo/Firstcut/context` | Codebase overview, check index freshness |
| `gitnexus://repo/Firstcut/clusters` | All functional areas |
| `gitnexus://repo/Firstcut/processes` | All execution flows |
| `gitnexus://repo/Firstcut/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
| --- | --- |
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
