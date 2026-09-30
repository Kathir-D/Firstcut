# Firstcut — Todo

> The single source of truth for what Firstcut is, the decisions already made, and every piece of
> work needed to ship it. Check items off as they land. When a decision changes, update the
> **Decisions** table first, then the tasks that depend on it.

**Status (2026-09-30): feature complete for v0.1.0 and green on CI.** Everything that can be built
and tested without a Mac, the test photos and a person looking at the screen is done. What is left is
in [§0.2](#02-left-for-the-owner) (only you can do it) and [§0.3](#03-other-open-work) (can wait until
after v0.1.0).

---

## 0. Where we are

> Every session starts by reading this section (the handoff in [§0.5](#05-handoff-where-the-last-session-stopped)
> first), then `AGENTS.md`.

### 0.1 Done

| Area | State |
| --- | --- |
| Rust core (`core/`) | CR3 reader (verified against exiftool on all 2,880 files), other formats from their specs, ordering, batching (metadata then visual signature), session DB, XMP read/write, Finish plan/execute/undo, rename reconciliation. 300+ tests, fmt and clippy clean |
| App | Welcome with recents and resume; loupe with pinch/click zoom, pan, zoom lock, AF and clipping overlays; filmstrip; grid; 2/3/4-up compare with synced zoom; info panel with histogram; progress HUD with the current rating; both rating modes; undo/redo; Finish Cull sheet (summary → options → dry run → typed confirmation for delete → report → undo); Settings (General, Keyboard with recorder/import/export, Viewer, Metadata, Performance); menus; error alerts; live folder watching; flush on quit |
| Pipeline | Concurrent decode engine with a priority queue (current photo, its batch, next, previous), cancellation when the user moves on, memory budget with LRU eviction, memory-pressure shedding, visual signatures in the background |
| CI | `ci.yml` (Xcode 16.4: Rust + Swift build and tests) green; `xcode26.yml` (shipping toolchain) green; `release.yml` rehearsed (ad-hoc signed zip + SHA-256 + cask stamping); `screenshots.yml` renders every screen on macOS 26 |
| Distribution | Homebrew cask template, curl and build-from-source paths in the README, GPL-3.0, third-party notices, issue templates |

History of how it got here: `git log`. The retired multi-agent process: `git show 4d4e43d:docs/review.md`,
`git show 4d4e43d:docs/agents/`.

### 0.2 Left for the owner

Only a person with the Mac, the test photos, the apps or the repo settings can do these. In order:

1. **Look at the app.** `scripts/build-app.sh --open`, cull part of a real game in both rating modes,
   run Finish on a *copy* and undo it. Compare each screen with Finder's gallery view (§9) and write
   down what looks wrong; the screenshots from the `Screenshots` workflow are a starting point.
2. **Ground truth for batching.** `firstcut contact-sheet --game <g>`, look at each ambiguous boundary
   (~210 across the four games), write `tests/fixtures/ground-truth/<g>.json` (file names only). This
   is what makes the ≥ 98% boundary-F1 claim measurable; an agent must not synthesise it.
3. **Game1JENKS re-press pauses** (`IMG_6117`–`IMG_6164`): one play or several? Decide by eye.
4. **Timings (§7.3)** on the M1 Pro with the photos: folder open → first photo, metadata scan,
   thumbnails, arrow → sharp photo, 100% zoom. `scripts/test-with-photos.sh` runs the opt-in tests.
5. **Lightroom / Capture One** read the sidecars (ratings and keep labels survive an import) on a copy
   of a few photos.
6. **Tap secret** on `Kathir-D/Firstcut`: `HOMEBREW_TAP_DEPLOY_KEY` (deploy key with write access on
   `Kathir-D/homebrew-tap`) or `HOMEBREW_TAP_TOKEN`. Do not add `Casks/firstcut.rb` to the tap by hand
   before the first release: its checksum is a placeholder until then and breaks the tap.
7. **README screenshots / GIF**, then **tag `v0.1.0`** (`git tag v0.1.0 && git push origin v0.1.0`):
   the release workflow builds, signs, zips, publishes and bumps the cask.

### 0.3 Other open work

Not blocking v0.1.0; an agent can do these.

- ~~**Keep threshold → core:**~~ **Done.** `AppModel` sends `session.setKeepThreshold(...)` after a
  folder opens and whenever `settings.keepThreshold` changes, so "Only 5 stars" now reaches Finish
  and `tierCounts`, not just the filmstrip. Wired through `SessionBackend` → `CoreSessionAPI` →
  `UniFFICoreSession` (the FFI export already existed; the app simply never called it). Two tests
  assert the call, and both fail if it is removed.
- **One definition per type (REV-72/73):** `Shared/CoreTypes.swift`, `Session/PipelineMirror.swift`
  and `Session/SessionTypes.swift` hand-mirror types the Rust core also defines.
- **Strict CI:** Swift warnings as errors and `swift-format` enforced (REV-12, REV-59); the tree is
  not formatted yet.
- **Pipeline refinements (§7.1, §7.2):** a display-sized (T2) decode per window size instead of the
  full image scaled by the layer; thumbnails at `.utility`; the embedded-preview vs `CIRAWFilter`
  quality comparison; the "Exact RAW" (T4) decode; `os_signpost` and a debug HUD.
- **Settings in the model but not shown** because nothing honours them yet: decode thread count,
  clipping thresholds, confirmation toggles.
- **Tests that need the photos or a person (§12):** performance baselines, the hold-→ stress test,
  a full manual QA pass, macOS 15 (only 26 is tested).
- **Contracts** in `docs/contracts/` predate the merge (pipeline-api.md still describes an IOSurface
  layer).
- **Performance plan (§7.5):** the research is done; the agent-side items there (CR3 full-size JPEG
  location, force-decoded display-space bitmaps, DCT-scaled T2, first-photo fast path, signposts,
  the `bench` extensions) can be built before the owner measures.
- **Later features (§16), after v0.1.0:** manual batch split/merge; simple batch edits with presets,
  importable from Lightroom (`.xmp`, `.lrtemplate`), Capture One (`.costyle`) and others.
- **Later:** Sparkle updates, folder bookmarks for a sandboxed build.

### 0.4 Working practices

- **Read first:** this section, `AGENTS.md`, [build.md](docs/contracts/build.md), then the contract for
  whatever you touch. Contracts: [build](docs/contracts/build.md),
  [photo-meta](docs/contracts/photo-meta.md), [batching](docs/contracts/batching.md),
  [session-api](docs/contracts/session-api.md), [pipeline-api](docs/contracts/pipeline-api.md),
  [app-model](docs/contracts/app-model.md). Add a changelog line when a surface changes.
- **A task is not done until something asserts it, and a performance number is not done until it is
  measured.**
- **Local checks before every push:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test` (all from `core/`), then `scripts/generate-project.sh` and `xcodebuild … test`.
- **Seeing the app:** `scripts/build-app.sh --open`, then `screencapture` and read the PNG back; or run
  the `Screenshots` workflow. The launch flags `-FirstcutMockShoot 1`, `-FirstcutFolder <path>`,
  `-FirstcutViewMode grid|compare2|…`, `-FirstcutFinish 1`, `-FirstcutSettings 1` and
  `-FirstcutSnapshot <png>` put the app in any screen. Close the app before rebuilding
  (`defaults write com.kathird.firstcut NSQuitAlwaysKeepsWindows -bool false` stops the reopen).
- **Folder access only through `NSOpenPanel`.** A hosted test reading `~/Documents` raises a TCC prompt
  that blocks forever; real-photo tests are opt-in behind `FIRSTCUT_ALLOW_PHOTO_TESTS=1`.
- **Test photos** are in `~/Documents/testing` (Canon R8 C-RAW, 4 games); never copy them into the repo.
- **Git:** one agent, work on `main` (or a short branch merged straight in), commit small, push right
  after committing, never force-push. CI runs on every push; release on a `v*` tag.

### 0.5 Handoff: where the last session stopped

> **Keep this current.** Any agent that stops mid-task (rate limit, context limit, end of session)
> rewrites this block in the same commit as its last change, so the next agent can pick up without the
> chat. Newest state only; history is `git log`.

**As of 2026-09-30 (agent session on Kathir's Mac, not the cloud container):**

- **The blockers in the previous handoff are gone: this session is on the real Mac.** `cargo`,
  `swift`, `xcodebuild`, `xcodegen`, `swift-format` and `exiftool` are all present, and the four
  games are in `~/Documents/testing`. So the Swift changes below can be built and tested locally
  instead of waiting for `ci.yml`, the perf rows in §7.5 can be measured rather than estimated, and
  the app can be launched and screenshotted. Baseline before this session's work: `cargo fmt`
  clean, `cargo clippy --all-targets -D warnings` clean, 343 Rust tests green, 231 Swift unit
  tests + 21 integration tests green.
- **Fixed (todo.md §0.3, the first item of the previous "Next, in order"):** the keep threshold now
  reaches the core. `AppModel` calls `session.setKeepThreshold(settings.keepThreshold)` after a
  folder opens and again whenever the setting changes, so "Only 5 stars" changes what Finish keeps
  and the summary sheet's tier counts, not only the filmstrip. The FFI export existed; the app
  never called it. New seam method on `SessionBackend` and `CoreSessionAPI`, implemented in
  `UniFFICoreSession` and recorded by `MockSession`. Two tests in `SessionTests` fail if either
  call is removed (checked by mutation).
- **Fixed (todo.md §0.5 item 2):** the thumbnail/preview cache no longer flickers away when the
  watched folder changes. `ImageProvider.open` used to call `engine.reset()` unconditionally, so a
  single file landing in (or leaving) the open folder blanked the filmstrip and re-decoded the whole
  shoot at ~300 ms per CR3 (todo.md §7.1) — several minutes for Game1JENKS — while the user was in
  the middle of rating it. `open` now distinguishes the two cases: a **different** folder resets
  (ids are hashes of file names, so a stale entry would show the wrong photograph), while the
  **same** folder with a different file set reconciles — it keeps every entry whose id is still
  there and whose file is still the same size, and drops the rest. A decode in flight is checked
  against both the epoch and the file's current size, so a file replaced under the same name cannot
  be served as the old one. Two new tests; the "added a photo" one fails if the reconcile is undone.

- **Next, in order:**
  1. ~~Wire the keep threshold from the app.~~ Done above.
  2. ~~Check the cache on a folder change.~~ Done above.
  3. §7.5 work items: the CR3 full-size JPEG byte range next to `PRVW` (and the wrong "full-size"
     doc comment on `PRVW` in `meta/cr3.rs`), `firstcut bench --folder`, `os_signpost` names. Then
     run `firstcut bench --folder` **on the photos** and write the rows into
     `docs/qa/perf-baselines.md` — the measurement earlier sessions could not make. All of this from
     the **CLI in a shell**, never a GUI test host, so nothing raises a `~/Documents` consent prompt
     with nobody there to answer it.
  4. §7.5: one shared 256 px decode for the filmstrip and the visual signature
     (`VisualSigWorker` decodes the file URL itself, so every photo is decoded twice today), and
     the display image decoded from the JPEG byte range rather than the CR3 URL.
  5. Known and left from the bug review: sidecar ratings are imported only when a folder's session
     is first created, so a Lightroom-rated second card copied into an open shoot is not imported; a
     different file saved under a known name (same path) inherits that name's rating.
  6. UI polish from screenshots the app can now be launched to take.
- **Fixed — the ~/Documents TCC hang, and the timeout tool that made it findable.** `xcodebuild test`
  on this machine was hanging indefinitely (no prompt, no failure, no output), which is the worst
  failure mode there is. Causes, all of them now gone:
  - `FixturePhotos.fixtureURL` asked the bundle for a `tests/fixtures/exiftool` **subdirectory**. But
    `project.yml` copies each fixture folder as a *folder reference*, so the bundle holds a flat
    `exiftool/`. The nested lookup never matched, so it fell through to the repository — inside
    `~/Documents` — and the GUI test host raised a TCC consent prompt and waited forever. It now tries
    the layouts that actually exist, and its repository fallback is off in any test process.
  - `TestEnvironment.testPhotos` was a `static let` that `stat`ed `~/Documents/testing` on first
    access **regardless of the opt-in**. The `FIRSTCUT_ALLOW_PHOTO_TESTS` gate covered the tests but
    not the lookup, so even `testPhotoDiscoveryIsOptional` triggered it. It now returns nil without
    touching the filesystem unless the opt-in is set.
  - `FirstcutCoreBridge.generatedBindingsSource` read the generated bindings from the repository for
    three tests. `scripts/build-core.sh` now also writes them as `FirstcutCore.bindings.txt` and
    `project.yml` bundles that into each test target (Xcode silently refuses to copy a `.swift` file
    through Copy Bundle Resources).
  - `TestEnvironment.repositoryRoot` walked up looking for `project.yml`, i.e. stat'ed inside
    `~/Documents` just to compute a path for a message. It is now pure path arithmetic.
  - `Fixtures.bundled` no longer falls back to the repository at all; it returns nil and the test
    fails with a message, which beats hanging.

  `scripts/with-timeout.sh <seconds> <command>` now wraps every build and test run, so a hang is a
  124 with a diagnosis instead of a frozen shell. It found the above in minutes. The full suite is
  235 unit + 24 integration + 3 performance tests, green, in about 4 seconds.
- **Fixed (§7.5 fact 1, the CR3's three JPEGs).** Measured, not assumed, on `Game1JENKS/IMG_3181.CR3`
  and `IMG_6117.CR3`: a CR3 carries **160×120** (`THMB`), **1620×1080** (`PRVW`) and **6000×4000**
  (the first image track's sample). The old doc comment called `PRVW` "full-size", which it is not.
  - The core now reads the sample table (`stsz` + `stco`/`co64` under `trak/mdia/minf/stbl`) and
    reports the full-resolution JPEG as `PhotoMeta.full_preview`, through the FFI to Swift's
    `PhotoMeta.fullPreview`. **Every image track's `stsd` says `CRAW`**, including the one whose sample
    is a JPEG, and all three are `vide`, so neither the codec nor the handler type identifies it —
    the sample's own `FF D8` and `SOF` do, and the largest JPEG wins.
  - **Display decodes now read that byte range** instead of handing the CR3 to ImageIO, so ImageIO
    never parses the container or picks an image. `byteRangeDecodes` counts the ones served this way;
    `decodeFull(url:)` stays as the fallback for a file with no reported range.
  - A test asserts the range path and the container path give **identical pixels**, and that partial
    or out-of-bounds ranges return nil (so a stale range falls back instead of showing a torn image).
  - `tests/cr3_exiftool.rs` gained a test that all three JPEGs are found on every sampled photo, that
    the bytes at each range really are a JPEG, and that `PRVW` is the 1620×1080 one — the fact that
    has been got wrong twice.
- **Still the owner's, for the reason that it is a judgement call, not for lack of a machine:** the
  ground truth in `tests/fixtures/ground-truth/` (an agent must not synthesise it), the
  Game1JENKS re-press decision, the "pick by eye" embedded-preview vs `CIRAWFilter` choice at
  16", and the visual sign-off against Finder.

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
- No editing, develop settings, or export processing **in v0.1**. Firstcut decides *what to keep*;
  editing happens in Lightroom/Capture One. Simple preset-based batch edits are a planned later
  feature ([§16.2](#162-batch-edits-with-importable-presets)).
- No card ingest/import. Firstcut opens **one folder already on disk** containing the whole shoot.
- No multi-folder sessions (e.g. `100CANON` + `101CANON` as one session). One folder = one session.
- No Intel Macs, no iOS/iPadOS.
- No keyboard shortcut for zoom (pinch / click only).
- No manual batch editing (split/merge) **in v0.1**; batching is fully automatic. Manual split/merge
  is a planned later feature ([§16.1](#161-manual-batch-editing-split--merge)).
- No network access, accounts, telemetry, or analytics. Firstcut works entirely offline.
- English only for v0.1 (strings still go through `String(localized:)` so localization is possible later).

---

## 2. Decisions (locked unless revisited)

| Area | Decision |
| --- | --- |
| Name | **Firstcut** (GitHub: `Kathir-D/Firstcut`, public) |
| Platform | macOS, Apple Silicon only (arm64) |
| Minimum OS | **macOS 15 Sequoia**, best effort. Real Liquid Glass (`glassEffect`) on macOS 26+, closest material fallback (`NSVisualEffectView` / `.ultraThinMaterial`) on 15. **Only macOS 26+ is tested** (no macOS 15 machine or VM); the README says so. Keep the fallback code paths working as well as possible without a test machine. Apple Silicon only, never Intel |
| UI | Swift 6, SwiftUI for chrome/settings + AppKit where needed for performance and exact native behavior (window, toolbar, key handling, filmstrip) |
| Image pipeline | Swift: ImageIO (embedded previews + thumbnails), Core Image `CIRAWFilter` (true RAW decode), Metal / IOSurface-backed layers for display |
| Core logic | **Rust** static library (`firstcut-core`) exposed to Swift via **UniFFI**: scanning, metadata parsing, ordering, batching, SQLite session DB, XMP read/write, file operations, undo log |
| Appearance | **Always dark** (forced `NSAppearance.darkAqua`), neutral gray photo background |
| Rating modes | Two modes, chosen in Settings: **Stars** and **Keep / Not keep** (see §6) |
| Rating scope | You can only rate photos in the batch you are currently in; to change another batch's photo, navigate to that batch |
| Batching | Fully automatic, no manual split/merge UI |
| Ordering | By capture time + sub-second + shutter count, **never by file name** (Canon `IMG_9999` → `IMG_0001` rollover). Across several bodies, shots interleave in true time order; camera serial only breaks exact ties and forces a batch split (decided 2026-09-29, REV-65) |
| Arrow at a batch end | **Setting, default: roll into the next batch** (the arrow on the last photo moves to the first photo of the next batch; with the setting off it stops). Decided 2026-09-29 |
| Single frames a few seconds apart | **Grouped into one batch**, not 1-photo batches. Exact window to be tuned against ground truth. Decided 2026-09-29 |
| App icon | Burst of five frames fading from outline (back) to a photo (front), on a cyan-to-pink gradient matching Sonar. Source: `logo/firstcut-icon.svg` (+ `firstcut-icon-small.svg` for 16/32 px); PNGs in `App/Resources/Assets.xcassets/AppIcon.appiconset/`. Decided 2026-09-29 |
| CI toolchain | CI keeps Xcode 16.2 as the minimum-toolchain check; local Xcode 27 covers the shipping toolchain (REV-58, decided 2026-09-29) |
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
├── logo/                     # app icon source SVGs + 512/1024 PNGs (README uses icon-512.png)
├── Casks/                    # firstcut.rb, mirrored into Kathir-D/homebrew-tap
├── VERSION
├── tests/fixtures/           # ground-truth batch files (filenames only — no images)
├── .github/workflows/        # CI + release
├── project.yml               # XcodeGen spec (no hand-edited .pbxproj merge conflicts)
├── todo.md
└── README.md
```

---

## 5. Batching (the heart of the app)

Batches are computed once when a folder is opened (and cached in the session DB). Target: **all
1,500 files batched in < 2 s** after the metadata scan.

### 5.1 Ordering

- [x] Sort key: `(DateTimeOriginal + SubSecTimeOriginal + OffsetTime, ShutterCount, camera serial, FileNumber, file name)`.
      Time first: several bodies interleave in true time order (decided 2026-09-29, REV-65; the old key put
      camera serial first).
- [x] Never rely on file names; handle `IMG_9999 → IMG_0001` rollover and renamed files.
- [x] If sub-seconds are missing, use `ShutterCount` (Canon), `ImageCount`/`SequenceNumber` (Sony),
      `ShutterCount` (Nikon) to order ties within the same second.
- [x] Multiple bodies in one folder: order globally by time, but never put two different camera
      serials in the same batch.
- [x] Files with no usable timestamp: fall back to file modification time and flag them in the log.

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

- [x] Implement signals + scoring in `firstcut-core::batch`, pure function
      `fn batch(photos: &[PhotoMeta], hashes: &[Option<PHash>]) -> Vec<Batch>`.
- [x] Two-phase: produce **provisional batches from metadata alone** instantly, then refine the
      ambiguous boundaries once thumbnail hashes arrive (a few seconds later). The batch the user is
      currently in must never be re-split under them — only batches not yet visited can change.
- [ ] Build **ground truth** for all four test games: generate contact sheets per candidate batch
      with `firstcut-cli`, check visually that every frame belongs to the same burst, and record the
      true boundaries in `tests/fixtures/ground-truth/<game>.json` (file names only).
- [ ] Metrics: boundary precision/recall, number of wrongly merged bursts, number of wrongly split
      bursts. Target ≥ 98% boundary F1 on all four games; zero merges of clearly different plays.
- [x] Regression test in CI that runs the batcher on committed **metadata dumps** (JSON of the
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

- [x] Filmstrip shows stars under/over each thumbnail (small, Finder-like), flags as badges.
- [x] Viewer HUD shows the current photo's stars/flag/color label.

### 6.2 Keep / Not keep mode

- [x] Every photo starts as **Not keep**.
- [x] One configurable key (default **P**) toggles Keep ↔ Not keep on the current photo
      (pressing it on a keep turns it back to not keep).
- [x] Filmstrip: **green ring** around keeps, **red ring** around not-keeps, for every frame of the
      current batch. The selected frame additionally gets the Finder-style rounded selection plate.
- [x] XMP mapping (configurable): Keep → `xmp:Rating = 5` (default) or a color label; Not keep → no
      rating (or `xmp:Rating = -1` "rejected" only at the finish step if chosen).
  - Note: Lightroom does **not** read pick flags from XMP, so a keep must be stored as a rating or
    label to survive import.

### 6.3 Shared behavior

- [x] Ratings can only be changed for photos in the **current batch**.
- [x] **Auto-advance** after rating/flag: setting, off by default; toggled also with Caps Lock
      (Lightroom behavior) — configurable.
- [x] Color labels 6–9 (red, yellow, green, blue) available in both modes.
- [x] Every rating change is undoable (⌘Z / ⇧⌘Z), including across batches: undoing a change made in
      another batch first navigates to that batch and photo, then reverts it (so the rule "only rate
      in the current batch" still holds visibly).
- [x] Each change writes to the DB immediately and to XMP on a debounced background queue
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

- [x] **Memory budget**: default = 40% of physical RAM (≈6.4 GB on 16 GB), configurable in Settings
      → Performance. Respond to `DispatchSource` memory-pressure warnings by shedding T3 → T1 far
      batches → T2 beyond ±1 batch, never the current batch. *Built:* LRU eviction to the budget;
      a pressure warning sheds everything outside the focus set (current photo, its batch, next and
      previous windows).
- [ ] **No lazy loading anywhere reachable**: nothing in the previous, current, or next batch is ever
      decoded on demand. If a cache miss does happen, it is a bug: log it in the debug HUD and count it
      in the stress test.
- [x] **Priority scheduler** (not FIFO): current frame > rest of current batch > next batch >
      previous batch > further batches. Re-prioritize instantly on every navigation. Cancel work
      for batches that fell out of range. *Built:* pending work is re-ranked or dropped on every
      move, and decodes that land after the folder changed are discarded.
- [ ] Decode concurrency = performance-core count; decode work runs at `.userInitiated`, thumbnail
      generation at `.utility` so it never competes with the current batch. *Partly:* 4 concurrent
      decodes at `.userInitiated`, thumbnails ranked below the current batch but on the same QoS.
- [ ] Pre-upload decoded bitmaps to the GPU (IOSurface-backed) so display = pointer swap, < 1 frame.
- [ ] Re-decode T2 when the window/screen size changes (debounced), keeping old bitmaps visible
      until new ones are ready.

### 7.2 Image quality rules (don't make a grainy high-ISO shot look soft)

- [ ] Display at **native pixel scale**: T2 bitmaps are decoded at exactly the viewer's backing
      pixel size (Retina aware) — no double resampling, no GPU minification blur.
- [ ] Downscale with a high-quality filter (Lanczos / area average via ImageIO's DCT scaling +
      vImage) — never nearest/bilinear.
- [x] Never re-encode to JPEG/HEIC for caching; cache decoded pixels or the camera's original
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

**Measured so far (2026-09-30, and only this):** `order()` + `batch()` on the metadata of a whole
shoot take **0.4 ms for 1,500 photos** (release build, Linux x86_64, `firstcut bench`; the target is
2 s). That is one step of "provisional batches ready", not the metadata scan, the decode or any of the
interactive targets above, none of which has been measured.

- [ ] Instrument with `os_signpost` + a hidden debug HUD (cache hits/misses, decode queue depth,
      memory by tier, frame times).
- [ ] Automated benchmark (`firstcut-cli bench` + XCTest perf tests) run against `~/Documents/testing`.
- [ ] Stress test: hold → for the entire shoot at key-repeat rate; zero cache misses in the
      current batch, no memory growth.

### 7.4 Metadata scan

- [x] Read only file headers (CR3 `moov`/`CMT*` boxes, TIFF IFDs), several files in parallel, not
      whole files — target < 2 ms/file (not yet measured on the photos).
- [x] Extract: capture time + sub-sec + offset, shutter count, file number, camera model/serial,
      lens, focal length, shutter/aperture/ISO/exposure comp, orientation, dimensions, AF area mode +
      AF points (for overlay), embedded preview offset/length (so Swift can read the JPEG bytes
      directly without re-parsing).

### 7.5 How the targets will be met (research, 2026-09-30)

> This is a plan, not a result. Every number below marked *expect* is arithmetic from §3 or from
> the sources listed, and **does not count as a measurement** (AGENTS.md). A target is met only when
> a row in `docs/qa/perf-baselines.md` says so. `App/Sources/Pipeline/ImageProvider.swift` and
> `AppModel.swift` were not read for this research. Items that say "check" need someone to read them
> against this plan.

**Three facts that shape everything:**

1. **A CR3 carries three JPEGs.** They are `THMB` at 160×120, `PRVW` at **1620×1080**, and the
   full 6000×4000 JPEG in the first `trak` (Laurent Clévy's CR3 description, `lclevy/canon_cr3`).
   The core's `PhotoMeta.preview` reads `PRVW` (`meta/cr3.rs`), whose doc comment wrongly calls it
   "full-size". The 166 ms in §3 is the 6000×4000 JPEG. The 1620×1080 one has 7% of the pixels.
   *Expect* ~10–20 ms to decode it; measure.
2. **ImageIO decodes lazily.** `CGImageSourceCreateImageAtIndex` and the thumbnail call return a
   `CGImage` whose pixels are decoded on first draw, unless `kCGImageSourceShouldCacheImmediately`
   is set. On first draw means inside the Core Animation commit on the main thread, which is exactly
   the 8 ms budget. A cache of lazy images is a cache of file handles.
3. **JPEG decodes can scale in the DCT.** With `kCGImageSourceThumbnailMaxPixelSize` (and
   `kCGImageSourceCreateThumbnailFromImageAlways`), ImageIO can decode a JPEG at 1/2, 1/4 or 1/8
   scale, doing roughly that fraction of the work. It can only do so when the target is at or below
   the scaled size. A 6000-px JPEG can be decoded at 3000 px cheaply; at 3456 px (a full-screen 16"
   viewer) it needs a full decode and a downscale.

**Per target:**

| Target | How | *Expect* | How it is measured |
| --- | --- | --- | --- |
| Open → first photo < 1 s | Do not wait for the whole scan. Header-read the resume photo (or the first by name) first, decode its `PRVW` from the byte range (`CGImageSourceCreateWithData` on the bytes, not the CR3 URL), and show it. The full scan, batching and T2 decode follow in parallel, and T2 replaces the `PRVW` when it lands. | One header read + one 1620 px decode ≈ tens of ms | Signpost from the open panel returning to the first CA commit with an image |
| Metadata scan < 3 s | Already header-only and multi-threaded (`meta::parallel_map`). Keep one bounded `pread` per file and do not pull `PRVW` bytes into the head read. Measure **cold** (`sudo purge` first) as well as warm, since a freshly copied card is often warm. | 2 ms/file × 1,500 ÷ 8 threads < 1 s | Extend `firstcut bench` to run the scan on a folder |
| Provisional batches < 3.5 s | Scan + `order()` + `batch()` (0.4 ms, measured) + the DB insert. Write the photos in one transaction with a prepared statement. | Dominated by the scan | `firstcut bench` prints each phase |
| Thumbnails + hashes < 20 s | 75 photos/s. Decode `PRVW` bytes to 256 px (DCT 1/4 → 405 px, then downscale), never the full JPEG or the RAW. Make one 256 px decode feed both the filmstrip (T0) and `VisualSigWorker`. Check whether they decode twice today: `VisualSigWorker` calls `DecodeEngine.decodeThumbnail(url:)` on the file URL. Keep this work at `.utility`, off the performance cores' queue. | A few ms per photo on 3 threads → well under 20 s | Signpost per chunk; total in the perf suite |
| T2 ready before the user can reach it | Decode the **full-size** JPEG by byte range (a new core field for the `trak` JPEG, next to `PRVW`) at the viewer's backing-pixel size, with `ShouldCacheImmediately`. Use a native 32-bit BGRA layout (`noneSkipFirst \| byteOrder32Little`), so Core Animation does not convert it. On a 14" viewer (≤ 3000 px) that is the cheap DCT-1/2 path. On a larger viewer, choose by eye (§7.2): decode the full image and downscale with vImage, or accept 3000 px. | 1/2-scale ≈ ¼ of 166 ms ≈ 40–50 ms per photo; 4–8 in flight ≫ key-repeat rate | `focusMisses` = 0 in the hold-→ stress test |
| Arrow → sharp photo ≤ 8 ms | Only a pointer swap on the main thread: a cached, already-decoded bitmap assigned to `layer.contents`. Make sure nothing on that path decodes, color-converts, resizes or touches SQLite. Write the rating off the main thread. If the commit is still too slow (it copies up to ~24 MB), make the bitmap IOSurface-backed and set `contents` to the `IOSurfaceRef`. That works on macOS: Chromium's `ca_renderer_layer_tree.mm` does it. The claim in `CGImageViewerHost.swift` (REV-38) that it cannot is wrong. Only do this if the measurement asks for it. | Assignment < 1 ms; commit to be measured | Signpost from `keyDown` to the next presented frame (`NSView.displayLink` / `CADisplayLink`), p50/p95/p99, `XCTOSSignpostMetric` |
| Batch switch ≤ 1 frame | Same path; the next/previous batch is already in T2 by the priority rules | — | Same signpost for `⌘→` |
| 100% zoom < 150 ms first time | The full decode is ~166 ms on one thread, so the first 100% needs a head start. On click, upscale T2 at once, so the zoom responds in one frame. Then start the full-resolution decode (T3). Also decode T3 speculatively for the current photo once the user has stayed on it for ~300 ms. With zoom lock, decode T3 ahead for the next photos in the batch. ImageIO cannot decode one JPEG on several threads or decode just a region, so speculation is the only lever. | Zoom feels instant; the sharp 100% lands ≤ 166 ms after the click, 0 ms when speculated | Signpost click → T3 on screen |
| Memory within budget, no leaks | 3000×2000×4 B = 24 MB per T2. Three batches of up to ~50 frames (Game1JENKS bursts) ≈ 3.6 GB, inside the 6.4 GB default. Hold T1 (compressed bytes, 1–6 MB) for further batches instead of T2. Keep only ±1 T3 (96 MB each). | Peak ≈ 4 GB on the worst burst | `footprint`/`phys_footprint` sampled in the stress test; `leaks` at the end |

**Work items (agent-side unless marked):**

- [x] Core: fix the `PRVW` doc comment. Expose the full-size `trak` JPEG's byte range next to
      `PRVW` (and keep `THMB`). **Done**: `PhotoMeta.full_preview` / Swift `fullPreview`, and
      display decodes read it. Verified against the real photos, not exiftool's tags — exiftool
      prints no `PreviewImageStart`/`JpgFromRawStart` for these files, so the sample table was the
      authority and the byte ranges are asserted to hold real JPEGs.
- [x] App: decode every display image from **byte ranges**, never from the CR3 URL. ImageIO then
      never parses the container or picks the RAW. **Done**, with the container decode kept as the
      fallback for a file that reports no range, and a test that the two give identical pixels.
- [ ] App: check that every cached `CGImage` is force-decoded (`ShouldCacheImmediately`) and in a
      native BGRA layout. Needs `ImageProvider.swift`.
- [ ] App: first-photo fast path (header + `PRVW` of the resume photo before the full scan).
      Needs `AppModel.swift`.
- [ ] App: one shared 256 px decode for the filmstrip and the visual signature.
- [ ] App: T2 at the viewer's backing size through DCT scaling; re-decode on resize (§7.1).
- [ ] App: speculative T3 for the current photo after a dwell; T2 upscale as the instant zoom.
- [ ] Instrumentation: `os_signpost` intervals named after the rows above, plus the debug HUD
      (§7.3).
- [ ] `firstcut bench --folder <dir>`: scan (cold/warm), order, batch and DB insert, per phase.
- [ ] **Owner:** run the perf suite and `firstcut bench` on the M1 Pro. Record the rows in
      `docs/qa/perf-baselines.md`. Pick by eye between the 3000 px DCT-scaled T2 and the
      full-decode-and-downscale T2 on a 16" screen.
- [ ] IOSurface-backed `contents` only if the arrow-key measurement exceeds 8 ms.

Sources: Clévy, *Describing the Canon Raw v3 (CR3) file format*
(github.com/lclevy/canon_cr3); Apple, *Image I/O Programming Guide* and the
`CGImageSourceCreateThumbnailAtIndex` options (`kCGImageSourceShouldCacheImmediately`,
`kCGImageSourceThumbnailMaxPixelSize`); Chromium `ui/accelerated_widget_mac/ca_renderer_layer_tree.mm`
(`CALayer.contents` = `IOSurfaceRef`); how Photo Mechanic culls from embedded JPEGs (imagen-ai.com
comparison articles). §3 of this file for the 166 ms and 570 ms single-thread decodes.

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

- [x] **RAW + JPEG/HEIF pairs** with the same base name are treated as **one photo** (RAW is the
      primary); every action (rating, XMP, move, trash) applies to all members of the pair, incl.
      sidecars.
- [x] Folders of only JPEG/HEIF must cull exactly like RAW folders.
- [x] Generic fallback: if the Rust parser doesn't recognize a file, ask ImageIO for its properties
      from Swift and pass them in, so no supported-by-macOS file is ever skipped.
- [x] Unsupported/corrupt files: shown in the filmstrip with a placeholder + reason, never block.
- [x] Validate every Canon parser field against `exiftool` output on the test games
      (`firstcut-cli verify <folder>` diff report). Non-Canon parsers are checked with unit tests on
      hand-built header byte fixtures only.

---

## 9. UI / UX

Reference: **Finder's Gallery view** (large image, filmstrip underneath, unified toolbar) on macOS 26
with Liquid Glass. It should be indistinguishable from an Apple app. Always dark.

### 9.1 Window & toolbar

- [x] Single-window app (`NSWindow` + unified toolbar, full-size content view, glass toolbar items).
- [x] Liquid Glass on macOS 26+: system toolbar glass, SwiftUI `glassEffect` / `GlassEffectContainer`
      for floating controls (HUD, overlays), AppKit `NSGlassEffectView` where views are AppKit. Use
      stock controls wherever possible so they pick up system styling automatically.
- [x] Standard macOS menu bar (File, Edit, View, Photo, Window, Help) with every command listed and
      its current (remapped) shortcut shown.
- [x] Toolbar, left: **‹ › batch navigation buttons** (Previous batch / Next batch) in a glass
      capsule like Finder's back/forward.
- [x] Toolbar, title: batch position + file name, e.g. `Batch 12 of 148 — IMG_8231`.
- [x] Toolbar, right: view mode segmented control (Loupe · Grid · Compare), Info panel toggle,
      Finish Cull button.
- [x] Full-screen support (hide chrome, filmstrip auto-hides at the bottom edge).
- [x] macOS 15 fallback: same layout with `NSVisualEffectView` materials; verify visually on a
      macOS 15 VM / machine.

### 9.2 Main viewer (Loupe)

- [x] Photo fills the area above the filmstrip, aspect-fit, rounded corners like Finder's gallery.
- [x] Neutral dark gray background (configurable darkness).
- [x] **Zoom (mouse/trackpad only, no keyboard shortcut)**:
  - **Pinch** to zoom in and out smoothly (trackpad magnify gesture), anchored at the pinch point.
  - **Click** a spot on the photo → jumps to **100% centered on that spot** (a single step, no
    multi-level zoom). **Click again** → back to fit.
  - When zoomed, drag (or two-finger scroll) pans. A click that turned into a drag must not toggle
    zoom.
  - Zoom in/out animates with the system spring, like Photos/Preview.
- [x] **Zoom lock** (setting, also in the View menu, no default key): when on, arrowing to the next frame keeps the same zoom
      level and position, so sharpness can be compared across the burst. T3 prefetches neighbours
      while zoom-locked.
- [x] **AF point overlay** (toggle): draw the in-focus AF point(s)/area from MakerNote data, mapped
      through orientation.
- [x] **Clipping overlay** (toggle, J like Lightroom): highlight/shadow clipping.
- [x] **Histogram** (toggle): computed from the T2 bitmap with vImage/Metal, shown in the info panel or
      as a floating glass HUD.

### 9.3 Filmstrip

- [x] Horizontal strip of **the current batch only**, Finder-style: thumbnails at their aspect
      ratio, selected frame on a rounded gray plate.
- [x] Scrolls to keep the selection visible; smooth at 120 Hz with 60+ frames. (Keeps the
      selection visible: done. 120 Hz with 60+ frames: still to measure.)
- [x] Shows rating stars / flags / color labels (stars mode) or green/red rings (keep mode).
- [x] Clicking a thumbnail selects it. No drag-reordering.
- [x] Custom AppKit / layer-backed implementation (not SwiftUI `ScrollView`) for guaranteed
      performance.

### 9.4 Batch navigation

- [x] ← / → move between photos **within** the batch. At the ends: stop (default) or continue into
      the next/previous batch (setting).
- [x] Previous/next batch: toolbar ‹ › buttons + remappable shortcuts (defaults in §10). Arrow keys
      stay reserved for photos.
- [x] Entering a batch selects its first photo (setting: or the last photo you viewed in it).
- [x] Visited/complete state per batch stored in the session DB.

### 9.5 Info panel (I)

- [x] **I** toggles a right-side glass inspector (like Lightroom's info), with: file name, capture
      time (with sub-seconds), camera + serial, lens, focal length, shutter, aperture, ISO, exposure
      comp, metering, AF mode + points, drive mode, shutter count, dimensions, file size, folder
      path, rating/flag/label, batch number and position, histogram.
- [x] Choose which fields are shown in Settings.

### 9.6 Other views (all optional, toggled from toolbar/shortcuts)

- [x] **Grid (G)**: the current batch as a grid; ratings and rings visible; Return/E goes back to loupe.
- [x] **Compare (C)**: 2-up / 3-up / 4-up of frames from the current batch with synchronized zoom and
      pan; arrows change the candidate frame.
- [x] **Progress HUD**: batch X of Y, photos left, keeps / good / maybe counts, elapsed time. Glass
      capsule, auto-hides, toggle key.
- [x] **Welcome window**: Open Folder…, recent sessions with progress (resume), drag-and-drop a
      folder onto the window/Dock icon.

### 9.7 Finish Cull flow

Triggered by the Finish button / shortcut, or offered automatically after the last batch.

- [x] Summary sheet: totals per tier (Keep / Good / Maybe / Unrated / Rejected), batches not visited
      (warning if any).
- [x] Actions for **unrated / not-keep / rejected** photos (choose one):
  - Leave files, mark them as rejected in XMP (Lightroom sees them as rejected)
  - Move to a subfolder next to the originals (name configurable, default `_Not kept`)
  - Move to Trash (recoverable in Finder)
  - Delete permanently (typed confirmation)
  - Do nothing
- [x] Optional actions for **kept** photos:
  - Copy or move to a destination folder
  - Split into subfolders by tier (`5 Keep`, `3 Good`, `1 Maybe`) or by star count
  - Write a text/CSV list of kept file names
  - Reveal in Finder / open the folder in Lightroom (via `open -a`)
- [x] Every file operation moves the whole group: RAW + paired JPEG/HEIF + `.xmp` sidecar.
- [x] Dry-run preview list before executing; progress + cancel; final report.
- [x] Moves are logged in the DB and undoable ("Undo Finish" restores files); Trash is recoverable via
      Finder; permanent delete is not undoable (said clearly in the UI).
- [x] Safety: never overwrite existing files at the destination (suffix instead), check free space
      before copying, handle read-only volumes gracefully.

### 9.8 Settings window (native Settings scene, tabbed)

- [x] **General**: rating mode, auto-advance, arrow behavior at batch ends (default: roll into the next batch), entering-batch
      behavior, default finish actions, confirmations.
- [x] **Keyboard**: full shortcut editor — every command listed, record a new key, conflict
      detection, reset to Lightroom defaults, import/export keymap JSON.
- [x] **Viewer**: background gray level, zoom lock default, AF overlay, clipping thresholds, info
      fields, HUD visibility, "Exact RAW" decode default.
- [x] **Metadata**: write XMP on/off, what Keep maps to (rating/label), overwrite vs merge existing
      XMP, also write ratings into DNG files directly (off).
- [x] **Performance**: memory budget slider, look-ahead batches, thumbnail size, decode threads
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

- [x] Zoom has **no keyboard shortcut** by design (pinch / click only, see §9.2).
- [x] Keymap stored as JSON in Application Support; default keymap shipped in the bundle.
- [x] Key handling via a single `NSEvent` local monitor / responder-chain router so no view steals keys;
      key repeat on arrows must be smooth.

---

## 11. Session, persistence & data safety

- [x] **Session DB**: SQLite (rusqlite, WAL mode) at
      `~/Library/Application Support/Firstcut/Sessions/<folder-id>.sqlite`, keyed by volume UUID +
      folder path + a fingerprint of file names/sizes, so a moved folder can be re-matched.
- [x] Tables: photos (path, group, metadata, hash), batches (members, visited, last position),
      ratings (current state), history (undo/redo log), file_ops (finish-step moves for undo).
- [x] **Resume**: reopening a folder restores batches, ratings, current batch/photo, zoom lock, view.
- [x] If the DB is missing but XMP sidecars exist, import ratings from XMP.
- [x] Ordering and batches never depend on file names; add a synthetic rollover fixture
      (`IMG_9998`, `IMG_9999`, `IMG_0001`, `IMG_0002` with increasing capture times) since the test
      games don't cross 9999 inside one folder.
- [x] Watch the folder with FSEvents: new files appear in new batches at the end (or are re-batched if
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
- [x] Rust unit tests: parsers (per format, fixtures = small header byte slices), ordering incl.
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
- [x] GitHub issue templates: bug report (camera model, file format, macOS version, steps) and feature
      request; `CONTRIBUTING.md` once there is code to contribute to.
- [x] Add `THIRD-PARTY-NOTICES.md` if any third-party code/crates with attribution requirements ship in
      the app (check every crate license is GPL-3.0 compatible).

---

## 14. Milestones

| Milestone | State | What is left |
| --- | --- | --- |
| M0 Repo & toolchain | Done | |
| M1 Core scan + order + batch | Built | Ground truth and the boundary-F1 number (§0.2) |
| M2 Pipeline | Built | The §7.3 timings on the photos; embedded-preview vs RAW comparison (§7.2) |
| M3 Core UX | Done | |
| M4 Inspection tools | Done | |
| M5 Finish flow, settings, keymap | Done | |
| M6 Formats | Built | Only Canon R8 CR3 is verified (§8) |
| M7 Polish | Built | Visual sign-off against Finder on macOS 26 (§0.2); macOS 15 is untested |
| M8 Release | Rehearsed | Tap secret, README screenshots, the `v0.1.0` tag (§0.2) |
| M9 Batch editing & presets (post-v0.1) | Planned | Everything in §16 |

---

## 15. Open questions & things to know

Decisions the owner has made are in §2. Work only the owner can do is in §0.2.

### A. Open decision

- **The high-speed pauses at the end of Game1JENKS** (`IMG_6117`–`IMG_6164`, ~11 fps with 0.2–0.8 s
  re-press pauses): one play or several? Build it as an eye test: show the owner "one batch" versus
  "split at each re-press pause" side by side, narrow until they pick, and ship only the winner.

### B. Things to know

- **The tree is green on CI** (2026-09-30). `RealRawDecodeTests` compiles; the real-photo tests are
  still opt-in. See §0.2 for what is left.
- **Nothing about accuracy or speed is measured yet.** Boundary F1 is unmeasured (no ground truth) and
  none of the §7.3 targets has a recorded number. Do not claim either.
- **Ad-hoc signing means TCC re-prompts on every rebuild.** With no paid Apple Developer account there
  is no stable signature, so a rebuilt app asking for `~/Documents` access blocks forever when
  unattended. Folder access must come from `NSOpenPanel` only; real-photo tests are opt-in
  (`FIRSTCUT_ALLOW_PHOTO_TESTS=1`).
- **Only Canon R8 CR3 is testable.** Every other format in §8 is implemented from its spec and unverified;
  the README must say so.
- **A rename keeps its rating (REV-68, done)**: the core reconciles by `st_dev`/`st_ino`, then by
  shutter count + size. `PhotoId` is still a hash of the relative path, so it changes on rename and
  the reconciliation carries the rating across.
- **Ratings must reach the sidecar.** A 4-star photo was once exported as `xmp:Rating="0"` (fixed in
  `5c98cf8`); keep an end-to-end test on it. Likewise keep/trash must be decided from the *mapped* tier,
  and unkeeping must clear the 5 stars it invented.
- **Rebuilding kills a running app.** Concurrent builds replace the binary under a running process and
  macOS reports "Firstcut quit unexpectedly". Close the app before building; §0.4 has the
  `NSQuitAlwaysKeepsWindows` fix for the phantom relaunch.
- **Dev environment is cleaned.** All worktrees, `core/target`, generated Xcode project and DerivedData
  were removed at the end of the merge; run `scripts/generate-project.sh` first (needs `xcodegen`,
  Rust via `~/.cargo/bin`, and `swift-format`). Rust is not on the default `PATH` in non-login shells.
- **History for the retired process** (review board, per-agent charters) is only in git:
  `git show 4d4e43d:docs/review.md`, `git show 4d4e43d:docs/agents/`.

---

## 16. Later features (after v0.1.0)

Planned but not part of v0.1. They lift two v0.1 non-goals (§1), so each one starts by recording its
decision in §2.

### 16.1 Manual batch editing (split / merge)

Automatic batching stays the default. The user can correct it where it is wrong.

- [ ] **Split** the current batch at the current photo, **merge** it with the previous or next batch,
      and **move a boundary** one photo either way. Each has a remappable shortcut (none by default)
      and a menu item. Every edit is undoable like a rating.
- [ ] Store edits as **overrides** in the session DB, keyed by photo ids: "boundary before X" and "no
      boundary before X". The automatic batcher (metadata, then visual refinement) keeps running;
      overrides are applied on top, so a re-batch or a rescan never loses them. A photo that is
      deleted drops its override.
- [ ] Mark user-made boundaries in the filmstrip and the batch HUD, so an automatic boundary and a
      manual one are told apart.
- [ ] Optional export of the overrides as `tests/fixtures/ground-truth/<g>.json`. Correcting a
      shoot then produces the §0.2 ground truth as a side effect. The owner still decides; an agent
      never writes ground truth itself.
- [ ] Tests: overrides survive re-batching, visual refinement and rename reconciliation; undo and
      redo; merging across a manual split.

### 16.2 Batch edits with importable presets

Simple edits applied to a whole batch (or a selection) at once, from presets the user imports.
Firstcut still does **not** render or export: an edit is a set of develop settings written to the
photo's XMP sidecar, which Lightroom Classic, Lightroom and Adobe Camera Raw apply when they read
the file.

- [ ] **Decision (§2):** confirm the scope. The proposal: white balance (temperature/tint), exposure,
      contrast, highlights, shadows, whites, blacks, vibrance/saturation, profile, and straighten/crop
      from a preset. Nothing brush-based or local.
- [ ] **Preset model** in the core: a named set of Camera Raw settings (`crs:` namespace), stored in
      Firstcut's library (`~/Library/Application Support/Firstcut/Presets/`) as Lightroom-format
      `.xmp`, so a preset exports back to Lightroom unchanged.
- [ ] **Apply to batch / selection:** merge the preset's `crs:` properties into each photo's sidecar
      with the existing XMP writer. Keep ratings, labels and unrelated properties. Ask before
      replacing develop settings the sidecar already has. Never touch a darktable-style
      `IMG.CR3.xmp` (§11). Undoable per batch; shown in the info panel.
- [ ] **Import presets:**
  - [ ] Lightroom Classic / ACR / Lightroom `.xmp` presets (`crs:PresetType`, `crs:*` settings,
        groups), single files and `.zip` packs.
  - [ ] Legacy Lightroom `.lrtemplate` (a Lua table: `s = { … value = { settings = { … } } }`),
        parsed and converted to the same model.
  - [ ] Capture One `.costyle` / `.costylepack` (XML `<E K="…" V="…"/>` entries): map the settings
        that have a clear Camera Raw equivalent, and list the rest as "not imported" in a report.
  - [ ] Others, best effort, with the same "not imported" report: darktable `.dtstyle` (mostly
        opaque module parameters, so likely the name only), DxO PhotoLab `.preset`, ON1, Luminar.
        Profiles and LUTs (`crs:Look`, `.cube`) are reported, not applied, at first.
- [ ] **Preview (optional, clearly marked "approximate"):** Core Image on the displayed image
      (exposure, temperature/tint, tone curve). It never replaces Lightroom's render, so the settings
      in the sidecar are what matters.
- [ ] **Capture One:** it reads ratings and labels from XMP but not `crs:` develop settings, so a
      preset reaches Lightroom/ACR only. Writing Capture One's own settings is a separate, later
      decision.
- [ ] **Tests:** hand-written fixture presets for every format (never commercial presets: licensing),
      import → model → sidecar round trip, the merge rules, and the unmapped-settings report. The
      owner's manual check: Lightroom applies an imported preset written by Firstcut.
