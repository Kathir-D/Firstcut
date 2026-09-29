# Firstcut: agent notes

- **One agent works on this repo.** There are no parallel agents, worktrees, or review board any more (they were retired at the 2026-09-29 merge to `main`). Work on `main` (or a short branch merged straight into it); commit small and push right after every commit. Never force-push.
- Start with `task.md` §0 ("Where we left off"): what works, what is broken, the parked code, and the critical path. Then `docs/contracts/build.md` for commands and layout. `task.md` is the source of truth for scope, decisions, and progress; update it (check off tasks, adjust decisions) in the same change as the work.
- Next step as of the last session: `xcodebuild test` does not compile (`App/Tests/Integration/RealRawDecodeTests.swift` vs the in-progress `Pipeline/ImageProvider.swift`). Fix that first. Rust (`cargo test` in `core/`) is green.
- Test photos live in `~/Documents/testing` (Canon R8 C-RAW, 4 games, 42 GB). Never copy them into the repo; reference them via `FIRSTCUT_TEST_PHOTOS`. Real-photo tests are opt-in (`FIRSTCUT_ALLOW_PHOTO_TESTS=1`, `scripts/test-with-photos.sh`).
- Batching changes must be checked against the ground truth in `tests/fixtures/ground-truth/` (it does not exist yet; building it by looking at the photographs is an open task).
- Performance claims need a measurement (see task.md §7.3), not an estimate.
- Duplicate implementations from the merge are in `parked/` and `core/firstcut-core/tests/parked/`; see task.md §0.3 before deleting or reviving them.
- Old per-agent charters and the review board are in git history only: `git show 4d4e43d:docs/review.md`, `git show 4d4e43d:docs/agents/`.
- App icon: source is `logo/firstcut-icon.svg` (and `firstcut-icon-small.svg` for 16/32 px). Regenerate the PNGs with `rsvg-convert` (Homebrew `librsvg`) into `App/Resources/Assets.xcassets/AppIcon.appiconset/` and `logo/icon-{512,1024}.png`; keep the transparent margin (Apple grid, 824 px body on a 1024 canvas).
