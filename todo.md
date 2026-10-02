# Firstcut — Todo

> The single source of truth for what Firstcut is, the decisions already made, and every piece of
> work needed to ship it. Check items off as they land. When a decision changes, update the
> **Decisions** table first, then the tasks that depend on it.

**Status (2026-10-01): feature complete for v0.1.0, green on CI, and every agent-side item in
[§0.3](#03-other-open-work) is now done** — the last of them, the T4 "Exact RAW" decode, landed in
`087c22c`. The work was done on the Mac itself rather than in the cloud container, so the app builds
here, the tests run against the real photos, and the timings in §7.3 are measured rather than
estimated. **What is left is [§0.2](#02-left-for-the-owner)**: it needs a person, not a machine.

---

## 0. Where we are

> Every session starts by reading this section (the handoff in [§0.5](#05-handoff-where-the-last-session-stopped)
> first), then `AGENTS.md`.

### 0.1 Done

| Area | State |
| --- | --- |
| Rust core (`core/`) | CR3 reader (verified against exiftool on all 2,880 files), other formats from their specs, ordering, batching (metadata then visual signature), session DB, XMP read/write, Finish plan/execute/undo, rename reconciliation. 300+ tests, fmt and clippy clean |
| App | Welcome with recents and resume; loupe with pinch/click zoom, pan, zoom lock, AF and clipping overlays; filmstrip; grid; 2/3/4-up compare with synced zoom; info panel with histogram; progress HUD with the current rating; both rating modes; undo/redo; Finish Cull sheet (summary → options → dry run → typed confirmation for delete → report → undo); Settings (General, Keyboard with recorder/import/export, Viewer, Metadata, Performance); menus; error alerts; live folder watching; flush on quit |
| Pipeline | Concurrent decode engine with a priority queue (current photo, its batch, next, previous), cancellation when the user moves on, memory budget with LRU eviction, memory-pressure shedding, visual signatures in the background, and an on-demand **Exact RAW** (T4) develop of the sensor data as its own cache tier |
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
2b. **Preview or RAW?** (§7.2's benchmark task.) Settings → Viewer → "Develop the sensor data, not
   the embedded preview" now toggles between them, so this is a matter of looking at the same
   high-ISO frame at 100% both ways and saying which you would cull on. The numbers are already
   measured (0.089 s preview vs 0.299 s RAW for the same 6000×4000, differing by 3–52/255 per
   pixel); only the judgement is missing. Whatever you decide should become the **default**, and
   §7.2's checkbox should be updated to say which.
3. **Game1JENKS re-press pauses** (`IMG_6117`–`IMG_6164`): one play or several? Decide by eye.
4. **Timings (§7.3)** on the M1 Pro with the photos: folder open → first photo, metadata scan,
   thumbnails, arrow → sharp photo, 100% zoom. `scripts/test-with-photos.sh` runs the opt-in tests —
   but note it **hangs** rather than fails when the host reads `~/Documents` (the TCC prompt has
   nobody to answer it), so point `FIRSTCUT_TEST_PHOTOS` at a copy outside a protected folder:
   `FIRSTCUT_TEST_PHOTOS=/tmp/fcphotos scripts/test-with-photos.sh`.
5. **Lightroom / Capture One** read the sidecars (ratings and keep labels survive an import) on a copy
   of a few photos.
6. **Tap secret** on `Kathir-D/Firstcut`: `HOMEBREW_TAP_DEPLOY_KEY` (deploy key with write access on
   `Kathir-D/homebrew-tap`) or `HOMEBREW_TAP_TOKEN`. Do not add `Casks/firstcut.rb` to the tap by hand
   before the first release: its checksum is a placeholder until then and breaks the tap.
7. **README screenshots / GIF**, then **tag `v0.1.0`** (`git tag v0.1.0 && git push origin v0.1.0`):
   the release workflow builds, signs, zips, publishes and bumps the cask.

### 0.3 Other open work

Not blocking v0.1.0; an agent can do these. **Every item an agent can do is now done** — the list
below is struck through where finished, and what remains is either in [§0.2](#02-left-for-the-owner)
(needs a person) or explicitly deferred for a stated reason.

- ~~**Keep threshold → core:**~~ **Done.** `AppModel` sends `session.setKeepThreshold(...)` after a
  folder opens and whenever `settings.keepThreshold` changes, so "Only 5 stars" now reaches Finish
  and `tierCounts`, not just the filmstrip. Wired through `SessionBackend` → `CoreSessionAPI` →
  `UniFFICoreSession` (the FFI export already existed; the app simply never called it). Two tests
  assert the call, and both fail if it is removed.
- ~~**One definition per type (REV-72/73):**~~ **Largely done, in two commits.** 17 hand-mirrors are
  now `typealias`es of the generated types (`Shared/CoreTypeAliases.swift`), so the `Ffi*` prefix
  stops there and `CoreTypeMapping` lost ~240 lines of 1:1 switches: Flag, ColorLabel (app enum, see
  below), TimeSource, FileKind, RawFormat, CaptureTime, ByteRange, EmbeddedPreview, AfPoint,
  PhotoMeta, AfInfo, Batch, SessionCursor, MatchKind, SkippedFile (now `relPath`, which is what the
  core always called it), Tier. Two things stayed mirrors, for reasons recorded in the alias file
  and in session-api.md: **Rating** (the app writes `Rating()` in dozens of places and a defaulted
  initializer cannot be added to an imported struct — delegating to the generated init of the same
  signature is recursion, assigning stored properties before `self.init` is an error), and the six
  session types that are genuinely app vocabulary. Two findings along the way: `Codable` on
  Rating/AfInfo/PhotoMeta was **vacuous** (nothing in the app encodes them — only `AppSettings` and
  the keymap are written), and the promised "the generated types take the plain names" swap never
  happened; `docs/contracts/build.md` and `session-api.md` now describe the shipped design instead.
- ~~**Strict CI:**~~ **Done.** `swift-format` has formatted the whole tree (4-space per
  `.swift-format`), so the advisory lint is enforced — which also meant *installing* it: the lint
  step had been exiting 127 (command not found) behind `continue-on-error` and reporting nothing
  since it was added. `SWIFT_TREAT_WARNINGS_AS_ERRORS=YES` on both CI builds (REV-12); six warnings
  the compiler had been carrying are fixed, including one SDK-dependent `try` that compiled on one
  toolchain and not the other (the Settings launcher now sends the AppKit action directly).
  `GCC_TREAT_WARNINGS_AS_ERRORS` is deliberately *not* set: it also covers the linker, which warns
  about the locally built Rust library's SDK — nothing to do with code quality.
- **Pipeline refinements (§7.1, §7.2):** ~~a display-sized (T2) decode per window size~~ done,
  ~~thumbnails at `.utility`~~ done (thumbnails decode on their own `.utility` queue now; the
  ranking and the shared cap are unchanged, and the comment says why the cap stays shared),
  ~~`os_signpost` and a debug HUD~~ done, and ~~the "Exact RAW" (T4) decode~~ **done** (`087c22c`).
  T4 develops the sensor data through `CIRAWFilter` as a **separate cache kind** (`case exactRaw`),
  for the current photograph only, and is now a real toggle in Settings → Viewer. Measured:
  **0.299 s and 92 MB** per develop against **0.089 s** for the embedded preview it replaces, which
  is why it is on demand rather than in the prefetch. **Left:** the embedded-preview vs
  `CIRAWFilter` comparison, which needs a person to judge the pictures.
- ~~**Settings in the model but not shown**~~ **Done, all three, and one of them found two bugs.**
  - *Clipping thresholds* reach `ClippingMask` through the viewer's presentation, and the model
    defaults were **wrong for the code that would read them** (1.0/0.0 means every pixel clips), so
    they are now the fractions the mask has always used, 250/255 and 5/255. The overlay had no tests
    at all, and they found two real bugs: the read buffer's byte order was not pinned, so the
    comparison read the **alpha** channel and every pixel of an opaque photograph counted as a
    highlight (J painted the whole picture red); and the buffer was premultiplied, so an
    unpremultiply turned premultiplied black into transparent black — the exact pixel the shadow
    overlay exists to find. Five tests, single-colour images.
  - *Decode thread count* is passed from `Dependencies.live`, and `0` (auto) resolves to the
    **measured knee** (four: this machine decodes 7.0 photos/s on four threads and on eight) bounded
    by the performance cores (`hw.perflevel0`, not `activeProcessorCount`) — the doc comment had
    claimed "performance core count", which the measurement does not support. Two tests, one of which
    fails if the setting is ignored.
  - *Confirmation toggles*: `confirmPermanentDelete` decides the typed DELETE gate, moved into the
    model at the dry-run stage because a rule in a private SwiftUI view had **no test coverage at
    all**; `confirmBeforeFinish` adds one stage before the summary; `confirmDiscardRatings` is
    **deleted** — no action in the app exists for it to confirm, so it promised a safeguard that did
    not exist.
- **Tests that need the photos or a person (§12):** performance baselines, the hold-→ stress test,
  a full manual QA pass, macOS 15 (only 26 is tested).
- **Contracts** in `docs/contracts/` — **done, all six.** `build.md` and `session-api.md` were
  corrected for the conversion layer and the type aliases; then `pipeline-api.md` (it described an
  IOSurface viewer layer, two provider methods and a `PipelineFocus` type that do not exist),
  `photo-meta.md` (it did not mention `full_preview` — the byte range every display decode reads —
  and still called `PRVW` full size), `batching.md` (no thresholds, no freeze rule) and
  `app-model.md` (no `canAct`, no fast path, stale `Tier` mapping). Every claim now carries a
  file:line, and T1/T4 are recorded as *not implemented* rather than described as if they were.
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

> **As of 2026-10-01 (agent session on Kathir's Mac, second pass).** Every agent-side item in §0.3
> is now done, including the T4 "Exact RAW" decode that the previous handoff stopped short of.
> `main` is green on CI: Rust fmt/clippy/tests, the Swift build and tests on **Xcode 16.4 with
> warnings as errors**, and the enforced `swift-format` lint. **What is left is §0.2, which needs a
> person** — and one §0.2-shaped judgement call (§7.2's preview-vs-RAW comparison).
>
> - **The §0.5 list from 2026-09-30 is complete:** the first-photo fast path, the bench db phase
>   (measured: 0.235–0.300 s cold, scan+db 0.87–1.10 s scaled, against a 3.5 s target), the sidecar
>   import and same-name-swap fixes, and the screenshot pass.
> - **§0.3 items done:** one definition per type (17 mirrors aliased), strict CI, all three
>   unhonoured settings, and **T4**. Two of the settings found real bugs — the clipping overlay
>   painted every pixel red and never saw black, and `Codable` on three core types was vacuous.
> - **§0.4's checklist is now the thing that would have saved this session:** every claim about T4
>   had to be measured because the plausible-looking ones were wrong. `CIRAWFilter` was recorded as
>   unusable when it is public API; the EXIF orientation was recorded as something to apply in the
>   app when the filter applies it. Both are documented where the code lives now.
> - **CI was red for three pushes before this session's first fix.** `bitmapInfo.byteOrder` is a
>   newer-SDK overlay member; CI builds with Xcode 16.4 against the macOS 15 SDK. The lesson is in
>   §0.4's checklist: **anything that compiles locally under Xcode 27 is not evidence about CI.**
>   Two more traps followed: `openSettings()` throws on one SDK and not the other, and
>   `GCC_TREAT_WARNINGS_AS_ERRORS` turns a deployment-target linker warning into a failure.
> - **A warning for whoever picks up item 2: measuring beats assuming, and it cost two wrong
>   conclusions before the right one.** The previous handoff recorded `CIRAWFilter` as unusable;
>   it is public API and works, and the reason it looked broken is that the *documented-looking*
>   `CIFilter(name:)` route returns an object with no input keys and **throws an uncatchable
>   `NSException`** on the obvious key. Along the way three plausible-looking probes all pointed
>   at "no demosaic available on macOS": `CIImage(contentsOf:)` and
>   `CGImageSourceCreateImageAtIndex` really are just the embedded preview (confirmed against the
>   camera's own extracted JPEG bytes: 0.24/255 apart from each other, ~5/255 from the JPEG), and a
>   4 ms "full resolution" render is not a demosaic. The discriminator that settled it was the
>   cheapest one available: **compare a candidate against the same object's own preview**, so
>   colour management cancels. If a probe here reports something surprising, check it against a
>   control before believing it — one of mine scored *identically* to its own control (3.827 vs
>   3.827), which is only possible if the control was broken.
>
>   Two more traps from the same session, both of which cost time and are now in the code as
>   comments: the EXIF orientation tag on a CR3 is at the **top level** of the properties (and
>   mirrored in `{TIFF}`), *not* in the `{Exif}` sub-dictionary — reading the Exif dictionary
>   returns nil, so a `?? 1` would report "every frame is upright" and silently make the
>   orientation test assert nothing. And **a RAW develop must not re-apply the orientation**,
>   because the filter has already applied it (see item 1).
>
> **Next, in order:**
>
> 1. ~~**"Exact RAW" (T4) decode.**~~ **Done**, in `087c22c`. T4 is a separate `DecodeEngine.Kind`
>    (`case exactRaw`), so the cache separation the design called for is free from `Key`'s
>    `id + kind`; `exactRaws` holds the current photograph only; `setExactRaw` replaces whatever the
>    tier was doing on every focus report, so moving on drops the develop; and work already running
>    when the setting is turned off is marked `cancelled` (deliberately not `failed`, so asking
>    again still develops) rather than re-inserting 92 MB. The toggle is now in Settings → Viewer,
>    the debug HUD has a row for it, and `docs/contracts/pipeline-api.md` is v0.4.
>    Verified: 6 unit tests (no photos) + 4 integration tests on real CR3s, Swift 284 green, Rust
>    338 + 3 CLI, lint clean. Four mutations were each checked to fail a test — filing the develop
>    into the display store, dropping the develop, removing the `kind` gate, and ignoring the
>    cancellation.
> 2. **Pipeline:** the embedded-preview vs `CIRAWFilter` comparison (§7.2) still needs a person to
>    judge the pictures. Note the timings that make it a real question rather than a formality: the
>    shipped preview is 0.089 s and T4 is 0.299 s for the same 6000×4000, and they differ by
>    3–52/255 per pixel. Both are now reachable from the app (the toggle is in Settings → Viewer),
>    so this is a matter of looking at the same frame both ways.
> 3. **Speculative T3 after a dwell** (§7.5) — deliberately deferred until the arrow-key measurement
>    exists, because the measurement is what decides whether it is worth a decode.
> 4. **The owner's items** in §0.2, unchanged and still first in line for a human: the boundary-F1
>    ground truth, the Game1JENKS re-press decision, the perf numbers on the running app, the
>    visual sign-off against Finder, and the tap secret / `v0.1.0` tag.
>
> **One thing worth knowing before running the photo tests on this machine:** they **hang**, rather
> than fail, when the app reads `~/Documents`, because the test host is a GUI app with a fresh
> ad-hoc identity on every rebuild and nobody is present to answer the TCC prompt
> (`docs/contracts/build.md`). The escape is the one build.md records — point
> `FIRSTCUT_TEST_PHOTOS` at a copy outside a protected folder:
> `FIRSTCUT_TEST_PHOTOS=/tmp/fcphotos scripts/test-with-photos.sh FirstcutIntegrationTests/<Suite>`.
> Note that `testAFolderOfCR3sScansBatchesPrefetchesAndAnswersFromCache` asserts `photos.count ==
> 708` for Game1JENKS, so a small copy will fail that test; the T4 tests take their own sample and
> pass on a small folder.
>
> **How to run things quickly** (the full suite is ~15 min of builds; these are seconds):
> - Rust, one area: `cargo test -p firstcut-core --lib session::` (or `meta::`, `batch::`).
> - Swift, one area: `xcodebuild … test -only-testing:FirstcutUnitTests` (the whole unit target is
>   ~1.2 s once built; add a suite name to narrow it, e.g. `-only-testing:FirstcutUnitTests/ImageProviderTests`).
> - One test against real photos: `FIRSTCUT_TEST_PHOTOS=/tmp/fcphotos scripts/test-with-photos.sh
>   FirstcutIntegrationTests/RealRawDecodeTests/testExactRawThroughTheProviderOnRealCR3s` (~2 min,
>   mostly the build).
> - Nothing in a test run may touch `~/Documents` (build.md); real-photo tests are opt-in behind
>   `FIRSTCUT_ALLOW_PHOTO_TESTS=1`.

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

- **Fixed — the shoot was decoded twice.** `VisualSigWorker` decoded every photograph from the file
  URL to compute its visual signature, *in addition to* the pipeline decoding the focus window. At
  ~300 ms per CR3 that is the whole 2,880-photo shoot read and decoded a second time, on the same
  four threads, competing with the decodes the user is waiting for — and it is why the §7.3 "all
  thumbnails + hashes < 20 s" row was hopeless as written. The two now share one decode: the
  signature pass asks the cache first and publishes what it decodes, so each photo is decoded once
  whichever pass gets there first, and the filmstrip gets a free thumbnail for everything the
  signature pass touched. `ImageProvider` conforms to a `ThumbnailSource` seam (`cachedThumbnail` /
  `offerThumbnail`, both `nonisolated` and both a single `NSLock` read, so the detached `.utility`
  pass never has to touch the main actor). **The remaining decode timings are still unmeasured** —
  this removes a duplicated decode, it does not by itself establish the 20 s number.
- **Next, in order:**
  1. ~~Wire the keep threshold from the app.~~ Done above.
  2. ~~Check the cache on a folder change.~~ Done above.
  3. ~~§7.5 work items~~ Done: the `PRVW` comment, the full-size JPEG byte range, `firstcut bench
     --folder` (including the **db phase**, measured), `os_signpost` names, the shared 256 px decode,
     display decodes from the byte range — and the rows are measured in `docs/qa/perf-baselines.md`.
  4. ~~The first-photo fast path.~~ Done: one header read puts a photograph on screen while the
     scan runs behind it (§7.5 work items, above).
  5. ~~Sidecar ratings were imported only when a folder's session was first created~~ **Fixed.**
     The import now runs at **every open and every rescan**, for photographs the session has no
     rating row of its own for — so a Lightroom-rated card copied into an open shoot, or a photo
     rated elsewhere between two opens, arrives rated, while a rating Firstcut made (or cleared)
     can never be overwritten from outside. The same change closes the mirror bug: **a different
     capture saved under a known name no longer inherits that name's rating.** `reconcile` treats
     a known id whose *every* identity disagrees — inode, shutter count and size — as a different
     photograph and drops the departed one's rating and undo history; the departed one's stale
     sidecar is recognised (it still holds exactly the departed row's stars and label) and is not
     re-imported onto the new capture. Any partial agreement (edited in place, same capture back
     from a card, a parser that missed the shutter count) keeps the rating — destroying a rating
     on a guess is worse than lending one on a coincidence. Five tests, each verified by mutation
     to fail when its half of the fix is undone.
  6. **Screenshot pass done, and it found two real bugs** (the loupe rendered flat black in the mock
     shoot, and `-FirstcutSettings 1` opened nothing):
     - **The viewer bound the wrong image source.** `AppEnvironment`'s host factory captured the
       real `ImageProvider` at registration, so after the mock shoot swapped the whole cull state
       (whose source is synthetic) the viewer still held a provider that had never been given a
       folder — every lookup nil, a black loupe, while the filmstrip read the state and drew its
       thumbnails. The factory now resolves **`state.images` and `state.activeModel` when a pane
       is created** (`CullViewState.activeModel`, new seam, nil for the preview stand-in), so a
       `use(_:)` swap is honoured and the panes report frames/viewport to the model on screen.
       Verified by pixel analysis of a `screencapture` before and after: the viewer region went
       from one flat colour to content. The CI Screenshots workflow benefits directly — its loupe
       captures were black.
     - **`-FirstcutSettings 1` opened nothing**: `openSettings()` threw silently on macOS 27
       (`try?` swallowed it), so the screenshot workflow had no Settings screen. It now reports
       the error and falls back to the AppKit action the Settings menu item itself sends
       (`showSettingsWindow:`, then the macOS 13 spelling) — verified: the General window opens.
     - Every screen was then launched and checked (pixel statistics — the agent cannot see, so
       this is a structural pass): loupe, grid, compare2/4, Finish sheet, Settings, Welcome all
       render content; no blank or one-colour region remains except where one belongs.
     - What is still the owner's: the *look* — spacing, text truncation, Finder-likeness (§0.2
       item 1). Those need eyes.
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
| What `CGImageSourceCreateImageAtIndex` gives for it | **16 bits per component, Display P3** → 6000×4000×8 B = **183 MB**, and **~850 ms** to narrow to 8-bit device RGB |
| Display decode through `CreateThumbnailAtIndex`, full size (6000 px) | **~85–150 ms**, 8-bit, **92 MB** |
| Display decode at 3456 px (a full-screen 16" viewer) | **~156 ms**, **30 MB** |
| Display decode at 3000 px | **~147 ms**, **23 MB** |
| Display decode at 2000 px | **~61 ms**, **10 MB** |
| Thumbnail decode at 256 px, 1 thread (`CreateThumbnailAtIndex`) | **~280–310 ms** (per *file*, not per pixel) |
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
- **Never read the display preview with `CGImageSourceCreateImageAtIndex`.** It is the same pixels
  but at 16 bits per component in Display P3, which costs ~850 ms and 183 MB; the thumbnail call on
  the same file returns 8-bit pixels and subsamples in the DCT — **~5.8× faster at the same size, and
  5.8× less memory at a viewer's size** (§7.1, `RealRawDecodeTests.testDisplayDecodeCosts`).
- 61–156 ms/image single-threaded is too slow to do on keypress but trivially fast to do *ahead of
  time* across 8–10 performance cores → the whole design is **prefetch everything reachable**.
- Every cached bitmap is normalised to 8-bit BGRA (`noneSkipFirst | byteOrder32Little`) in the
  **device** colour space, because a bitmap in any other layout makes Core Animation convert it
  *inside the commit* the key-to-frame interval closes on (§7.1, §7.5).
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
| T3 | Decoded full-resolution (6000×4000, 8-bit) bitmaps for 100% zoom | The photo on screen while zoomed, plus the next one in the batch when zoom is locked | 92 MB each |
| T4 | True RAW decode (`CIRAWFilter`) — **built** | Only when "Exact RAW" is toggled, and only the current photograph: 92 MB and 0.299 s a picture, measured, against 0.089 s for the preview it replaces | one picture at a time |

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
      generation at `.utility` so it never competes with the current batch. **Done:** 4 concurrent
      decodes at `.userInitiated` — the **measured knee** on this machine (7.0 photos/s on four
      threads and on eight, over 24 real CR3s), not `activeProcessorCount`, which includes the
      efficiency cores; thumbnails decode on their own `.utility` queue, ranked below the current
      batch but sharing the same 4-slot cap so a filmstrip pass cannot starve the frame the user is
      waiting for. T4 shares `.userInitiated` with display decodes: at 0.299 s it is the longest
      single decode in the app.
- [ ] Pre-upload decoded bitmaps to the GPU (IOSurface-backed) so display = pointer swap, < 1 frame.
- [x] Re-decode T2 when the window/screen size changes, keeping old bitmaps visible until new ones
      are ready. **Done**: a display entry carries the size it was decoded at; a request for more
      returns the smaller entry *and* schedules the bigger decode, so a resize never blanks the
      viewer, and a decode that lands after a bigger one does not shrink the cache back. Not
      debounced: `FocusRequest` is debounced by the fact that it is only re-sent when the size
      actually changes, and a stale T2 for a tenth of a second is not worth a timer.

### 7.2 Image quality rules (don't make a grainy high-ISO shot look soft)

- [x] Display at **native pixel scale**: T2 bitmaps are decoded at exactly the viewer's backing
      pixel size (Retina aware) — no double resampling, no GPU minification blur. **Done**: the
      viewer reports its frame in backing pixels to the model, the model puts it in every
      `FocusRequest`, and the queue sizes each display decode from it (§7.1). A window that grows
      re-decodes and **keeps the old bitmap on screen** meanwhile; `PipelineStats.displayResizes`
      counts that and must settle at 0 while the window is still.
- [x] Downscale with a high-quality filter (Lanczos / area average via ImageIO's DCT scaling +
      vImage) — never nearest/bilinear. **Done by construction**: the decode asks ImageIO for the
      target size and it subsamples *in the DCT*, so there is no second resample to get wrong, and
      the 1:1 redraw into the display layout uses `interpolationQuality = .none` so it cannot
      soften anything. Measured on a real CR3: 6000 px → 146 ms/92 MB, 3456 → 156 ms/30 MB,
      3000 → 147 ms/23 MB, 2000 → 61 ms/10 MB.
- [x] Every cached bitmap is force-decoded (`kCGImageSourceShouldCacheImmediately`) **and** in a
      native 32-bit BGRA layout in the device colour space, so `layer.contents` is a pointer swap
      with no conversion inside the commit. A bitmap in a foreign layout fails
      `DecodeEngine.isDisplayLayout`, which two tests assert for every decode path.
- [x] Never re-encode to JPEG/HEIC for caching; cache decoded pixels or the camera's original
      compressed bytes only.
- [ ] Preserve color: honor the embedded ICC profile, render in the display's color space
      (Display P3), 8-bit is fine for previews; consider 10-bit for T3.
- [ ] **Benchmark task**: compare embedded preview vs `CIRAWFilter` decode on the high-ISO night
      shots in the test set at 100% and fit-to-screen; pick defaults from the result and document it
      here (grain structure, sharpness, noise reduction differences). **Both halves of this are now
      in place:** the numbers are measured (preview **0.089 s**, T4 **0.299 s** for the same
      6000×4000, and the two differ by 3–52/255 per pixel), and both paths are reachable from the
      app — Settings → Viewer → "Develop the sensor data, not the embedded preview". What is left is
      the only part no measurement can do: **a person looking at the same frame both ways** and
      saying which is better, which is §0.2's judgement call, not an agent's.

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

**Measured (2026-09-30, M1 Pro, macOS 27, internal SSD, `firstcut bench --folder`, release
build).** The metadata scan and provisional batching are now measured on the real photos, cold and
warm, and both are inside their targets with room to spare:

| | measured (1,500 files, scaled) | target |
| --- | --- | --- |
| Metadata scan, headers only, **cold** (`sudo purge` first) | **0.54-0.59 s** | < 3 s |
| Metadata scan, **warm** (best of 5) | **0.054-0.056 s** | - |
| `order()` + `batch()` | **< 0.5 ms** | < 2 s (§5) |
| Provisional batches ready, cold, end to end | **0.54-0.59 s** | < 3.5 s |

Per-photo scan cost is 0.35-0.39 ms cold on all four games, so it is linear and I/O-bound; the ~9x
cold/warm gap is the page cache and nothing else. The interesting consequence is that **the batching
algorithm is not the bottleneck** - it is roughly 4,000x inside budget, so the remaining work on
"folder open -> first photo" is decoding and first paint, not metadata.

Full table, with the command used: [docs/qa/perf-baselines.md](docs/qa/perf-baselines.md).

Still **unmeasured**, and so still not claimed: folder open -> first photo on screen, all thumbnails
+ hashes, arrow key -> sharp photo, batch switch, 100% zoom, and memory. Those need the running app,
and their rows in that file are still empty.

- [x] Instrument with `os_signpost` + a hidden debug HUD (cache hits/misses, decode queue depth,
      memory by tier, frame times). **Done.** `App/Sources/Pipeline/Signpost.swift` names intervals
      after the §7.3 rows (`keyToFrame`, `batchToFrame`, `zoomToSharp`, `openToFirstPhoto`,
      `decodeThumbnail`, `decodeDisplay`, `decodeFromBytes`, `setFocus`, `evictions`) and the decode
      engine, the focus update and the eviction pass emit them. The debug HUD
      (`Views/HUD/DebugHUD.swift`, Settings → Performance → Debug HUD, which was a setting nothing
      honoured until now) shows focus misses, queue depth, cache hits, memory by tier against the
      budget, decode failures, pressure sheds and the viewport size T2 is decoded at — plus the
      **measured key-to-frame**, last and worst, and the count of frames that reached the user as the
      256 px stand-in.
      The four intervals that need a **presented frame** are now wired end to end: `AppModel` opens
      the span in the command handler and `CGImageViewerHost` closes it from a `CATransaction`
      completion block at the commit that swaps `layer.contents`. Three details are the difference
      between a measurement and a number that looks like one: a **stand-in does not close a span**
      that is waiting for the display decode (otherwise a missed prefetch reports sub-millisecond
      key-to-frame over a soft picture); a key that **moves nothing** closes its own span (otherwise
      a → at the end of the shoot reports the time until the *next* keystroke); and a **failed open**
      or a **closed session** ends the span rather than leaving it open for the rest of the process.
      `zoomToSharp` is bounded by what the pipeline can answer — today the display decode *is* the
      full-resolution one, so it closes on the first committed frame; it becomes "T3 on screen" when
      the T2/T3 split below lands. Nine tests in `FrameIntervalTests`, and each of the three
      properties above fails if its guard is removed.
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
| T2 ready before the user can reach it | **Done, as written.** The full-size JPEG by byte range (the core's `PhotoMeta.fullPreview`), at the viewer's backing-pixel size, `ShouldCacheImmediately`, normalised to a native 32-bit BGRA layout so Core Animation does not convert it. ImageIO subsamples in the DCT, and **measured** (not estimated): 147 ms/23 MB at 3000 px, 156 ms/30 MB at 3456 px, versus 146 ms/92 MB at the camera's full 6000 px. The by-eye choice §7.2 asked for turned out to be unnecessary: the thumbnail call *is* the DCT path, so there is nothing to choose between. | **measured**: 147–156 ms per photo at a viewer's size, 61 ms at 2000 px; 4 in flight ≫ key-repeat rate | `focusMisses` = 0 in the hold-→ stress test; `displayResizes` = 0 while the window is still |
| Arrow → sharp photo ≤ 8 ms | Only a pointer swap on the main thread: a cached, already-decoded bitmap assigned to `layer.contents`. Make sure nothing on that path decodes, color-converts, resizes or touches SQLite. Write the rating off the main thread. If the commit is still too slow (it copies up to ~24 MB), make the bitmap IOSurface-backed and set `contents` to the `IOSurfaceRef`. That works on macOS: Chromium's `ca_renderer_layer_tree.mm` does it. The claim in `CGImageViewerHost.swift` (REV-38) that it cannot is wrong. Only do this if the measurement asks for it. | Assignment < 1 ms; commit to be measured | Signpost from `keyDown` to the next presented frame (`NSView.displayLink` / `CADisplayLink`), p50/p95/p99, `XCTOSSignpostMetric` |
| Batch switch ≤ 1 frame | Same path; the next/previous batch is already in T2 by the priority rules | — | Same signpost for `⌘→` |
| 100% zoom < 150 ms first time | The full-size decode is **~146 ms** on one thread (measured, §3), so the first 100% needs a head start. Zooming shows the T2 upscaled at once, so the zoom responds in one frame, and the full-resolution decode (T3) for the photo on screen and — with zoom lock — the next one in the batch is already in flight. ImageIO cannot decode one JPEG on several threads or decode just a region, so speculation is the only lever, and the zoom-lock prefetch is the only speculation that matters. **Half done:** the tiers and the prefetch are wired and the `zoomToSharp` interval closes on the frame that answers the click; the *speculative T3 after a dwell* is not implemented (open below) | Zoom responds in one frame; the sharp 100% lands ~146 ms after the click, 0 ms when zoom lock already had it | `zoomToSharp` signpost, click → the committed frame |
| Memory within budget, no leaks | A T2 at a 3456 px viewer is 3456×2304×4 B = **30 MB** (measured), not 24 MB and certainly not the 96 MB the old full-size decode spent. Three batches of up to ~50 frames (Game1JENKS bursts) ≈ 4.5 GB, inside the 6.4 GB default. Hold T1 (compressed bytes, 1–6 MB) for further batches instead of T2. Keep only the two T3s zoom lock needs (92 MB each). | Peak ≈ 4.5 GB on the worst burst | `footprint`/`phys_footprint` sampled in the stress test; `leaks` at the end |

**Work items (agent-side unless marked):**

- [x] Core: fix the `PRVW` doc comment. Expose the full-size `trak` JPEG's byte range next to
      `PRVW` (and keep `THMB`). **Done**: `PhotoMeta.full_preview` / Swift `fullPreview`, and
      display decodes read it. Verified against the real photos, not exiftool's tags — exiftool
      prints no `PreviewImageStart`/`JpgFromRawStart` for these files, so the sample table was the
      authority and the byte ranges are asserted to hold real JPEGs.
- [x] App: decode every display image from **byte ranges**, never from the CR3 URL. ImageIO then
      never parses the container or picks the RAW. **Done**, with the container decode kept as the
      fallback for a file that reports no range, and a test that the two give identical pixels.
- [x] App: check that every cached `CGImage` is force-decoded (`ShouldCacheImmediately`) and in a
      native BGRA layout. **Done, and it was not a formality.** Measured on a real CR3: the
      `CreateImageAtIndex` path the app used returned **16-bit Display P3** pixels, so (a) every
      cached display bitmap cost 183 MB rather than 92 MB, (b) Core Animation converted it to the
      display's format *inside the commit* the key-to-frame interval closes on, and (c) narrowing it
      cost ~850 ms per decode. Every decode now goes through `CreateThumbnailAtIndex` (8-bit, DCT
      scaled) and `inDisplayLayout` (one 1:1 redraw into `noneSkipFirst | byteOrder32Little` in the
      device space, skipped when the decode is already there).
- [x] App: first-photo fast path (header + `PRVW` of the resume photo before the full scan).
      **Done.** `first_photo_name` / `read_photo` in the core (a directory listing names a file, one
      header read parses it — the same `meta_for` the scan uses, so the photograph is byte-for-byte
      what the scan will return, asserted on the real photos in `tests/cr3_exiftool.rs`), and
      `AppModel.startFastPath` shows that one frame in `.culling` while the 2,880-header scan runs
      behind it. The frame is display-only (`canAct` false): every mutating command refuses rather
      than writing through the *previous* folder's backend, it is not recorded as a recent, and the
      watcher for the old folder cannot rescan into it. A failed open drops it and puts the shoot
      that was open back; a superseded open takes it down before the new one shows its own. Eight
      tests; the pipeline keeps the decode (same folder + id), so the real open costs nothing.
- [x] App: one shared 256 px decode for the filmstrip and the visual signature. **Done.** The pass
      asked the pipeline's cache first and published whatever it decoded, so each photograph is
      decoded once whichever pass gets there first, and the filmstrip gets a free thumbnail for every
      photo the signature pass touched. This was the largest remaining cost in §7.3: a CR3 is ~300 ms
      to decode, and the old code decoded the whole shoot from the file URLs *on top of* the focus
      window, on the same 4 threads, fighting the decodes the user is waiting for. Four tests, and
      both directions fail if the sharing is removed (checked by mutation).
- [x] App: T2 at the viewer's backing size through DCT scaling; re-decode on resize (§7.1).
      **Done**, and it needed one thing nobody had wired: `AppModel.setViewportPixelSize` existed and
      **nothing called it**, so `FocusRequest.viewportPixelSize` was always `.zero` and the HUD's
      "viewport: not reported" was literal. `CGImageViewerHost` now reports its frame in backing
      pixels on layout (only when it changed), and the rule is one pure function,
      `ImageProvider.displayEdges`: T2 = the viewer's longest edge, T3 = the photograph's own size
      for the photo on screen while zoomed plus the next one in the batch when zoom is locked,
      capped at the file's own size, with a 2048 px fallback before the window has reported one.
      `ViewerPresentation.pixelSize` came with it, so "100%" is 100% of the *photograph* rather than
      of whatever bitmap the cache held — without that, a 2880 px T2 would have been called 100%
      and 100% would have been soft.
- [ ] App: speculative T3 for the current photo after a dwell. The **T2-upscale-as-instant-zoom** half
      is done as a consequence of the tier split: at 100% the viewer asks for the photograph's own
      pixels (`ViewerPresentation.pixelSize`), the T2 stays on screen at `contentsGravity = .resize`
      meanwhile, and the T3 replaces it when it lands — so nothing about the zoom is ever blank. The
      dwell-based speculation (~300 ms on a photo) is not implemented: it has to be weighed against
      the decode it costs, and the measurement that would decide it does not exist yet.
- [x] Instrumentation: `os_signpost` intervals named after the rows above, plus the debug HUD
      (§7.3). **Done**, including the four spans that a presented frame has to close.
- [x] `firstcut bench --folder <dir>`: scan (cold/warm), order, batch, **db (session open: create +
      insert + sidecar import + rebatch)**, per phase. **Done, and measured on the photos** — see
      [docs/qa/perf-baselines.md](docs/qa/perf-baselines.md) (format v2). `Session::from_scan` is
      the seam: it takes the scan the bench already made, so "scan" and "db" add up to one open with
      nothing counted twice; a fresh scratch sessions dir per run keeps every run on the Created
      path, and the bench refuses a scratch dir that was not fresh. db cold 0.235–0.300 s across the
      four games, scan+db 0.87–1.10 s scaled to 1,500 — inside the 3.5 s target with ~3× headroom.
      The db phase is the second-biggest cost after the scan, not a rounding error.
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
| M2 Pipeline | Built | The §7.3 timings on the photos; the embedded-preview vs RAW judgement (§7.2, §0.2 item 2b — both paths now ship) |
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
