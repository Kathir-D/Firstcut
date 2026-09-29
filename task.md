# Firstcut — Task Plan

> The single source of truth for what Firstcut is, the decisions already made, and every piece of
> work needed to ship it. Check items off as they land. When a decision changes, update the
> **Decisions** table first, then the tasks that depend on it.

**Status (2026-09-29): everything is on `main`, and parts of it are broken.** Nine parallel agents
wrote the first version, then their work was consolidated onto one tree and merged. One agent (you)
now continues from here. Read [§0](#0-where-we-left-off) first: it says what works, what does not, and
what to do next.

---

## 0. Where we left off

> Every session starts by reading this section, then `AGENTS.md`.

### 0.1 How the project is run now

**One agent, one branch, no reviewer.** Work on `main` directly or on short branches merged straight
into it. Commit and push after every meaningful change (the owner's global rule). Never force-push.
The nine-agent arrangement (roster, worktrees, review board, per-agent charters) is gone; its history
is in git only:

```sh
git show 4d4e43d:docs/review.md            # the senior-dev review board, 74+ findings (REV-n)
git show 4d4e43d:docs/agents/worker.md     # the last worker charter and status
git show 4d4e43d:docs/agents/archive/      # the original eight area charters
```

Because there is no reviewer any more, the discipline is: **a task is not done until something
asserts it, and a performance number is not done until it is measured.** For anything visual, build,
launch, `screencapture`, *look at the image*, compare with Finder, fix, repeat (see §0.5).

### 0.2 State of the tree (measured 2026-09-29, at the merge to `main`)

| Check | Result |
| --- | --- |
| `cargo test --manifest-path core/Cargo.toml` | **green** — 274 lib tests + integration tests |
| `xcodebuild -project Firstcut.xcodeproj -scheme Firstcut -destination 'platform=macOS,arch=arm64' test` | **does not compile** — `App/Tests/Integration/RealRawDecodeTests.swift:54,74`: missing argument `orientation` in a call. It is the test that goes with the in-progress `App/Sources/Pipeline/ImageProvider.swift` (memory budget, `focusMisses` counter), which was mid-edit when work stopped |
| Boundary F1 (batching) | **UNMEASURED** — no verified ground truth exists yet (§5.4). The suite prints a loud SKIPPED instead of passing |
| `swift-format`, warnings-as-errors in CI | not enforced yet |

Fix the Swift test compile error first; it is the only thing between here and a green tree.

### 0.3 What was merged, and the parked code

The merge brought together two lines of work that had each implemented the same things separately:
the **consolidated tree** (`integrate/consolidate`: Rust `meta/cr3.rs` CR3 reader, `scan/mod.rs`,
`session.rs`, `ffi.rs`, `Pipeline/ImageProvider.swift`, `CGImageViewerHost.swift`, real
`AppEnvironment` wiring) and the **worker branch** (`agent/worker`: its own CR3 parser in
`scan/cr3.rs`, a real-folder `Session`, `LiveCullViewState`, `PreviewPipeline`, contact-sheet tools).
On every conflict the consolidated tree won. The worker's parallel versions of the same features were
**parked, not deleted**, in case they are better in places:

| Parked at | What it is | To use it |
| --- | --- | --- |
| `parked/worker-scan-cr3.rs` | Worker's CR3 parser (813 lines) | Compare with `core/firstcut-core/src/meta/cr3.rs`; both claim exiftool parity on 2,880 files |
| `parked/worker-swift/App/…` | `LiveCullViewState`, `PreviewPipeline`, `PreviewImageSource`, `PreviewViewerView`, `CoreErrorInfo`, `CoreSessionBackendRealFolderTests` | Move back under `App/`, reconcile with `AppEnvironment`, regenerate the project |
| `core/firstcut-core/tests/parked/worker_{cr3,session}.rs` | Worker's integration tests against its own API | Cargo does not build `tests/parked/`; port them to the consolidated API |
| `tools/contact-sheet/`, `scripts/build-contact-sheet.sh`, `scripts/test-with-photos.sh` | Contact-sheet renderer (Core Text) and the opt-in real-photo test runner | Live, working from the worker branch; the batch/`eval` CLI hooks they pair with were overwritten by the consolidated `firstcut-cli` |

Also taken from the consolidated side over the worker's: the batching files (`batch/*`, REV-63 was
fixed differently on each side), `store/*`, `xmp/*` and `fileops/*`. Worker-only fixes to
`fileops` (a 700-line expansion, REV-78 "one function answers is-this-kept") were dropped with it;
re-check that the Finish step decides keep-versus-trash from the *mapped tier*, never the raw `keep`
field, and that unkeeping clears the 5 stars it invented.

### 0.4 Known open issues (from the old review board)

Not exhaustive; the full text is in the review board (`git show 4d4e43d:docs/review.md`). Verify each
against the code before trusting it — several were fixed on one side of the merge only.

- **Ground truth (blocker for M1).** `tests/fixtures/ground-truth/<game>.json` for all four games,
  built by *looking at the photographs*, especially `Game1JENKS IMG_6117–6164`. Do not synthesise it.
- REV-64 `visual_sig` must call the Rust reference, never a Swift reimplementation; it skipped the
  256 px stage in the contract.
- REV-65 ordering key: §5.1 says camera serial first, §2 says time + subsec + shutter count. Decide.
- REV-68 per-photo rename reconciliation (a rename changes `PhotoId` and orphans its rating).
- REV-12 / REV-59 CI: `SWIFT_TREAT_WARNINGS_AS_ERRORS` and a `swift-format` step are missing; CI uses
  Xcode 16.2 while local builds use Xcode 27 (REV-58).
- REV-72 / REV-73 leftover mirrored types (`PipelineMirror`, renamed `SessionTypes`) — one definition
  of each type is the rule; delete stand-ins.
- REV-38 the pipeline contract said `CALayer.contents = IOSurfaceRef`, which cannot work; the real host
  (`CGImageViewerHost`) uses a `CGImage`. Update `docs/contracts/pipeline-api.md` to match.
- Homebrew tap (`Kathir-D/homebrew-tap`) needs `Casks/firstcut.rb` pushed and a tap secret for the
  release workflow; then the `v0.1.0` tag.
- Open PRs #2 (ui), #3 (qa), #4 (core-store), #5 (consolidation) on GitHub are superseded by this merge.

### 0.5 Working practices

- **Read first:** this section, `AGENTS.md`, [build.md](docs/contracts/build.md), then the contract for
  whatever you touch. Contracts: [build](docs/contracts/build.md),
  [photo-meta](docs/contracts/photo-meta.md), [batching](docs/contracts/batching.md),
  [session-api](docs/contracts/session-api.md), [pipeline-api](docs/contracts/pipeline-api.md),
  [app-model](docs/contracts/app-model.md). Add a changelog line when a surface changes.
- **Local checks before every push** (until CI is strict): `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test` (all from `core/`),
  then `scripts/generate-project.sh` and the `xcodebuild … test` command above.
- **Seeing the app:** `scripts/build-app.sh --open`, then
  `osascript -e 'tell application "System Events" to tell process "Firstcut" to get {position, size} of window 1'`,
  `screencapture -x -o -R<x>,<y>,<w>,<h> docs/ui/shot-<what>.png`, and read the PNG back. Compare with the
  Finder reference in `docs/ui/` and §9. Close the app when done; a running copy gets killed by rebuilds
  and macOS reports "Firstcut quit unexpectedly" (`defaults write com.kathird.firstcut
  NSQuitAlwaysKeepsWindows -bool false` stops the reopen).
- **The app must only get folder access through `NSOpenPanel`.** A hosted test app reading `~/Documents`
  raises a TCC prompt that blocks forever; real-photo tests are opt-in behind
  `FIRSTCUT_ALLOW_PHOTO_TESTS=1` (`scripts/test-with-photos.sh`).
- **Test photos** are in `~/Documents/testing` (Canon R8 C-RAW, 4 games); never copy them into the repo.
- **Git:** work on `main`, `git pull` before you start, commit small, push right after committing.

### 0.6 Schedule

A priority order, not a calendar. Work the critical path (§0.7) top to bottom.

| Wave | Goal | What "done" means |
| --- | --- | --- |
| **1. Foundations** | Everything compiles and is tested | `cargo test` and `xcodebuild test` green on `main`; CI enforcing fmt, clippy, warnings-as-errors, swift-format |
| **2. Real data** | It runs on the test games | CR3 parser matches exiftool on all 2,880 files; `order()`+`batch()` measured ≥ 98% boundary F1 against visually checked ground truth; the app opens a real folder through the real `Session` |
| **3. Features** | Feature complete | Finish Cull end to end with undo, both rating modes, Settings, keymap editor, every §9 screen |
| **4. Polish & ship** | v0.1.0 | Liquid Glass pass, macOS 15 fallback, accessibility, README screenshots, `v*` tag published, cask live |

Waves 1–2 are partly done: the CR3 reader and a real-folder session exist and the Rust suite is green;
the Swift suite, ground truth and measurements are what remain.

### 0.7 The critical path

Ordered by what blocks what. Start at the top every session; do not skip ahead.

| # | Step | Status | Blocks |
| --- | --- | --- | --- |
| 1 | Make `xcodebuild test` compile and pass again (`RealRawDecodeTests.swift`, the `ImageProvider` WIP) | **next** | everything Swift |
| 2 | One `PhotoMeta`/`Batch`/`Session` type each; delete stand-ins (`CoreTypes.swift`, `PipelineMirror.swift`, renamed `SessionTypes`) and settle the parked duplicates (§0.3) | open | Swift work compiling against another area |
| 3 | CR3 parser verified against exiftool on all 2,880 files | done in `meta/cr3.rs` (re-run with `FIRSTCUT_CR3_FULL=1`) | every real measurement |
| 4 | Ground truth and F1 for `order()` + `batch()` | **open — needs a human looking at photos** | the filmstrip meaning anything |
| 5 | `Session`, XMP, undo, resume, rename reconciliation (REV-68) | partly done | ratings surviving a real session |
| 6 | Thumbnails, `visual_sig` from the Rust reference, T2 decode, priority scheduler | partly done (`ImageProvider`) | the zero-wait promise |
| 7 | `AppModel` on the real `Session` and real pipeline; viewer layer | wired in `AppEnvironment`; unverified | a usable app on real photos |
| 8 | Every §7.3 target measured and written down | open | any performance claim |
| 9 | Finish Cull, Settings, keymap editor, every screen | partly done | feature complete |
| 10 | Liquid Glass, macOS 15, accessibility, then the release | open | v0.1.0 |

### 0.8 Git and CI

- Everything lives on `main`. CI (`.github/workflows/ci.yml`) runs fmt/clippy/test and the Swift build on
  every push; if the Swift job reports "pending" for a long time it is macOS runner queueing.
- Release: pushing a `v*` tag runs the release workflow (zip + SHA-256 + cask bump). Rehearsed on a test
  tag; the real `v0.1.0` waits on the tap secret and README screenshots.

---

## 1. What Firstcut is

Firstcut is a **hyper-fast, manual photo culler for macOS on Apple Silicon**. After a shoot (e.g. a
football game producing ~500–1500 RAW files) you open the folder, Firstcut splits the shoot into
**batches** (one batch ≈ one burst / one moment of action), and you cull batch by batch with the
keyboard: arrow through the frames of a burst, rate the ones you want, move on. When every batch
has been seen, Firstcut asks what to do with everything you didn't keep.

### Goals

1. **Zero waiting.** Every photo you can reach with the arrow keys or the batch buttons is already
   decoded and on the GPU. No spinners, no progressive "blurry then sharp" loading, no throttling
   after 50 photos (the core Lightroom pain point this app exists to fix).
2. **Burst-aware batching** that is accurate for anything from slow continuous shooting to 40 fps
   electronic shutter bursts (the test set reaches ~11 fps; thresholds adapt to each burst's speed), using every signal in the files (time, shutter count, lens/exposure data,
   visual similarity) — not file names.
3. **Looks and feels like a first-party Apple app.** Finder's gallery view is the reference: large
   image, filmstrip underneath, unified toolbar, Liquid Glass. Always dark.
4. **Lightroom muscle memory.** Default shortcuts match Lightroom Classic; every shortcut is
   remappable.
5. **Non-destructive until you say so.** Ratings are written to XMP sidecars (Lightroom / Capture One
   compatible) and an app database. Files are only moved/deleted at the explicit end-of-cull step.

### Non-goals

- **No AI / ML auto-culling.** No "eyes closed" detection, no auto-rating. The only image analysis is
  a cheap perceptual hash used to find burst boundaries.
- No editing, develop settings, or export processing. Firstcut decides *what to keep*; editing
  happens in Lightroom/Capture One.
- No card ingest/import. Firstcut opens **one folder already on disk** containing the whole shoot.
- No multi-folder sessions (e.g. `100CANON` + `101CANON` as one session). One folder = one session.
- No Intel Macs, no iOS/iPadOS.
- No keyboard shortcut for zoom (pinch / click only).
- No manual batch editing (split/merge); batching is fully automatic.
- No network access, accounts, telemetry, or analytics. Firstcut works entirely offline.
- English only for v0.1 (strings still go through `String(localized:)` so localization is possible later).

---

## 2. Decisions (locked unless revisited)

| Area | Decision |
| --- | --- |
| Name | **Firstcut** (GitHub: `Kathir-D/Firstcut`, public) |
| Platform | macOS, Apple Silicon only (arm64) |
| Minimum OS | **macOS 15 Sequoia**. Real Liquid Glass (`glassEffect`) on macOS 26+, closest material fallback (`NSVisualEffectView` / `.ultraThinMaterial`) on 15 |
| UI | Swift 6, SwiftUI for chrome/settings + AppKit where needed for performance and exact native behavior (window, toolbar, key handling, filmstrip) |
| Image pipeline | Swift: ImageIO (embedded previews + thumbnails), Core Image `CIRAWFilter` (true RAW decode), Metal / IOSurface-backed layers for display |
| Core logic | **Rust** static library (`firstcut-core`) exposed to Swift via **UniFFI**: scanning, metadata parsing, ordering, batching, SQLite session DB, XMP read/write, file operations, undo log |
| Appearance | **Always dark** (forced `NSAppearance.darkAqua`), neutral gray photo background |
| Rating modes | Two modes, chosen in Settings: **Stars** and **Keep / Not keep** (see §6) |
| Rating scope | You can only rate photos in the batch you are currently in; to change another batch's photo, navigate to that batch |
| Batching | Fully automatic, no manual split/merge UI |
| Ordering | By capture time + sub-second + shutter count, **never by file name** (Canon `IMG_9999` → `IMG_0001` rollover) |
| Storage of ratings | XMP sidecars **and** app DB (DB = instant resume; XMP = interoperability) |
| Shortcuts | Lightroom Classic defaults, all remappable. Batch navigation: ⌘← / ⌘→ + toolbar ‹ › buttons; arrow keys are for photos only |
| Distribution | Personal Homebrew tap + GitHub Releases (ad-hoc signed `.zip`, curl install) + build from source, same setup as [Sonar](https://github.com/Kathir-D/Sonar#install). **No paid Apple Developer account** → no notarization (see §13) |
| License | **GPL-3.0** (copyleft: anyone who distributes a modified version must release its source under GPL-3.0 too) |
| Zoom | **Mouse/trackpad only, no keyboard shortcut**: pinch to zoom in/out; click a spot → 100% there (one step); click again → back to fit |
| Testing | **Canon only** (the only RAW files available). All other formats are implemented from their specs and must work, but are untested; the README says so |
| Test data | `~/Documents/testing` (never committed; see §12) |

---

## 3. Measured facts about the test data

Collected on the dev machine (Apple M1 Pro, 16 GB RAM, macOS 27) from `~/Documents/testing`.
These numbers shape the architecture — re-measure when anything changes.

| Fact | Value |
| --- | --- |
| Test set | 4 games, **2,880 CR3 files, 42 GB** (Game1JENKS 708, Gane2NC 529, Game3KC 920, Game4VRE 723) |
| Camera | Canon EOS R8, `Quality = CRAW` (compressed CR3), 6000×4000 (24 MP), ~15 MB/file |
| Embedded preview in each CR3 | **Full resolution 6000×4000 JPEG** |
| Decode embedded preview, full res, 1 thread (ImageIO) | **~166 ms / image** |
| Full RAW decode, 1 thread (ImageIO) | **~570 ms / image** |
| Drive mode tag | Always `Continuous, High+` → useless as a burst boundary signal |
| Sub-second time | `SubSecTimeOriginal` present, **10 ms resolution** |
| `ShutterCount` (Canon MakerNote) | Present and monotonic → reliable ordering + detects deleted frames |
| AF data | `AFAreaMode`, `AFPointsInFocus`, `AFAreaXPositions/YPositions` present → AF point overlay is possible |
| ImageIO metadata coverage | ImageIO exposes almost none of the Canon MakerNote → **Rust must parse CR3 MakerNotes itself** |
| Frame interval inside bursts (Game3KC) | ~0.16 s (≈6 fps) |
| Frame interval inside bursts (Game1JENKS) | **~0.09 s (≈11 fps)**, with 13 gaps of 40–60 ms. The fastest bursts in the set are at the end (`IMG_6117`–`IMG_6164`): 48 frames at 90 ms, broken by 0.23–0.77 s pauses where the shutter was released and pressed again during the same play |
| Shutter modes in Game1JENKS | Electronic (692), Electronic First Curtain (16); drive `Continuous, High+` / `High` / `Low` all appear |
| Gap histogram, Game3KC (919 gaps) | ≤0.2 s: 663 · 0.2–0.5 s: 24 · 0.5–2 s: 57 · 2–30 s: 101 · >30 s: 74 |
| File name rollover | Game4VRE ends at `IMG_9999`; rollover to `IMG_0001` within a shoot is possible |

**Implications:**
- The embedded preview *is* a full-resolution image, so the default view never needs a RAW decode
  and 100% zoom can use it instantly. True RAW decode becomes an optional "exact" toggle.
- 166 ms/image single-threaded is too slow to do on keypress but trivially fast to do *ahead of
  time* across 8–10 performance cores → the whole design is **prefetch everything reachable**.
- Game1JENKS is the **high-speed test case**. No file in the set is 25 ms apart (true 40 fps), so the
  thresholds must be derived from the local frame interval, not hard-coded to one speed.
- The ~80 gaps between 0.2 s and 2 s are the ambiguous zone where timing alone can't decide a
  burst boundary → visual similarity + exposure/lens signals decide those.

---

## 4. Architecture

```
┌──────────────────────────── Firstcut.app (Swift) ─────────────────────────────┐
│  UI layer (SwiftUI + AppKit)                                                   │
│   Window/Toolbar · Viewer · Filmstrip · Info panel · Grid · Compare · Settings │
│                         │ observes                                             │
│  App state (@Observable): Session, current batch/photo, ratings, settings      │
│                         │                                                      │
│  Image pipeline (Swift)                                                        │
│   PreviewDecoder (ImageIO) · RawDecoder (CIRAWFilter) · ThumbnailStore         │
│   CacheManager (tiers, memory budget) · Renderer (Metal / IOSurface layers)    │
│                         │ UniFFI (sync calls + async callbacks)                │
├─────────────────────────┼──────────────────────────────────────────────────────┤
│  firstcut-core (Rust static lib, arm64)                                        │
│   scan · metadata (TIFF/EXIF/CR3/ARW/NEF/... + MakerNotes) · order · batch     │
│   session DB (SQLite) · XMP sidecar read/write · file ops · undo log           │
└────────────────────────────────────────────────────────────────────────────────┘
```

**Why the split:** Swift owns everything Apple-accelerated (ImageIO hardware paths, Core Image RAW,
Metal, Liquid Glass). Rust owns the pure-logic, heavily-tested, performance-sensitive-but-not-GPU
parts (parsing 1500 headers in parallel, the batching algorithm, DB/XMP I/O) where its type system
and test tooling make correctness easy to prove against the test shoots.

### Repository layout (target)

```
Firstcut/
├── App/                      # Swift app target (Xcode project generated by XcodeGen)
│   ├── Sources/
│   │   ├── App/              # @main, AppDelegate, window + toolbar setup
│   │   ├── Session/          # Session model, navigation, rating actions, undo bridging
│   │   ├── Pipeline/         # decoders, CacheManager, ThumbnailStore, memory budget
│   │   ├── Render/           # Metal/IOSurface viewer layer, zoom/pan, overlays (AF, clipping)
│   │   ├── Views/            # Viewer, Filmstrip, InfoPanel, Grid, Compare, HUD, Finish sheet
│   │   ├── Settings/         # Settings window, keybinding editor
│   │   └── Input/            # Key command routing, remappable shortcut table
│   ├── Resources/            # Assets, default keymap JSON, Info.plist
│   └── Tests/                # XCTest / Swift Testing
├── core/                     # Rust workspace
│   ├── firstcut-core/        # library crate (UniFFI)
│   ├── firstcut-cli/         # dev CLI: scan/batch/benchmark a folder, dump JSON
│   └── Cargo.toml
├── scripts/                  # build-core.sh (→ XCFramework), build-app.sh (→ dist/Firstcut.app), bench scripts
├── Casks/                    # firstcut.rb, mirrored into Kathir-D/homebrew-tap
├── VERSION
├── tests/fixtures/           # ground-truth batch files (filenames only — no images)
├── .github/workflows/        # CI + release
├── project.yml               # XcodeGen spec (no hand-edited .pbxproj merge conflicts)
├── task.md
└── README.md
```

---

## 5. Batching (the heart of the app)

Batches are computed once when a folder is opened (and cached in the session DB). Target: **all
1,500 files batched in < 2 s** after the metadata scan.

### 5.1 Ordering

- [ ] Sort key: `(camera serial, DateTimeOriginal + SubSecTimeOriginal + OffsetTime, ShutterCount, FileNumber, file name)`.
- [ ] Never rely on file names; handle `IMG_9999 → IMG_0001` rollover and renamed files.
- [ ] If sub-seconds are missing, use `ShutterCount` (Canon), `ImageCount`/`SequenceNumber` (Sony),
      `ShutterCount` (Nikon) to order ties within the same second.
- [ ] Multiple bodies in one folder: order globally by time, but never put two different camera
      serials in the same batch.
- [ ] Files with no usable timestamp: fall back to file modification time and flag them in the log.

### 5.2 Signals per consecutive pair (frame *i−1* → *i*)

| Signal | Source | Use |
| --- | --- | --- |
| Δt (time gap) | capture time, 10 ms resolution | primary |
| Local frame interval | median Δt of neighbouring frames | makes Δt thresholds adaptive (6 fps vs 40 fps) |
| Shutter-count gap | MakerNote | a jump means frames were deleted in camera → weaker boundary evidence, not a boundary by itself |
| Focal length change | EXIF | large zoom change → likely new moment |
| Orientation change | EXIF | portrait ↔ landscape → strong boundary |
| Exposure change | shutter / aperture / ISO | big jumps suggest a new scene; small auto-ISO drift ignored |
| AF area / point jump | MakerNote | supporting evidence only |
| Visual difference | perceptual hash (dHash/pHash 64-bit) + tiny color histogram of the 256 px thumbnail | decides the ambiguous zone |

### 5.3 Algorithm (initial version, to be tuned against ground truth)

1. Compute Δt and a local frame interval `f` (rolling median of Δt among gaps < 0.5 s).
2. **Hard join** if Δt ≤ max(2.5·f, 0.25 s) and no orientation change → same burst.
3. **Hard split** if Δt > 2.0 s, or orientation changed, or camera serial changed.
4. **Ambiguous zone** (everything else): score = weighted sum of normalized visual distance,
   focal length change, exposure change, Δt/f. Split if score > threshold.
5. Post-pass: very short runs of singles between two bursts are allowed to stand as 1-photo batches
   (a single frame of a different moment is a real moment). Evaluate whether grouping consecutive
   singles within a few seconds improves the culling flow — decide from the test games.
6. Deterministic: same input → same batches (important for resume).

### 5.4 Tasks

- [ ] Implement signals + scoring in `firstcut-core::batch`, pure function
      `fn batch(photos: &[PhotoMeta], hashes: &[Option<PHash>]) -> Vec<Batch>`.
- [ ] Two-phase: produce **provisional batches from metadata alone** instantly, then refine the
      ambiguous boundaries once thumbnail hashes arrive (a few seconds later). The batch the user is
      currently in must never be re-split under them — only batches not yet visited can change.
- [ ] Build **ground truth** for all four test games: generate contact sheets per candidate batch
      with `firstcut-cli`, check visually that every frame belongs to the same burst, and record the
      true boundaries in `tests/fixtures/ground-truth/<game>.json` (file names only).
- [ ] Metrics: boundary precision/recall, number of wrongly merged bursts, number of wrongly split
      bursts. Target ≥ 98% boundary F1 on all four games; zero merges of clearly different plays.
- [ ] Regression test in CI that runs the batcher on committed **metadata dumps** (JSON of the
      extracted fields + hashes, no images) so CI doesn't need the 42 GB of RAW files.
- [ ] Use the end of Game1JENKS (`IMG_6117`–`IMG_6164`, ~11 fps with 0.2–0.8 s re-press pauses) as the
      high-speed / ambiguous-pause regression case. Decide from the photos whether those pauses are
      one play (one batch) or several.

---

## 6. Rating modes

Selected in **Settings → General → Rating mode**. Switching mode mid-session is allowed; existing
data is preserved and mapped (a keep ↔ 5 stars by default).

### 6.1 Stars mode

| Input | Meaning | Tier |
| --- | --- | --- |
| 5 or 4 stars | Full keep | **Keep** |
| 3 stars | Good | **Good** |
| 2 or 1 star | Maybe | **Maybe** |
| No rating | Not good (not reviewed or not wanted) | **Unrated** → handled at the end |
| X (reject flag) | Explicit reject | **Rejected** → handled at the end, same as unrated |
| P (pick flag) | Supported for Lightroom parity; independent of stars | (no effect on tier) |

- [ ] Filmstrip shows stars under/over each thumbnail (small, Finder-like), flags as badges.
- [ ] Viewer HUD shows the current photo's stars/flag/color label.

### 6.2 Keep / Not keep mode

- [ ] Every photo starts as **Not keep**.
- [ ] One configurable key (default **P**) toggles Keep ↔ Not keep on the current photo
      (pressing it on a keep turns it back to not keep).
- [ ] Filmstrip: **green ring** around keeps, **red ring** around not-keeps, for every frame of the
      current batch. The selected frame additionally gets the Finder-style rounded selection plate.
- [ ] XMP mapping (configurable): Keep → `xmp:Rating = 5` (default) or a color label; Not keep → no
      rating (or `xmp:Rating = -1` "rejected" only at the finish step if chosen).
  - Note: Lightroom does **not** read pick flags from XMP, so a keep must be stored as a rating or
    label to survive import.

### 6.3 Shared behavior

- [ ] Ratings can only be changed for photos in the **current batch**.
- [ ] **Auto-advance** after rating/flag: setting, off by default; toggled also with Caps Lock
      (Lightroom behavior) — configurable.
- [ ] Color labels 6–9 (red, yellow, green, blue) available in both modes.
- [ ] Every rating change is undoable (⌘Z / ⇧⌘Z), including across batches: undoing a change made in
      another batch first navigates to that batch and photo, then reverts it (so the rule "only rate
      in the current batch" still holds visibly).
- [ ] Each change writes to the DB immediately and to XMP on a debounced background queue
      (≤ 1 s), flushed on batch change and on quit. A crash never loses more than ~1 s.

---

## 7. Image pipeline & performance

The single most important property of the app: **navigation never waits for decoding.**

### 7.1 Cache tiers

| Tier | Content | Scope | Approx size (1,500 × 24 MP) |
| --- | --- | --- | --- |
| T0 | 256 px thumbnails (filmstrip, grid, perceptual hash input) | **Whole shoot**, generated on open | ~30–60 MB |
| T1 | Compressed embedded preview bytes (JPEG, as stored in the RAW) | As many batches as the RAM budget allows, nearest first | ~3–6 MB each |
| T2 | **Decoded display-resolution bitmaps** (fit to the viewer's pixel size, IOSurface/Metal textures) | **Previous + current + next batch, always**; extends further ahead/behind while under budget | ~10–25 MB each |
| T3 | Decoded full-resolution (6000×4000) bitmaps for 100% zoom | Current frame ±N in the current batch, only while zoomed / zoom-locked | ~96 MB each |
| T4 | True RAW decode (`CIRAWFilter`) | Only when "Exact RAW" is toggled | on demand + neighbours |

- [ ] **Memory budget**: default = 40% of physical RAM (≈6.4 GB on 16 GB), configurable in Settings
      → Performance. Respond to `DispatchSource` memory-pressure warnings by shedding T3 → T1 far
      batches → T2 beyond ±1 batch, never the current batch.
- [ ] **No lazy loading anywhere reachable**: nothing in the previous, current, or next batch is ever
      decoded on demand. If a cache miss does happen, it is a bug: log it in the debug HUD and count it
      in the stress test.
- [ ] **Priority scheduler** (not FIFO): current frame > rest of current batch > next batch >
      previous batch > further batches. Re-prioritize instantly on every navigation. Cancel work
      for batches that fell out of range.
- [ ] Decode concurrency = performance-core count; decode work runs at `.userInitiated`, thumbnail
      generation at `.utility` so it never competes with the current batch.
- [ ] Pre-upload decoded bitmaps to the GPU (IOSurface-backed) so display = pointer swap, < 1 frame.
- [ ] Re-decode T2 when the window/screen size changes (debounced), keeping old bitmaps visible
      until new ones are ready.

### 7.2 Image quality rules (don't make a grainy high-ISO shot look soft)

- [ ] Display at **native pixel scale**: T2 bitmaps are decoded at exactly the viewer's backing
      pixel size (Retina aware) — no double resampling, no GPU minification blur.
- [ ] Downscale with a high-quality filter (Lanczos / area average via ImageIO's DCT scaling +
      vImage) — never nearest/bilinear.
- [ ] Never re-encode to JPEG/HEIC for caching; cache decoded pixels or the camera's original
      compressed bytes only.
- [ ] Preserve color: honor the embedded ICC profile, render in the display's color space
      (Display P3), 8-bit is fine for previews; consider 10-bit for T3.
- [ ] **Benchmark task**: compare embedded preview vs `CIRAWFilter` decode on the high-ISO night
      shots in the test set at 100% and fit-to-screen; pick defaults from the result and document it
      here (grain structure, sharpness, noise reduction differences).

### 7.3 Performance targets (M1 Pro, 1,500-file shoot on internal SSD)

| Metric | Target |
| --- | --- |
| Folder open → first photo on screen | < 1 s |
| Metadata scan of all files (header reads only, parallel) | < 3 s |
| Provisional batches ready | < 3.5 s |
| All thumbnails + hashes | < 20 s, in background |
| Current + next + previous batch fully decoded (T2) | before the user can reach them |
| Arrow key → sharp photo on screen | ≤ 1 display frame (≤ 8 ms at 120 Hz) when cached, which must be always |
| Batch switch → sharp photo | ≤ 1 display frame |
| 100% zoom from embedded preview | < 150 ms first time, instant with zoom-lock prefetch |
| Idle memory after full cull of 1,500 photos | within budget, no leaks |

- [ ] Instrument with `os_signpost` + a hidden debug HUD (cache hits/misses, decode queue depth,
      memory by tier, frame times).
- [ ] Automated benchmark (`firstcut-cli bench` + XCTest perf tests) run against `~/Documents/testing`.
- [ ] Stress test: hold → for the entire shoot at key-repeat rate; zero cache misses in the
      current batch, no memory growth.

### 7.4 Metadata scan

- [ ] Read only file headers (CR3 `moov`/`CMT*` boxes, TIFF IFDs) with parallel `pread`, not whole
      files — target < 2 ms/file.
- [ ] Extract: capture time + sub-sec + offset, shutter count, file number, camera model/serial,
      lens, focal length, shutter/aperture/ISO/exposure comp, orientation, dimensions, AF area mode +
      AF points (for overlay), embedded preview offset/length (so Swift can read the JPEG bytes
      directly without re-parsing).

---

## 8. File format support

**Priority:** Canon first, Sony second, everything else must work.

**Testing scope:** only Canon files are available, so only Canon is tested. Every other format is
implemented from its published spec (and the ExifTool tag docs), relies on Apple's ImageIO for
decoding, and must fail gracefully (placeholder + reason) rather than crash if something is off. The
README states that only Canon has been tested.

| Brand | Extensions | Metadata parser | Preview / decode |
| --- | --- | --- | --- |
| Canon | `.CR3` (incl. C-RAW), `.CR2`, `.CRW` | Rust: ISO-BMFF + CMT1–4 IFDs + Canon MakerNote (CR3), TIFF + MakerNote (CR2), CIFF (CRW) | ImageIO embedded preview; CIRAWFilter |
| Sony | `.ARW`, `.SR2`, `.SRF` | Rust: TIFF + Sony MakerNote (incl. sequence/shot number, AF) | ImageIO; CIRAWFilter |
| Nikon | `.NEF`, `.NRW` | Rust: TIFF + Nikon MakerNote (shutter count may be encrypted — use time + sub-sec if unavailable) | ImageIO; CIRAWFilter |
| Fujifilm | `.RAF` | Rust: RAF header + embedded TIFF/EXIF + Fuji MakerNote | ImageIO; CIRAWFilter |
| Panasonic | `.RW2` | Rust: TIFF-variant + Panasonic MakerNote | ImageIO; CIRAWFilter |
| Olympus / OM System | `.ORF` | Rust: TIFF + Olympus MakerNote | ImageIO; CIRAWFilter |
| Pentax / Ricoh | `.PEF`, `.DNG` | Rust: TIFF + Pentax MakerNote | ImageIO; CIRAWFilter |
| Adobe / Leica / Apple ProRAW | `.DNG` | Rust: TIFF/DNG | ImageIO; CIRAWFilter |
| Leica | `.RWL`, `.DNG` | Rust: TIFF | ImageIO |
| Hasselblad | `.3FR`, `.FFF` | Rust: TIFF | ImageIO |
| Phase One | `.IIQ` | Rust: TIFF-variant | ImageIO |
| Samsung | `.SRW` | Rust: TIFF | ImageIO |
| Kodak / Epson / Mamiya / Leaf | `.DCR`, `.KDC`, `.ERF`, `.MEF`, `.MOS` | Rust: TIFF | ImageIO |
| GoPro | `.GPR` | Rust: TIFF/DNG | ImageIO |
| Sigma | `.X3F` | Rust: X3F directory + embedded JPEG | **Embedded JPEG only** — Apple doesn't decode Foveon and LibRaw dropped X3F; true RAW decode unavailable |
| Non-RAW | `.JPG/.JPEG`, `.HEIC/.HEIF`, `.HIF` (Canon HEIF), `.TIF/.TIFF`, `.PNG` | Rust: EXIF (JPEG APP1, HEIF `meta`) | ImageIO |

- [ ] **RAW + JPEG/HEIF pairs** with the same base name are treated as **one photo** (RAW is the
      primary); every action (rating, XMP, move, trash) applies to all members of the pair, incl.
      sidecars.
- [ ] Folders of only JPEG/HEIF must cull exactly like RAW folders.
- [ ] Generic fallback: if the Rust parser doesn't recognize a file, ask ImageIO for its properties
      from Swift and pass them in, so no supported-by-macOS file is ever skipped.
- [ ] Unsupported/corrupt files: shown in the filmstrip with a placeholder + reason, never block.
- [ ] Validate every Canon parser field against `exiftool` output on the test games
      (`firstcut-cli verify <folder>` diff report). Non-Canon parsers are checked with unit tests on
      hand-built header byte fixtures only.

---

## 9. UI / UX

Reference: **Finder's Gallery view** (large image, filmstrip underneath, unified toolbar) on macOS 26
with Liquid Glass. It should be indistinguishable from an Apple app. Always dark.

### 9.1 Window & toolbar

- [x] Single-window app (`NSWindow` + unified toolbar, full-size content view, glass toolbar items).
- [ ] Liquid Glass on macOS 26+: system toolbar glass, SwiftUI `glassEffect` / `GlassEffectContainer`
      for floating controls (HUD, overlays), AppKit `NSGlassEffectView` where views are AppKit. Use
      stock controls wherever possible so they pick up system styling automatically.
- [ ] Standard macOS menu bar (File, Edit, View, Photo, Window, Help) with every command listed and
      its current (remapped) shortcut shown.
- [x] Toolbar, left: **‹ › batch navigation buttons** (Previous batch / Next batch) in a glass
      capsule like Finder's back/forward.
- [x] Toolbar, title: batch position + file name, e.g. `Batch 12 of 148 — IMG_8231`.
- [x] Toolbar, right: view mode segmented control (Loupe · Grid · Compare), Info panel toggle,
      Finish Cull button.
- [ ] Full-screen support (hide chrome, filmstrip auto-hides at the bottom edge).
- [ ] macOS 15 fallback: same layout with `NSVisualEffectView` materials; verify visually on a
      macOS 15 VM / machine.

### 9.2 Main viewer (Loupe)

- [ ] Photo fills the area above the filmstrip, aspect-fit, rounded corners like Finder's gallery.
- [ ] Neutral dark gray background (configurable darkness).
- [ ] **Zoom (mouse/trackpad only, no keyboard shortcut)**:
  - **Pinch** to zoom in and out smoothly (trackpad magnify gesture), anchored at the pinch point.
  - **Click** a spot on the photo → jumps to **100% centered on that spot** (a single step, no
    multi-level zoom). **Click again** → back to fit.
  - When zoomed, drag (or two-finger scroll) pans. A click that turned into a drag must not toggle
    zoom.
  - Zoom in/out animates with the system spring, like Photos/Preview.
- [ ] **Zoom lock** (setting, also in the View menu, no default key): when on, arrowing to the next frame keeps the same zoom
      level and position, so sharpness can be compared across the burst. T3 prefetches neighbours
      while zoom-locked.
- [ ] **AF point overlay** (toggle): draw the in-focus AF point(s)/area from MakerNote data, mapped
      through orientation.
- [ ] **Clipping overlay** (toggle, J like Lightroom): highlight/shadow clipping.
- [ ] **Histogram** (toggle): computed from the T2 bitmap with vImage/Metal, shown in the info panel or
      as a floating glass HUD.

### 9.3 Filmstrip

- [x] Horizontal strip of **the current batch only**, Finder-style: thumbnails at their aspect
      ratio, selected frame on a rounded gray plate.
- [ ] Scrolls to keep the selection visible; smooth at 120 Hz with 60+ frames. (Keeps the
      selection visible: done. 120 Hz with 60+ frames: still to measure.)
- [x] Shows rating stars / flags / color labels (stars mode) or green/red rings (keep mode).
- [x] Clicking a thumbnail selects it. No drag-reordering.
- [x] Custom AppKit / layer-backed implementation (not SwiftUI `ScrollView`) for guaranteed
      performance.

### 9.4 Batch navigation

- [ ] ← / → move between photos **within** the batch. At the ends: stop (default) or continue into
      the next/previous batch (setting).
- [ ] Previous/next batch: toolbar ‹ › buttons + remappable shortcuts (defaults in §10). Arrow keys
      stay reserved for photos.
- [ ] Entering a batch selects its first photo (setting: or the last photo you viewed in it).
- [ ] Visited/complete state per batch stored in the session DB.

### 9.5 Info panel (I)

- [x] **I** toggles a right-side glass inspector (like Lightroom's info), with: file name, capture
      time (with sub-seconds), camera + serial, lens, focal length, shutter, aperture, ISO, exposure
      comp, metering, AF mode + points, drive mode, shutter count, dimensions, file size, folder
      path, rating/flag/label, batch number and position, histogram.
- [ ] Choose which fields are shown in Settings.

### 9.6 Other views (all optional, toggled from toolbar/shortcuts)

- [ ] **Grid (G)**: the current batch as a grid; ratings and rings visible; Return/E goes back to loupe.
- [ ] **Compare (C)**: 2-up / 3-up / 4-up of frames from the current batch with synchronized zoom and
      pan; arrows change the candidate frame.
- [ ] **Progress HUD**: batch X of Y, photos left, keeps / good / maybe counts, elapsed time. Glass
      capsule, auto-hides, toggle key.
- [ ] **Welcome window**: Open Folder…, recent sessions with progress (resume), drag-and-drop a
      folder onto the window/Dock icon.

### 9.7 Finish Cull flow

Triggered by the Finish button / shortcut, or offered automatically after the last batch.

- [ ] Summary sheet: totals per tier (Keep / Good / Maybe / Unrated / Rejected), batches not visited
      (warning if any).
- [ ] Actions for **unrated / not-keep / rejected** photos (choose one):
  - Leave files, mark them as rejected in XMP (Lightroom sees them as rejected)
  - Move to a subfolder next to the originals (name configurable, default `_Not kept`)
  - Move to Trash (recoverable in Finder)
  - Delete permanently (typed confirmation)
  - Do nothing
- [ ] Optional actions for **kept** photos:
  - Copy or move to a destination folder
  - Split into subfolders by tier (`5 Keep`, `3 Good`, `1 Maybe`) or by star count
  - Write a text/CSV list of kept file names
  - Reveal in Finder / open the folder in Lightroom (via `open -a`)
- [ ] Every file operation moves the whole group: RAW + paired JPEG/HEIF + `.xmp` sidecar.
- [ ] Dry-run preview list before executing; progress + cancel; final report.
- [ ] Moves are logged in the DB and undoable ("Undo Finish" restores files); Trash is recoverable via
      Finder; permanent delete is not undoable (said clearly in the UI).
- [ ] Safety: never overwrite existing files at the destination (suffix instead), check free space
      before copying, handle read-only volumes gracefully.

### 9.8 Settings window (native Settings scene, tabbed)

- [ ] **General**: rating mode, auto-advance, arrow behavior at batch ends, entering-batch
      behavior, default finish actions, confirmations.
- [ ] **Keyboard**: full shortcut editor — every command listed, record a new key, conflict
      detection, reset to Lightroom defaults, import/export keymap JSON.
- [ ] **Viewer**: background gray level, zoom lock default, AF overlay, clipping thresholds, info
      fields, HUD visibility, "Exact RAW" decode default.
- [ ] **Metadata**: write XMP on/off, what Keep maps to (rating/label), overwrite vs merge existing
      XMP, also write ratings into DNG files directly (off).
- [ ] **Performance**: memory budget slider, look-ahead batches, thumbnail size, decode threads
      (auto), debug HUD.

---

## 10. Default keyboard shortcuts (Lightroom Classic parity, all remappable)

| Action | Default |
| --- | --- |
| Previous / next photo in batch | ← / → |
| Previous / next batch | ⌘← / ⌘→ and toolbar ‹ › (alternates: `[` / `]`) |
| Set 1–5 stars | 1 … 5 |
| Clear stars | 0 |
| Set rating and advance | ⇧ + 1…5 (Lightroom behavior) |
| Pick flag / Keep toggle (keep mode) | P |
| Reject flag | X |
| Unflag | U |
| Toggle flag | ` |
| Color label red / yellow / green / blue | 6 / 7 / 8 / 9 |
| Toggle auto-advance | Caps Lock |
| Info panel | I |
| Grid / Loupe / Compare | G / E / C |
| Clipping overlay | J |
| AF point overlay | A |
| Progress HUD | H |
| Undo / Redo | ⌘Z / ⇧⌘Z |
| Open folder | ⌘O |
| Finish Cull | ⌘↩ |
| Full screen | ⌃⌘F |
| Settings | ⌘, |

- [ ] Zoom has **no keyboard shortcut** by design (pinch / click only, see §9.2).
- [ ] Keymap stored as JSON in Application Support; default keymap shipped in the bundle.
- [ ] Key handling via a single `NSEvent` local monitor / responder-chain router so no view steals keys;
      key repeat on arrows must be smooth.

---

## 11. Session, persistence & data safety

- [x] **Session DB**: SQLite (rusqlite, WAL mode) at
      `~/Library/Application Support/Firstcut/Sessions/<folder-id>.sqlite`, keyed by volume UUID +
      folder path + a fingerprint of file names/sizes, so a moved folder can be re-matched.
- [x] Tables: photos (path, group, metadata, hash), batches (members, visited, last position),
      ratings (current state), history (undo/redo log), file_ops (finish-step moves for undo).
- [ ] **Resume**: reopening a folder restores batches, ratings, current batch/photo, zoom lock, view.
- [ ] If the DB is missing but XMP sidecars exist, import ratings from XMP.
- [ ] Ordering and batches never depend on file names; add a synthetic rollover fixture
      (`IMG_9998`, `IMG_9999`, `IMG_0001`, `IMG_0002` with increasing capture times) since the test
      games don't cross 9999 inside one folder.
- [ ] Watch the folder with FSEvents: new files appear in new batches at the end (or are re-batched if
      not yet visited); deleted/renamed files are removed gracefully.
- [x] **XMP sidecars**: `<basename>.xmp` next to the RAW (Lightroom naming), writing `xmp:Rating`,
      `xmp:Label`; preserve any existing unknown XMP content (merge, don't clobber). JPEG/HEIF-only
      photos: sidecar as well (don't rewrite originals) — configurable. _(sidecar naming, merging
      and atomic writes done and tested; the "configurable" switch is Settings plumbing, wave 2:
      the old core-store charter (`git show 4d4e43d:docs/agents/archive/core-store.md`))_
- [x] Atomic writes (temp file + rename) for XMP and DB checkpoints.
- [x] Never modify original image files.

---

## 12. Testing

- [ ] **Test photos live in `~/Documents/testing`** (4 games, Canon R8 C-RAW) and are **never
      committed**. Tests locate them via `FIRSTCUT_TEST_PHOTOS` (default `~/Documents/testing`) and
      skip if absent.
- [ ] Rust unit tests: parsers (per format, fixtures = small header byte slices), ordering incl.
      rollover, batching on metadata dumps, XMP round-trip, DB migrations, file-op undo.
- [ ] Ground-truth batching tests (§5.4) on all four games.
- [ ] Swift tests: cache scheduler priorities, memory-budget eviction, keymap routing, rating actions
      + undo, finish-flow planning (dry-run output).
- [ ] Performance tests (§7.3) with baselines; fail CI on >10% regression (local perf job, since CI
      can't access the test photos).
- [ ] UI checks: screenshots of each view on macOS 26+ and 15 compared to Finder side by side.
- [ ] Rollover test with the synthetic fixture from §11.
- [ ] Batching review is done by **looking at the photos** (contact sheets per batch), not only by
      checking timestamps, for every boundary in the ambiguous zone.
- [ ] Manual QA checklist: full cull of each test game start to finish in both rating modes,
      including finish-step moves and undo.

---

## 13. Build, CI & distribution (no paid Apple Developer account)

### Build

- [x] `scripts/build-core.sh`: `cargo build --release --target aarch64-apple-darwin`, generate UniFFI
      Swift bindings, package `FirstcutCore.xcframework`.
- [x] XcodeGen `project.yml` → `Firstcut.xcodeproj` (generated, git-ignored), pre-build phase runs
      `build-core.sh` when Rust sources changed.
- [x] `scripts/build-app.sh`: build-core → xcodegen → `xcodebuild` Release → ad-hoc sign → `dist/Firstcut.app`.
- [ ] Swift 6 strict concurrency, warnings as errors in CI; `cargo clippy -D warnings`, `rustfmt`,
      `swift-format`. (strict concurrency + `clippy -D warnings` + `rustfmt` are enforced;
      `swift-format` runs advisory until the tree is formatted, then it becomes an error)

### CI (GitHub Actions, `macos-latest` arm64 runners)

- [x] On PR/push: Rust fmt/clippy/test, Swift build + unit tests, batching regression on metadata dumps
      (the regression tests are core-batch's; CI just runs `cargo test`, so they are included the
      moment they land).
- [x] On tag `v*`: `scripts/build-app.sh` builds Release, stamps `VERSION` + an increasing build number,
      **ad-hoc signs** (`codesign --force --sign - --timestamp=none`), zips `Firstcut-<version>.zip`
      (`ditto -c -k --keepParent`, so the executable bit and the bundle signature survive), attaches it
      to a GitHub Release with its SHA-256. Rehearsed on a test tag; see the old infra charter (`git show 4d4e43d:docs/agents/archive/infra.md`).

### Distribution

Same setup as [Sonar](https://github.com/Kathir-D/Sonar#install): three install paths in the README.

- [x] **Homebrew (recommended)**: add cask `Casks/firstcut.rb` to the existing tap
      [`Kathir-D/homebrew-tap`](https://github.com/Kathir-D/homebrew-tap) (it already holds `sonar.rb`;
      reuse that cask's structure):
      `brew tap Kathir-D/tap && brew trust Kathir-D/tap && brew install --cask firstcut`.
  - `brew trust` is required: Homebrew 7 refuses to load casks from an untrusted tap.
  - Not eligible for `homebrew/cask` (ad-hoc signed apps fail Gatekeeper assessment), so a personal
    tap is the route.
  - The cask clears the quarantine attribute in a `postflight` block (after Homebrew has verified
    the SHA-256), so the app opens with no Gatekeeper prompt. Homebrew's `--no-quarantine` flag was
    removed in 7.x.
  - Release workflow bumps the cask version + sha256 automatically; `brew upgrade --cask firstcut`
    updates it. The cask lives in this repo with a `REPLACE_WITH_RELEASE_SHA256` placeholder and the
    release stamps it, so the tap is never out of sync with an artifact.
  - Pushing the cask into the tap needs a `HOMEBREW_TAP_TOKEN` secret (the workflow's own
    `GITHUB_TOKEN` cannot write to another repository). Until that secret exists the release warns
    and attaches the cask to the release instead; the owner has to add the secret (REQ-infra-5).
- [x] **Direct download (curl)**: `curl -fLO …/Firstcut-<version>.zip`, unzip, move to
      `/Applications`. curl doesn't set quarantine so it usually opens without a prompt; a browser
      download does, and needs one **System Settings → Privacy & Security → Open Anyway**.
- [x] **Build from source**: `git clone`, `scripts/build-app.sh`, `open dist/Firstcut.app`; or open the
      generated Xcode project and run the `Firstcut` scheme. Verified from a clean clone.
- [ ] Optional later: in-app updates via **Sparkle** (EdDSA-signed appcast works without an Apple
      Developer account).
- [ ] Keep the app non-sandboxed; store folder bookmarks anyway in case of a future sandboxed build.

### Repository & README upkeep

- [x] README in the [awesome-readme](https://github.com/matiassingers/awesome-readme) style (header,
      badges, TOC, features, shortcuts, formats, install, build, architecture, roadmap, license).
- [x] README install section mirrors [Sonar](https://github.com/Kathir-D/Sonar#install): Homebrew,
      curl direct download, build from source.
- [x] README warns that only Canon has been tested.
- [x] GPL-3.0 `LICENSE`.
- [ ] Add screenshots and a short GIF of culling a burst to the README once the UI exists (M3/M7).
- [ ] Keep README in sync with reality: shortcut table, formats table, roadmap checkboxes, and the
      version/URL in the curl example on every release.
- [ ] GitHub issue templates: bug report (camera model, file format, macOS version, steps) and feature
      request; `CONTRIBUTING.md` once there is code to contribute to.
- [ ] Add `THIRD-PARTY-NOTICES.md` if any third-party code/crates with attribution requirements ship in
      the app (check every crate license is GPL-3.0 compatible).

---

## 14. Milestones

Each milestone ends with something runnable and measured.

### M0 — Repo & toolchain

- [x] Project name chosen (**Firstcut**), public repo `Kathir-D/Firstcut` created and pushed.
- [x] `task.md`, `README.md`, `AGENTS.md`, GPL-3.0 `LICENSE`, `.gitignore` (excludes all RAW
      extensions and `.xmp`), `.gitattributes`, `.editorconfig`.
- [x] Xcode 27 selected as the active developer directory (`xcode-select`).
- [x] Rust stable (1.98.1) installed via rustup; `~/.cargo/env` sourced from `~/.zprofile`.
- [x] `exiftool` installed (Homebrew) for checking the metadata parser.
- [x] Test data measured (§3).
- [x] Install `xcodegen` and `swift-format` (Homebrew).
- [x] Rust workspace skeleton (`core/`: `firstcut-core`, `firstcut-cli`, every module declared).
- [x] exiftool fixtures for all four games (`tests/fixtures/exiftool/`).
- [x] Shared Swift stand-in types (`App/Sources/Shared/CoreTypes.swift`).
- [x] UniFFI set up.
- [x] `project.yml` + placeholder app and three test targets (builds, tests pass).
- [x] The app calls one Rust function through UniFFI ("hello") — `FirstcutCoreBridge.greeting`,
      asserted by `App/Tests/Unit/Core/CoreBridgeTests.swift`; ui asked to show it in the
      placeholder window (REQ-infra-1).
- [x] `scripts/build-core.sh`, `scripts/build-app.sh`, `VERSION`.
- [x] CI skeleton (fmt, clippy, tests, app build).

### M1 — Core scan + order + batch (CLI)

- [ ] CR3 header parser complete (all fields in §7.4), verified against exiftool on all four games.
- [ ] `firstcut-cli batch <folder>` prints batches; `contact-sheet` output for visual review.
- [ ] Ground truth for all four games; batching F1 measured and ≥ 98%.

### M2 — Pipeline spike

- [ ] Minimal window that arrows through a whole game from the cache with zero misses.
- [ ] Memory budget + priority scheduler; embedded-preview vs RAW quality benchmark (§7.2).
- [ ] All §7.3 targets measured and recorded here.

### M3 — Core UX

- [ ] Finder-style window, toolbar, viewer, filmstrip, batch navigation.
- [ ] Both rating modes, XMP + DB, undo, resume, welcome window.

### M4 — Inspection tools

- [ ] Pinch/click zoom, zoom lock, AF overlay, info panel, histogram, clipping, grid, compare, HUD.

### M5 — Finish flow, settings, keymap editor

- [ ] Finish Cull flow (§9.7), all Settings tabs (§9.8), keyboard shortcut editor.

### M6 — Formats

- [ ] Sony next, then all others (spec-based, untested beyond Canon); RAW + JPEG/HEIF pairs;
      JPEG/HEIF-only folders.

### M7 — Polish

- [ ] Liquid Glass fidelity pass vs Finder, macOS 15 fallback, accessibility (VoiceOver labels,
      Reduce Transparency / Reduce Motion), app icon.

### M8 — Release

- [ ] CI release pipeline, zip release, `firstcut.rb` in the Homebrew tap, README screenshots, v0.1.0.
      (Pipeline, zip and cask done and rehearsed on a test tag; left open for the tap secret, the
      README screenshots from ui, and the v0.1.0 tag itself.)

---

## 15. Open questions

- [ ] Arrow at the end of a batch: stop, or roll into the next batch? (Planned: setting, default stop.)
- [ ] Should consecutive single frames a few seconds apart be grouped into one batch or stay 1-photo
      batches? Decide from ground truth on the test games.
- [x] ~~40 fps test shoot~~ → Game1JENKS covers high-speed bursts (~11 fps recorded, see §3).
- [x] ~~Sony samples~~ → not available; only Canon is tested (see §8).
- [ ] App icon design.
