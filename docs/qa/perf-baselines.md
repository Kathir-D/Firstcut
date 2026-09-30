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

_Machine: Apple M1 Pro, 16 GB, macOS 27, internal SSD._

**The scan and batching rows below were taken with `firstcut bench --folder`**, a release build of
the Rust core, from a shell:

```sh
cargo build --release --manifest-path core/Cargo.toml
core/target/release/firstcut bench --folder ~/Documents/testing/<Game> --repeat 5
sudo purge && core/target/release/firstcut bench --folder ~/Documents/testing/<Game> --repeat 1
```

That is not the XCTest suite, and the difference matters: this command is a **cold, single process**
over the real files, which is what "open this folder" actually is. The XCTest rows below that are
still empty need the app, because they measure interaction (key to frame, zoom, memory) and a command
line has no window. It is also why these rows carry both a **cold** and a **warm** number — `purge`
first for cold, and the best of five re-runs for warm — while the interactive rows will carry a
p50/p90/max.

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

Measured 2026-09-30, `firstcut bench --folder`, 8 threads. **These are real.**

| Game | Photos | Cold (`purge` first) | Warm (best of 5) | ms/photo cold | Budget |
| --- | --- | --- | --- | --- | --- |
| Game1JENKS | 708 | 0.252 s | 0.026 s | 0.36 | < 3.0 s |
| Gane2NC | 529 | 0.186 s | 0.020 s | 0.35 | < 3.0 s |
| Game3KC | 920 | 0.343 s | 0.033 s | 0.37 | < 3.0 s |
| Game4VRE | 723 | 0.283 s | 0.027 s | 0.39 | < 3.0 s |
| **Scaled to 1,500** | 1500 | **0.535–0.587 s** | **0.054–0.056 s** | ~0.4 | < 3.0 s |

**Inside the target with roughly 5× headroom**, cold, on every game. The spread across the four
games (0.186–0.343 s cold) tracks the photo count, not the content: per-photo cost is 0.35–0.39 ms
cold everywhere, so the scan is I/O-bound and linear.

The cold/warm gap is ~9×, which is the page cache and nothing else: the same headers are re-read from
RAM. A shoot on an internal SSD after the Finder preview has touched it is the warm case, and a card
freshly copied is closer to the cold one. **The cold number is the one the §7.3 target should be
judged against**, because "folder open → first photo" happens before any of it is cached.

### Provisional batches ready (§7.3, < 3.5 s)

Scan + `order()` + `batch()`. `order()` + `batch()` are a rounding error next to the scan, which is
the useful finding: the batching *algorithm* is not what costs time, the header reads are.

| Game | Photos | Batches (provisional) | Total cold | order | batch | Budget |
| --- | --- | --- | --- | --- | --- | --- |
| Game1JENKS | 708 | 157 | 0.252 s | 0.008 ms | 0.275 ms | < 3.5 s |
| Gane2NC | 529 | 109 | 0.186 s | 0.006 ms | 0.248 ms | < 3.5 s |
| Game3KC | 920 | 207 | 0.343 s | 0.008 ms | 0.324 ms | < 3.5 s |
| Game4VRE | 723 | 152 | 0.283 s | 0.007 ms | 0.270 ms | < 3.5 s |
| **Scaled to 1,500** | 1500 | — | **0.535–0.588 s** | ~0.01 ms | ~0.4 ms | < 3.5 s |

`order()` + `batch()` together are **under 0.5 ms for 1,500 photos**, against a 2 s target in §5 and a
3.5 s one here — so the algorithm is roughly 4,000× inside budget and the remaining work on "open →
first photo" is decode and first paint, not metadata.

### One display decode, at the sizes the viewer asks for (§7.1 T2, §7.2)

**Measured 2026-09-30, `RealRawDecodeTests.testDisplayDecodeCosts`, Game1JENKS `IMG_3181.CR3`,
M1 Pro, macOS 27, warm page cache, one image at a time.** The test reports the second of two runs
per size, so the first pays for reading a 15 MB file off the SSD.

| Longest edge asked for | Pixels returned | Time | Cached at |
| --- | --- | --- | --- |
| 6000 (T3, 100% zoom) | 6000×4000 | **146 ms** | 92 MB |
| 3456 (a full-screen 16" viewer) | 3456×2304 | **156 ms** | 30 MB |
| 3000 (a 14" viewer) | 3000×2000 | **147 ms** | 23 MB |
| 2000 (a small window) | 2000×1333 | **61 ms** | 10 MB |
| — the same file through `CGImageSourceCreateImageAtIndex` | 6000×4000, **16-bit, Display P3** | **~850 ms** | **183 MB** |

The last row is what the app used to do for every display frame, and it is the whole point of this
section. It is not the same pixels at a different size: it is 16 bits per component in a colour
space Core Animation has to convert, so every display decode cost half a second and 183 MB, and the
conversion happened on the main thread inside the commit the key-to-frame interval closes on. The
thumbnail call with `kCGImageSourceThumbnailMaxPixelSize` gives 8-bit pixels and subsamples in the
DCT: **5.8× faster at the same size, and 5.8× less memory at a viewer's size.**

A 256 px thumbnail decode of the same file is ~280–310 ms and is *per file, not per pixel* — the
reading behind the one shared decode for the filmstrip and the visual signatures.

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
| — | — | M1 Pro | — | no rows | Wave-1 harness landed with no baselines; load average on the dev machine was ~30 with nine agents compiling, which is exactly what the quiet-machine guard is for. |
| 2026-09-30 | 5c5aa7d | M1 Pro, macOS 27, internal SSD | all four | scan + batches recorded | `firstcut bench --folder`, release build, cold (after `sudo purge`) and warm (best of 5). Interactive rows still empty: they need the app, and this session had no window measurement yet. |
| 2026-09-30 | (this work) | M1 Pro, macOS 27, internal SSD | Game1JENKS | display-decode row recorded; 708-file scan through the core at **0.7 s** | `RealRawDecodeTests`, opt-in photo tests. The same scan through the legacy ImageIO `PhotoFolderScanner` takes 3.1–3.4 s, which is why the test now goes through the core — that is the path the app takes. Interactive rows still empty. |

## Notes

- §3 in todo.md is being corrected (REV-2 / REV-19): drive mode is **not** constant and Game1JENKS
  has 5 gaps of 40–60 ms, not 13. Any threshold that was tuned against the old numbers has to be
  re-measured here, not re-argued.
- §3's frame intervals (90 ms for Game1JENKS and Gane2NC, 170 ms for Game3KC and Game4VRE) are the
  input to the "arrow key ≤ 8 ms" measurement: a burst is 6–11 fps, so the pipeline has 90–170 ms
  per frame to stay ahead of the user. That is the entire engineering budget.
