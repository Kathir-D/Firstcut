# Bug list

- **Owner:** qa. qa files bugs in other agents' code; **the owner fixes them**, qa closes them.
- Senior-dev's `docs/review.md` findings (`REV-n`) are a separate, pre-merge channel. A `REV` that
  turns into shipped broken behaviour becomes a `BUG` here; a `BUG` found before merge should
  become a `REV`.

**Severity:** **P0** data loss, a crash on real photos, or a broken build · **P1** a documented
feature does not work, a §7.3 target is missed, or work is silently lost · **P2** wrong or missing
behaviour with a workaround · **P3** cosmetic.

**No open P0/P1 at the v0.1.0 sign-off** (docs/agents/qa.md, end goal).

## Open

| ID | Sev | Area | Owner | Found | Summary | Status |
| --- | --- | --- | --- | --- | --- | --- |
| BUG-1 | P2 | `tests/fixtures/exiftool/*.json`, photo-meta.md | core-batch (fixture), core-meta (contract) | 2026-09-29 | `AFPointsInFocus` is not type-stable and is not a count | open |
| BUG-2 | P2 | `App/Tests/**` build config | infra | 2026-09-29 | No shared test-support folder; perf suite cannot run alone | open |

### BUG-1 — `AFPointsInFocus` is a list, and sometimes a string

**Found by:** `FixtureHarnessTests` in `App/Tests/Integration/`, first run of the wave-1 harness.

**What.** In the committed exiftool dumps, `AFPointsInFocus` has two shapes:

- **2,722 of 2,880 records**: a JSON integer.
- **158 records**: a JSON **string** holding a comma-separated list, e.g.
  `"14,15,43,44,45,75,76,77,78,104,105,106,…"` — up to 130 indices.

So the field is neither type-stable nor a scalar. Any strict decoder of the committed fixtures —
Swift's `Int`, `serde`'s `i32` — throws `typeMismatch` on those 158 records. That is not a cosmetic
problem: the fixtures are what app-logic's `MockSession` and every Swift mock decode from
(photo-meta.md "Fixtures for consumers"), and a decoder that only works for 94% of them produces
mocks that disagree with the real parser on exactly the frames that use point-expansion AF.

**Also.** The values are **AF point indices, not a count of in-focus points**. photo-meta.md
documents `AfInfo { area_mode, points }` and REV-21 already assumes `AFPointsInFocus` is a count
that is often `0`. It is not: `0` here means "point 0", i.e. the centre point of a 639-point
registration array. Any overlay logic built on "how many points are in focus" is reading the wrong
quantity. (This does not contradict REV-21's conclusion that the overlay often has nothing to draw
— that conclusion still needs measuring.)

**Needed, in order:**

1. core-batch: regenerate or document the fixtures so each record has a stable type. Cheapest
   correct option: split into `AFPointsInFocus` (always an array of ints) and drop the raw string
   form. If the fixtures stay as raw exiftool output, say so in photo-meta.md and the decoders must
   be lenient — qa's harness is (see `decodeLossyIntListIfPresent`).
2. core-meta: state in `AfInfo` what `in_focus` means given that these are indices into an
   `AFAreaMode`-dependent array, and cross-link REV-21.

**Interim.** `App/Tests/Integration/Support/FirstcutTestSupport.swift` decodes both shapes, so the
suite is green today. Anyone else reading these fixtures needs the same leniency or the same fix.

### BUG-2 — no shared test-support folder, and no way to run the perf suite alone

**Found by:** trying to run the wave-1 harness.

**What.** Two problems, both in `project.yml` (infra), both blocking qa's wave-1/2 deliverables:

1. `FirstcutIntegrationTests` and `FirstcutPerformanceTests` each compile only their own folder, so
   shared test code has to exist twice. qa's harness is currently duplicated verbatim in
   `App/Tests/Integration/Support/FirstcutTestSupport.swift` and
   `App/Tests/Performance/Support/FirstcutTestSupport.swift`. That copy will drift the moment anyone
   edits one of them. It needs a single `App/Tests/Support/` folder in both targets (REQ-qa-3).
2. There is no perf-only scheme (REV-13). The perf suite needs a quiet machine and 42 GB of photos;
   running it inside the normal test pass produces either skips nobody reads or timings that are
   noise.

**Interim.** qa keeps the two copies byte-identical and re-`cp`s them; the duplication is deleted
as soon as `App/Tests/Support/` exists.

## Closed

| ID | Sev | Owner | Summary | Fixed in | Closed by qa |
| --- | --- | --- | --- | --- | --- |
| — | | | | | |
