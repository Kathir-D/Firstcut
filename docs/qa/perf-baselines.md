# Performance baselines

- **Owner:** qa
- **Targets:** todo.md §7.3
- **Format version:** 1 (this document). Bump when the metric set changes, and keep old rows.

Every number in this file is a **measurement**, taken from a test that still exists. No estimates,
no numbers copied from a blog post, no "should be fine". A row that has no test behind it does not
belong here.

## How a run works

```sh
# Only the performance bundle, on a machine that is doing nothing else.
# The Firstcut-Perf scheme is requested in REQ-qa-3; until it lands:
xcodegen
xcodebuild -project Firstcut.xcodeproj -scheme Firstcut -destination 'platform=macOS,arch=arm64' \
  -only-testing:FirstcutPerformanceTests test
```

Rules the suite enforces, so a baseline cannot be recorded on a bad machine:

- The photos come from `FIRSTCUT_TEST_PHOTOS` (default `~/Documents/testing`) and the run **skips**
  if they are absent. Baselines are never recorded from synthetic data.
- Every timing test calls `skipUnlessMachineIsQuiet()` first (1-minute load average ≤ 1.0 via
  `getloadavg`). A busy run skips and reports nothing.
- The machine that produced a baseline is named in the table. A different machine gets its own
  section; the two are never compared.

## Regression rule

A metric **fails** when it is more than **10% worse** than its recorded baseline. For metrics with a
hard target in §7.3 (arrow key ≤ 8 ms, zoom < 150 ms, scan < 3 s), the target wins: a baseline above
the target is already a failure, whatever the delta.

```
regression = (measured - baseline) / baseline > 0.10   → fail
```

`focusMisses` and memory growth are pass/fail, not ratios: one focus miss is a failure no matter how
fast everything else was.

## Baselines

_Machine: Apple M1 Pro, 16 GB, macOS 26, internal SSD. First baseline run lands with the wave-2
perf suite; every cell below is empty until then, on purpose._

### Open → first photo (§7.3, < 1 s)

| Game | Photos | p50 | p90 | max | Budget |
| --- | --- | --- | --- | --- | --- |
| Game1JENKS | 708 | — | — | — | < 1.0 s |
| Gane2NC | 529 | — | — | — | < 1.0 s |
| Game3KC | 920 | — | — | — | < 1.0 s |
| Game4VRE | 723 | — | — | — | < 1.0 s |
| Synthetic 1,500-file shoot | 1500 | — | — | — | < 1.0 s |

The 1,500-file row is the one §7.3 actually names. None of the test games has 1,500 photos (the
largest is 920), so a 1,500-file shoot has to be synthesised by **hard-linking** the real files into
a temp folder (no copying 42 GB, no modifying the originals). qa builds that fixture in wave 2.

### Metadata scan, header reads only (§7.3, < 3 s)

| Game | Photos | p50 | p90 | max | Budget |
| --- | --- | --- | --- | --- | --- |
| Game1JENKS | 708 | — | — | — | < 3.0 s |
| Gane2NC | 529 | — | — | — | < 3.0 s |
| Game3KC | 920 | — | — | — | < 3.0 s |
| Game4VRE | 723 | — | — | — | < 3.0 s |
| Synthetic 1,500-file shoot | 1500 | — | — | — | < 3.0 s |

### Provisional batches ready (§7.3, < 3.5 s)

| Game | Photos | Batches (provisional) | p50 | Budget |
| --- | --- | --- | --- | --- |
| Game1JENKS | 708 | — | — | < 3.5 s |
| Gane2NC | 529 | — | — | < 3.5 s |
| Game3KC | 920 | — | — | < 3.5 s |
| Game4VRE | 723 | — | — | < 3.5 s |

### All thumbnails + hashes, background (§7.3, < 20 s)

| Game | Photos | p50 | p90 | Budget |
| --- | --- | --- | --- | --- |
| Game1JENKS | 708 | — | — | < 20 s |
| Gane2NC | 529 | — | — | < 20 s |
| Game3KC | 920 | — | — | < 20 s |
| Game4VRE | 723 | — | — | < 20 s |

### Arrow key → sharp photo, cached (§7.3, ≤ 8 ms at 120 Hz)

| Metric | p50 | p95 | p99 | max | Budget |
| --- | --- | --- | --- | --- | --- |
| `photo.next` / `photo.previous` in a cached batch | — | — | — | — | ≤ 8 ms |
| `batch.next` / `batch.previous` | — | — | — | — | ≤ 8 ms |

Measured from `perform(_:)` to the next frame presented, not to the call returning.

### Zero focus misses (todo.md §7.1, the app's core promise)

| Scenario | Focus misses | Requirement |
| --- | --- | --- |
| Hold `→` through Game1JENKS at key-repeat rate | — | 0 |
| Hold `→` through Gane2NC | — | 0 |
| Hold `→` through Game3KC | — | 0 |
| Hold `→` through Game4VRE | — | 0 |
| `⌘→` through every batch, then `←` back | — | 0 |
| Arrow sweep at 30 Hz (worse than key repeat) | — | 0 |

Needs `PipelineStats.focusMisses` (pipeline-api.md) — REQ-qa-1 / REV-43.

### 100% zoom (§7.3, < 150 ms first time)

| Metric | p50 | p95 | max | Budget |
| --- | --- | --- | --- | --- |
| Click to 100%, zoom lock **off** (first time) | — | — | — | < 150 ms |
| Click to 100%, zoom lock **on** (prefetched) | — | — | — | instant |
| Full RAW decode (T4), 1 image | — | — | — | measured, no target |

### Memory (§7.3, no leaks)

| Metric | After full cull | Peak | Budget |
| --- | --- | --- | --- |
| Resident footprint, Game1JENKS | — | — | — |
| Resident footprint, Game3KC (largest) | — | — | — |
| Growth per 1,000 arrow-key steps | — | — | ~0 |

Two things get recorded: the tier-by-tier bytes from `PipelineStats.tierBytes`, and the **process
footprint** from `phys_footprint` (which includes the Metal/ImageIO allocations the tier counters do
not see). A leak that only shows up in the footprint is the one that ships as "Firstcut eats 8 GB
after a game" — the tier counters alone would call that clean.

## Run log

| Date | Commit | Machine | Games | Result | Notes |
| --- | --- | --- | --- | --- | --- |
| — | — | — | — | — | Wave-1 harness landed with no baselines; load average on the dev machine was ~30 with nine agents compiling, which is exactly what the quiet-machine guard is for. |

## Notes

- §3 in todo.md is being corrected (REV-2 / REV-19): drive mode is **not** constant and Game1JENKS
  has 5 gaps of 40–60 ms, not 13. Any threshold that was tuned against the old numbers has to be
  re-measured here, not re-argued.
- §3's frame intervals (90 ms for Game1JENKS and Gane2NC, 170 ms for Game3KC and Game4VRE) are the
  input to the "arrow key ≤ 8 ms" measurement: a burst is 6–11 fps, so the pipeline has 90–170 ms
  per frame to stay ahead of the user. That is the entire engineering budget.
