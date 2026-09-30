# Continuation prompt

Paste the block below into a new agent session (Claude Code or similar) opened on this repository
to pick up where the last session stopped. It holds no state of its own: the state is in
`todo.md` §0.5, which every session rewrites before it stops.

---

```text
You are taking over Firstcut, a macOS photo-culling app. The core is Rust (core/), exposed to the
Swift/SwiftUI app (App/) over UniFFI. The repository is Kathir-D/Firstcut.

1. Read AGENTS.md, then todo.md §0 (start with §0.5, "Handoff: where the last session stopped"), then
   docs/contracts/build.md and the contract for anything you touch. todo.md is the source of truth
   for scope, decisions and progress.
2. Check the branch state: `git fetch origin && git log --oneline origin/main..HEAD` and
   `git status`. Work on the branch §0.5 names, or on main (or a short branch merged straight into
   it). Commit small and push right after every commit. Never force-push. Move main only by a
   fast-forward to a commit whose ci.yml run is green.
3. Do the "Next, in order" list in §0.5 from the top. Skip an item only if it is blocked, and say
   why in §0.5.
4. Before every push, from core/: `cargo fmt --check`,
   `cargo clippy --all-targets -- -D warnings`, `cargo test`. Swift builds only on CI (macOS) when
   there is no Swift toolchain, so a Swift change is done only when ci.yml is green on it.
5. Rules that do not bend:
   - A task is done when a test asserts it. A performance claim needs a measurement (todo.md §7.3,
     docs/qa/perf-baselines.md), never an estimate.
   - Test photos live in ~/Documents/testing (FIRSTCUT_TEST_PHOTOS); never copy them into the repo.
     Real-photo tests are opt-in (FIRSTCUT_ALLOW_PHOTO_TESTS=1).
   - Batching changes are checked against tests/fixtures/ground-truth/, which only the owner may
     write, by looking at the photos.
   - Update todo.md (check off tasks, adjust decisions) in the same commit as the work.
6. Before you stop for any reason (rate limit, context limit, end of session), rewrite todo.md
   §0.5 with where you are, what comes next and what is blocked. Then commit and push.
   Uncommitted work is lost.

Start by summarising §0.5 in three lines, then begin the first unblocked item.
```
