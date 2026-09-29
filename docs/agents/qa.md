# Agent: qa

> **Charter.** Fixed; edit only with the owner's approval. The **Live status** section at the bottom is
> yours to update. Protocol: [task.md §0.4](../../task.md#04-protocol).

## Mission

Independently prove that everything works and stays fast. You don't build features; you build the tests, measure, verify other agents' done claims, and file bugs.

## End goal (definition of done for this agent)

Integration tests drive a full cull of all four games through `AppModel` in both rating modes; the performance suite measures every §7.3 target with recorded baselines and fails on > 10% regression; the zero-miss stress test passes; the ground truth is independently spot-checked; the manual QA checklist passes on macOS 26 and 15; every bug is filed and closed before v0.1.0.

**Done means (and senior-dev has signed it off in `docs/review.md`):** All suites green; baselines recorded; `docs/qa/bugs.md` has no open P0/P1; written sign-off in your status file.

## Owns (only you edit these)

`App/Tests/Integration/`, `App/Tests/Performance/`, `docs/qa/` (`perf-baselines.md`, `qa-checklist.md`, `bugs.md`)

## Does NOT own

Fixing bugs in other agents' code. File them and let the owner fix them.

## task.md sections to read

§7.3 Performance targets, §12 Testing (all), §5.4 ground-truth metrics, §9.7 finish safety

## Contracts

- **Owns:** none (you own `docs/qa/*`)
- **Consumes:** every contract
- **Provides to others:** Test harnesses, perf baselines, bug reports, sign-off on each milestone

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

- [ ] **Wave 1**: `docs/qa/perf-baselines.md` format; fixture loader honoring `FIRSTCUT_TEST_PHOTOS` (skip if absent); `docs/qa/qa-checklist.md` drafted from task.md §9 and §12.
- [ ] **Wave 1**: review every contract draft for testability; file requests for missing hooks (e.g. `PipelineStats.focusMisses`).
- [ ] **Wave 2**: stress test (hold → across a whole game at key-repeat rate, assert 0 focus misses and flat memory); first full perf run; independent spot check of core-batch's ground truth (look at the photos, at least 20% of boundaries per game, 100% of the ambiguous zone in Game1JENKS `IMG_6117–6164`).
- [ ] **Wave 3**: integration tests: full cull per game × both modes through `AppModel`; finish flow on a **copy** of a game (never the originals): every option + undo; crash/resume tests with core-store; XMP import check.
- [ ] **Wave 4**: manual QA on macOS 26 and 15, UI side-by-side with Finder, final perf run, v0.1.0 sign-off.

## Kickoff prompt

Paste this to start a session for this agent:

```
You are the "qa" agent for Firstcut (~/Documents/projects/Firstcut).
Read, in order: task.md §0 (team, protocol, schedule), docs/agents/qa.md (your charter + status),
docs/review.md (fix every open finding under "qa" and "All agents" first, highest severity first),
docs/contracts/build.md, the contracts listed under "Contracts" in your file, the task.md sections listed
in your file, and the "Requests to others" sections of every other file in docs/agents/.
Work only in your own worktree, ~/Documents/projects/Firstcut-wt/qa (branch agent/qa, already
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

_Last updated: 2026-09-29 — wave 1_

### Current focus

Wave 1 is done: harness, fixture loader, baseline format, QA checklist, and a testability pass over
all six contract drafts with requests filed. Next is the wave-2 stress test and the first real perf
run, which start the moment `PipelineStats` (REQ-qa-1) and `Progress::cancelled` (REQ-qa-2) exist —
both are the last thing standing between the contracts and an assertable end-goal.

### Done log

| Date | What | Commit |
| --- | --- | --- |
| 2026-09-29 | Wave-1 harness: `FirstcutTestSupport.swift` (env + games + fixtures + skip helpers), `FixtureHarnessTests`, `PerformanceHarnessTests`. 11 tests green. Deleted the two placeholder tests. | `qa: wave-1 test harness, fixture loader and perf baseline format` |
| 2026-09-29 | `docs/qa/perf-baselines.md` — §7.3 metric set, 10% regression rule, empty-baseline tables, machine + quiet-run rules | `401a54b` |
| 2026-09-29 | `docs/qa/qa-checklist.md` — 13 sections covering §9, §12 and §9.7 safety | `401a54b` |
| 2026-09-29 | `docs/qa/bugs.md` — severity model, BUG-1 (`AFPointsInFocus`), BUG-2 (test build config) | `401a54b` |
| 2026-09-29 | Contract testability pass over all six contracts → REQ-qa-1 … REQ-qa-6 | `401a54b` |

### Measurements taken this session

From the committed `tests/fixtures/exiftool/*.json` (not the photos), so CI can reproduce them:

- 2,880 records across 4 games decode cleanly once `AFPointsInFocus` is read leniently — see BUG-1.
- **0** sub-second ties (< 30 ms apart) in all four games: capture time alone is a total order on the
  test set, so `order()` has no ambiguity to resolve.
- `ShutterCount` is monotonic in all four games (2,880/2,880).
- File-name order equals capture order in all four games with **0 inversions**, and Game4VRE runs
  9146→9999 monotonically — confirming REV-26. `FixtureHarnessTests` now asserts this on purpose,
  with a comment saying why it is a trap.
- This confirms senior-dev's independent computation in REV-2/REV-19; §3 is still wrong in the repo
  and the corrected numbers are still the authoritative ones.

### Blockers

| ID | What | Unblocks when |
| --- | --- | --- |
| REQ-qa-1 | `PipelineStats` undefined → the zero-miss stress test has nothing to assert | pipeline lands the type |
| REQ-qa-2 | `Progress` has no methods → a cancelled finish cannot be asserted | core-store specifies the trait |

Both are wave-1 blockers on *my* wave-2 deliverable. Working against mocks until then.

### Requests to others

| ID | To | Need | Why | Status |
| --- | --- | --- | --- | --- |
| REQ-qa-1 | pipeline | Define `PipelineStats { focusMisses, decodes, thumbnailProgress, queueDepth, tierBytes, peakMemoryBytes }` + `CacheTier` T0…T4 + `resetStats()`, and give `focusMisses` a normative definition (REV-43). Add one more field: `physFootprintBytes`, because the tier counters do not see Metal/ImageIO allocations and the leak we must catch in §7.3 only shows up in the process footprint. | task.md §7.1's zero-miss promise is my end-goal assertion and it is currently unverifiable; §7.3's memory row needs a number that reflects the process, not just the cache | open |
| REQ-qa-2 | core-store | Specify `Progress`: `fn report(&self, done: u32, total: u32)` and `fn cancelled(&self) -> bool`; poll `cancelled()` between files; return a **partial** `FinishReport` that is still undoable; state that `done` counts file operations and add `photos_done` if both are wanted (REV-35). Add `pub fn xmp_sidecar_path(&self, photo) -> Option<String>` so a test can assert XMP output without reaching into the filesystem by guesswork. | task.md §9.7 requires progress + cancel, and my wave-3 finish test must assert a cancelled run leaves consistent, undoable state | open |
| REQ-qa-3 | infra | (a) Add `App/Tests/Support/` as a folder compiled into **both** `FirstcutIntegrationTests` and `FirstcutPerformanceTests`, so shared test code exists once instead of being duplicated per target (BUG-2). (b) Add the `Firstcut-Perf` scheme (REV-13) and put both xcodebuild lines in build.md. (c) `FirstcutTestSupport.swift` must compile under `SWIFT_STRICT_CONCURRENCY: complete` — it is written to, and if `SWIFT_TREAT_WARNINGS_AS_ERRORS` lands (REV-12) I will hear about it. | I own the two test bundles and currently keep a byte-identical copy of the harness in each; the copy will drift. The perf suite must be runnable alone, on a quiet machine, with the photos | open |
| REQ-qa-4 | core-batch | Publish in wave 1, as the review board requires (REV-55): (1) the ground-truth **format** for `tests/fixtures/ground-truth/<game>.json`; (2) a per-game index — batch count, ambiguous-zone boundary count, contact-sheet file names; (3) contact sheets written somewhere stable and named by batch so qa can sample boundaries without re-deriving them. Ground truth must be produced by **looking at the photos**, independently of `batch()`'s output — otherwise F1 is self-certified. | I have to spot-check ≥20% of boundaries per game and 100% of Game1JENKS `IMG_6117–6164` by eye (task.md §12), and I cannot sample from an index I do not have | open |
| REQ-qa-5 | core-batch | The synthetic rollover fixture from task.md §11 (`IMG_9998, IMG_9999, IMG_0001, IMG_0002`, increasing capture times) as a **committed** fixture, plus the scrambled-name dump variant (REV-26). Both are small, both are CI-only, and today nothing protects the "never order by file name" rule. | The test set has zero name/capture inversions and no rollover at all, so a name-based `order()` passes every test we have while breaking a locked decision | open |
| REQ-qa-6 | app-logic | Test hooks in `AppModel`, additive and off the user path: (a) an injectable `ImageProvider` (already implied by `let images: ImageProvider`) with a controllable one; (b) an observable "settled" signal — a way for a test to await thumbnails/decodes finishing instead of sleeping a guessed interval; (c) no wall-clock dependency in the model, or an injectable clock, so a cull driven through `perform(_:)` is deterministic; (d) a documented `perform(_:)` that is safe to call before the session finishes loading, or an explicit "not ready" contract. | I drive a full cull of every game through `AppModel` in wave 3; sleeps are how integration suites become flaky, and a flaky perf suite is worse than none | open |

### Incoming requests

Requests from other agents (`REQ-…`) and senior-dev findings from `docs/review.md` (`REV-…`).

| ID | From | Response | Status |
| --- | --- | --- | --- |
| REV-1 | senior-dev | Done. The live path is the first line of **Notes for other agents** below, and it is in this commit. Worth noting for the audit: my first instinct was to read `docs/review.md` from my own checkout, which is the stale copy; I only found the 55 findings because the kickoff prompt's "(live copy in the senior-dev worktree)" made me look. | fixed |
| REV-3 | senior-dev | Acknowledged: commit + push is the notification, and I push mid-task, not only at session end. | fixed |
| REV-4 | senior-dev | Acknowledged: this status update, the `task.md` ticks and the work are in one commit. | fixed |
| REV-54 | senior-dev | Confirmed and closed the loop as asked. `focusMisses` is REQ-qa-1; I have endorsed senior-dev's field names (`focusMisses`, `decodes`, `thumbnailProgress`, `queueDepth`, `tierBytes`, `peakMemoryBytes`, `resetStats()`) and added exactly one field, `physFootprintBytes`, because the §7.3 memory row cannot be judged from the tier counters alone. `Progress::cancelled` is REQ-qa-2 with the same `done`/`total` shape. Both are filed as requests today, wave 1, not wave 2. | fixed in this commit (requests filed) |
| REV-55 | senior-dev | Agreed, and this is the one I care about most. Ground truth that the same agent produces and scores cannot certify itself. REQ-qa-4 asks for the format, the per-game index (batch count, ambiguous-zone count, contact-sheet names) and stable contact-sheet naming so I can sample boundaries without re-deriving them. I will audit ≥20% per game and 100% of `IMG_6117–6164` by eye, and record which boundaries I checked in this status file so the sample is auditable rather than asserted. | fixed in this commit (request filed) |

### Notes for other agents

- **⚠️ Read `docs/review.md` LIVE, never your own checkout:** `~/Documents/projects/Firstcut-wt/senior-dev/docs/review.md`. The copy in your worktree is stale and has zero findings in it. (REV-1)
- The committed exiftool fixtures are **not** type-stable: `AFPointsInFocus` is an `Int` in 2,722
  records and a comma-separated **string** of AF point indices in 158. A strict decoder throws on
  those 158. If your mock decodes `tests/fixtures/exiftool/`, decode it leniently — see BUG-1 in
  `docs/qa/bugs.md`, which also notes the values are point indices, not a count.
- `Fixtures` is ready to use: `Fixtures.exifToolRecords(for: .game1JENKS)` gives you a typed,
  fully-populated record with `captureUnixMicroseconds` already parsed to UTC µs. It is duplicated
  across the two test bundles until REQ-qa-3 lands.
- Contract testability verdict: **all six contracts have at least one thing qa cannot assert on
  today.** Nothing is unfixable and nothing is a P0 in the contracts themselves, but `PipelineStats`
  (pipeline) and `Progress` (core-store) are hard blockers for the wave-2 gate. Everything else is in
  the requests table above.
- Two more things I found that are not yet anyone's: §7.3's "1,500-file shoot" has no fixture —
  the largest test game is 920 — so someone has to hard-link a 1,500-file shoot to measure against
  the number the plan actually names. I will do it in wave 2. And `ProcessInfo.systemLoadAverage`
  does not exist in Swift on macOS; `getloadavg` does, which is what the quiet-machine guard uses.
