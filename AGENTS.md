# Firstcut: agent notes

- **One agent works on this repo.** There are no parallel agents, worktrees, or review board any more (they were retired at the 2026-09-29 merge to `main`). Work on `main` (or a short branch merged straight into it); commit small and push right after every commit. Never force-push.
- A new session can be started with the prompt in `CONTINUE.md`.
- Start with `todo.md` §0.5 ("Handoff: where the last session stopped"): what was just finished, what comes next, and what is blocked. Then the rest of §0 (done, owner tasks, open work, practices) and `docs/contracts/build.md` for commands and layout. `todo.md` is the source of truth for scope, decisions, and progress; update it (check off tasks, adjust decisions) in the same change as the work.
- **Before you stop for any reason** (rate limit, context running out, end of session), rewrite todo.md §0.5 with where you are and what comes next, commit, and push. Uncommitted work is lost when a cloud container is reclaimed.
- Test photos live in `~/Documents/testing` (Canon R8 C-RAW, 4 games, 42 GB). Never copy them into the repo; reference them via `FIRSTCUT_TEST_PHOTOS`. Real-photo tests are opt-in (`FIRSTCUT_ALLOW_PHOTO_TESTS=1`, `scripts/test-with-photos.sh`).
- Batching changes must be checked against the ground truth in `tests/fixtures/ground-truth/` (it does not exist yet; building it by looking at the photographs is an open task).
- Performance claims need a measurement (see todo.md §7.3), not an estimate.
- Old per-agent charters and the review board are in git history only: `git show 4d4e43d:docs/review.md`, `git show 4d4e43d:docs/agents/`.
- **GitNexus index flags.** Re-index with `gitnexus analyze --embeddings --pdg` (≈4 min), not the bare `--index-only` the generated section below suggests: `--embeddings` is what turns `query` from keyword-only into semantic search, and `--pdg` is what `pdg_query`, `explain` and `impact(mode: "pdg")` read. Check with `gitnexus status` (freshness) and `.gitnexus/meta.json` → `stats.embeddings` > 0. The index (`.gitnexus/`) is machine-local and git-excluded; never commit it.
- App icon: source is `logo/firstcut-icon.svg` (and `firstcut-icon-small.svg` for 16/32 px). Regenerate the PNGs with `rsvg-convert` (Homebrew `librsvg`) into `App/Resources/Assets.xcassets/AppIcon.appiconset/` and `logo/icon-{512,1024}.png`; keep the transparent margin (Apple grid, 824 px body on a 1024 canvas).

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **Firstcut** (25389 symbols, 62477 relationships, 779 execution flows).

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
