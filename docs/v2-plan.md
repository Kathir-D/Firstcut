# Firstcut v2 — Plan of Record

> The pure-Rust rewrite branch. Scope, decisions, and milestones for everything on
> `v2`. **This file is the contract for the branch**: when reality here diverges from it, this
> file is updated in the same commit as the divergence, with the reason. Nothing on `v2`
> breaks a rule in it without an owner sign-off.
>
> Written 2026-10-19 with the owner, before any M0 code. Start prompt: the LightCraft-shaped
> "Pure-Rust App, Native + WebAssembly" kickoff prompt (owner-supplied), amended below.
>
> **Owner amendments at plan review, same day:**
> 1. **The Web/WASM target is removed from the goals entirely.** Not deferred — dropped. No web
>    build tasks, no wasm crate shaping, no `docs/web.md`, no browser parity rows.
> 2. **Tiered test discipline replaces "full CI before every commit"** (§9): slow suites run
>    when they matter — at milestone gates — not after every single commit.

---

## 1. What v2 is

**Firstcut is rewritten as pure Rust with no Swift anywhere in the build**, so the same
application runs natively on macOS, Windows and Linux from one codebase. The rewrite adopts the
*LightCraft shape* (github.com/storytold/lightcraft): a small, engine-first workspace of
single-purpose crates, one command registry behind every surface (UI, CLI, control socket,
MCP), strict layering enforced in CI, agents able to drive the real UI.

Also decided here: the macOS-26 Liquid Glass presentation is dropped. The v2 look is a dark,
translucent-panel design ("transparency and what looks good") that renders identically on all
three OSes.

Not v2: new features. The branch is a **port + re-platform to full parity** with the v1.0
feature list. The owner merges `v2` into `main` by fast-forward when done; `main` never
receives partial v2 state.

## 2. Decisions (owner and agent, all settled)

| # | Decision | Call | Notes |
| --- | --- | --- | --- |
| D1 | GUI framework | `egui` + `eframe`/`wgpu` | Immediate mode fits a culler's per-frame state machine; custom paint for loupe/overlays; `emulsion` precedent |
| D2 | RAW decode | **LibRaw** via bindings, native only | See D3. Embedded-JPEG decode via `zune-jpeg` stays the fast path (§4.2) |
| D3 | C/C++ dependency approval | **Approved** by owner | The kickoff prompt's "no C/C++ unless I approve" clause: LibRaw (C++) is approved for the v2 native target, recorded here and in AGENTS.md. The only `unsafe` in the workspace is the LibRaw bindings crate (§4.5) |
| D4 | Web/WASM target | **Removed from scope entirely** | Not a goal of this project — not built, not deferred. v2 targets three native OSes and nothing else |
| D5 | License | **GPL-3.0 stays** | Prompt's MIT/Apache dual-license files are dropped. LibRaw links under its LGPL-2.1 option (CDDL not used) |
| D6 | Existing Rust core | **Split into the ladder** | `firstcut-core`'s modules fold into the domain crates unchanged in logic, tests move with them; UniFFI/`ffi.rs` deleted |
| D7 | Scope | **Full parity before merge** | Screen-by-screen checklist (§7) gates the merge |
| D8 | Packaging | **All three OS at merge** | mac `.app` + Homebrew cask; Windows zip; Linux AppImage + `.deb` (§6) |
| D9 | Branch | **`v2`** | One branch; small commits; push after every commit; never force-push; merge on green only |
| D10 | Presentation | **No Liquid Glass** | Dark translucent panels, one bundled font family (Inter), open icon set (Lucide). SF Symbols is Apple-licensed and cannot be bundled cross-platform |
| D11 | Exact RAW developer | **rawler 0.8 first, LibRaw fallback** | rawler is pure Rust, builds on all three OSes (closing the Windows gap), and develops the M1 Pro benchmark CR3 in ~0.26 s warm versus LibRaw's ~1.03 s. The pixel pipeline (PPG demosaic, white balance, camera matrix, sRGB, pack, rotate) is the fused `firstcut-decode-libraw::fused` plan, which is rawler's PPG arithmetic over three planes instead of a 288 MB interleaved buffer. LibRaw stays the macOS/Linux fallback for formats rawler refuses; the unsafe carve-out does not widen |
| D12 | Exact RAW presentation | **Progressive, never a blank loupe** | The display-sized embedded preview paints first; the sensor develop replaces it in place when it lands. Exact RAW is still a tier of its own and the timing row below measures the develop itself, not the first frame |
| D13 | Held-arrow navigation (owner directive, 2026-10-22, revised 2026-10-26) | **Fast `PRVW` tier + long display prefetch + deferred exact develop + zero-copy handoff** | A held arrow key moves every 33–50 ms, faster than the ~57–68 ms full embedded-preview decode and much faster than the ~0.26 s sensor develop. The loupe paints the camera's 1620×1080 `PRVW` JPEG in ~6–8 ms (prefetched four behind/eight ahead), then the sharp display replaces it. The display tier is prefetched 32 ahead/12 behind inside the current batch, plus eight-frame heads of the neighbouring batches, because eight display workers (the automatic cap on this 8-core M1 Pro) measure 89.4 frames/s at 2048 px and 82.4 at the 3240 px 4K fit (4.1–4.5× the 20 Hz budget; the 5076 px 6K fit still measures 62.4 frames/s). The exact develop is queued only after the cursor rests for 250 ms. Display requests are fitted to the pane's physical width×height, and the decoder picks the smallest embedded JPEG that covers that target: a 3:2 photo on a 16:9 4K pane is 3240 px (not the screen's 3840), while a 1440×900 1x laptop pane asks for 1350 px, which the 1620 px `PRVW` already covers, so the 6000×4000 entropy decode is skipped (638 frames/s on Game1JENKS and 581 on Game3KC with eight workers). A 2× zoom asks for 2700 px, not the whole sensor. The decoded RGBA buffer is allocated 4-byte aligned and the decode worker adopts it as egui's `Color32` without a per-pixel conversion; a texture that fits the device limit shares the decoder's `Arc` instead of cloning it, keeping a 4.1 ms conversion and a 0.6 ms copy off the paint thread per 3240×2160 frame. Progress is never cancelled by navigation and exact is never allowed to starve it |

## 3. What survives, what dies

**Survives (the crown jewels, moved not rewritten):**
- `core/firstcut-core` modules: scan, meta, formats (CR3 and every other format reader),
  ordering/batching (metadata then visual signature), store/session DB, XMP, fileops,
  rename reconciliation. ~400 green tests port with them.
- `core/firstcut-cli` — becomes `apps/firstcut-cli` (see §5). Its `verify`, `contact-sheet`,
  `bench`, `scan` subcommands become commands in the registry reachable via `run`, kept as
  convenience subcommands where clearly better than `run`.
- `tests/fixtures/` (exiftool dumps, header fixtures) and the opt-in real-photo protocol
  (`FIRSTCUT_TEST_PHOTOS`, `FIRSTCUT_ALLOW_PHOTO_TESTS=1`) unchanged.
- Ground truth batching data rules (§0.2 of todo.md): the owner's boundary ground truth task
  moves to the v2 checklist (§7) — agents still never synthesise it.
- App icon: `logo/firstcut-icon.svg` PNG pipeline keeps working; icon paths per-platform in
  packaging. The mark is v1's, unchanged: `assets/branding/firstcut-128/256/512/1024.png` are
  v1's own appiconset exports copied byte-for-byte (so the 16/32 px simplified art is kept), and
  `firstcut.icns`/`firstcut.ico` are built from those exports.

**Port reality notes, M1 (written with the port commit, not after):**
- v1's `store/identity.rs` used `libc` `statfs`/`getuid`. The workspace's `unsafe_code = "forbid"` (§4.5)
  rules libc out, so v2 uses `nix` (`statvfs`, `getuid`); the volume identity string is
  byte-identical on little-endian, and macOS's `f_mntonname` is replaced by a `st_dev`-based
  ancestor walk shared with `firstcut-fileops::volume_root`. Windows/non-macOS paths return
  `None`/`Unsupported` until M5 (documented in code).
- The §4.5 lint sweep while porting turned v1's hand-checked `expect`s/indices into checked reads
  (parsers skip a truncated field instead of crashing). Test-name parity: the workspace runs 418
  green (+3 opt-in ignored, per §9) against v1's 426; the delta is the v1 FFI/uniffi tests that die
  with `ffi.rs` (D6 above). Two id round-trip tests moved from `batch` into `firstcut-catalog::db`
  where the store lives now, and the xmp "symlink is claimed, not followed" test keeps its race
  retry but now also cleans a contended attempt's output (v1 left the made sidecar behind).
- `firstcut-meta` grows a non-default `test-fixtures` cargo feature so `firstcut-engine`'s tests
  can reuse the exif byte builders; product builds carry none of it.
- v1's release-pipeline scripts and workflows (`build-app.sh`, `build-core*.sh`,
  `generate-project.sh`, `with-timeout.sh`, `common.sh`, `package-release.sh`,
  `bump-cask.sh`, `third-party-notices.sh`, `build-contact-sheet.sh`, and the
  `ci/xcode26/release/screenshots` workflows) die with the Swift tree per this section; §6's
  packaging story is rebuilt for the v2 native app at its own milestone.

**Dies on v2:**
- The entire `App/` Swift tree (AppKit/SwiftUI app), `project.yml`/xcodegen, UniFFI bridge
  (`ffi.rs`, generated bindings, conversion layer), Swift CI/test lanes, `swift-format`,
  `with-timeout`'s TCC hang mitigations tied to the GUI test host, xcodebuild-based
  distribution scripts, Sparkle-class updaters (deferred, §8), and the §0.4
  Xcode-16.4-vs-27 toolchain-trap notes (moot).
- macOS-26-only presentation code (Liquid Glass).

**Fixes by construction (not ported defects):** the four known `ImageProvider` defects from
the 2026-10-05 audit — in-flight slot leak (`store(_:_:)` discard branch), request-recorded-
as-result, eviction crossing tiers, histogram path dropping EXIF orientation — are written
as engine tests at build time. Each fails if regressed.

## 4. Architecture

### 4.1 Crate ladder

One-way dependency ladder; `cargo xtask layers` fails CI on any upward or sideways edge. The
allowed edges live in exactly one file in `xtask` (`xtask/src/layers.rs`), not in prose.

```
crates/
├── firstcut-types       # ids, geometry, sizes, errors, event types (pure)
├── firstcut-meta        # RAW container + metadata readers from specs (existing core code)
├── firstcut-xmp         # sidecar read/write, ratings, keep labels (existing)
├── firstcut-fileops     # move/copy/delete, path containment, typed-DELETE gate (existing)
├── firstcut-batching    # visual signatures, ordering, boundaries (existing)
├── firstcut-catalog     # session DB + key-value storage behind one storage trait
│                        #   (native target: a plain folder on disk)
├── firstcut-decode      # decode trait + priority window + tiers; backends attach, never bake in:
│                        #   - native backend: LibRaw bindings (C++; lives in its own crate
│                        #     firstcut-decode-libraw, the only `unsafe` crate in the workspace)
│                        #   - embedded preview fast path: zune-jpeg + core's container walkers
│                        #   - CPU fallbacks exist for anything the GPU path does (same results,
│                        #     tested against each other within stated tolerance)
├── firstcut-engine      # command registry, state, undo/redo, background jobs, event stream.
│                        #   The UI never mutates state except by dispatching commands.
├── firstcut-ui          # egui/eframe crate: windows, panels, theme, widget ids (all stable strings)
apps/
├── firstcut             # thin desktop entry: wires engine → ui (mac/win/linux)
└── firstcut-cli         # `run`, `mcp` (--connect host:port), `--control` socket lives in app
```

Every crate: own tests, one-paragraph purpose comment at the top of `lib.rs`.

Workspace root: `Cargo.toml` (workspace deps + lints), `Cargo.lock` committed,
`rust-toolchain.toml` (pinned stable), `rustfmt.toml`, `clippy.toml`,
`.cargo/config.toml`, `AGENTS.md`, `CLAUDE.md` (pointer to AGENTS.md), `README.md`,
`ROADMAP.md`, `docs/`, `assets/` (+ `ATTRIBUTION.md`), `packaging/`, `xtask/`.

### 4.2 The decode pipeline (the heart of the port)

`firstcut-engine` owns the port of the v1 Swift `Pipeline` redesign — priority queue (current
photo, its batch, next, previous), cancellation when the user moves on, memory budget with
LRU eviction **per tier** (leak, D6 note), memory-pressure shedding, background visual
signatures shared with the thumbnail pass (one decode each, whichever gets there first),
first-photo fast path (header + preview on screen while the scan runs), reconcile on folder
events, and the on-demand **Exact RAW** develop tier as its own cache tier (current photo
only, dropped on focus change).

Tiers: thumbnail → **navigation preview** (the camera's small `PRVW` JPEG at native
size) → display (T2 display-sized) → Exact RAW (T4 full develop via rawler). The preview
tier exists for a held arrow key: one decode is ~6–8 ms, so it lands inside a 33–50 ms key
repeat even when the ~57–68 ms display decode has not; it is prefetched four behind and
eight ahead inside the current batch. The display tier carries the sharp frames: 32 ahead
and 12 behind (bounded by its memory budget), plus eight-frame heads of the next and
previous batches, because one display decode is slower than a 50 ms 20 Hz frame but eight
workers have 89.4 frames/s at 2048 px and 82.4 at the 3240 px fit. Each display request is **fitted to the viewer's
actual backing width×height**, not to one long edge: a 3:2 photograph on a 16:9 4K pane is
3240 px, not 3840, and a portrait frame is 2160 px. `firstcut-decode` then reads the smallest
embedded JPEG that covers the target, so a 1440×900 1x laptop pane (1350 px) is answered by
the camera's 1620 px `PRVW` without touching the full-preview entropy decode, while a 4K or
Retina pane still uses the full preview. The reduction hits the requested edge exactly
rather than an integer stride that could undershoot by 22% (a 512 px request gave 400 px
before). Zoom is sized the same way: 2× a 1350 px fit asks for 2700 px, and only the native
zoom reaches the 6000 px sensor. T4's memory (≈92 MB/frame at 6000×4000) is the tier's own
budget, as in v1, and its demand is queued last — the app additionally defers it for 250 ms
after the cursor stops so a develop's all-core ~0.29 s never slows navigation.

**Exact RAW backend (D11, revised in M5):** `rawler` 0.8 is the developer on every
platform; LibRaw remains the macOS/Linux fallback and the pixel cross-check. rawler is
pure Rust, so Windows now gets real sensor data instead of the embedded-JPEG fallback the
M2 note recorded. A rawler refusal (or a panic inside the dependency, caught at the
boundary) falls through to LibRaw on macOS/Linux; on Windows it is a typed error and the
loupe keeps the display preview. The unsafe carve-out is unchanged and still covers only
the LibRaw FFI.

**Progressive Exact RAW (D12/D13):** enabling the tier queues the develop, but the loupe
keeps painting the sharp display (or, while scrolling, the faster navigation preview)
until the 92 MB sensor frame arrives and swaps in place. The user never waits on a blank
viewer; the benchmark still reports the develop's own time separately so the performance
gate stays honest. The develop is queued only after the cursor has been still for 250 ms,
because it temporarily takes every core and would otherwise starve the previews that keep
a held arrow key smooth.

**Non-RAW display is a known gap (recorded 2026-10-30, not claimed as done).**
`decode_display` has a byte range to read only for a RAW's embedded JPEG and for a plain
JPEG, which is its own image. The scanner still accepts HEIF/TIFF/PNG as photographs — they
get metadata, sidecars and a session row — so a folder of only `.heic` files opens and then
every frame reports a decode error instead of painting. v1 got those files from ImageIO, and
its parity list says a JPEG/HEIF-only folder must cull like a RAW folder (todo.md:978).
TIFF/PNG can be closed with the `image` crate already in the workspace (pure Rust, no new
`unsafe`). HEIF needs a decoder, and that is an owner decision: libheif (a C dependency and a
second `unsafe` carve-out, plus a dylib to ship on Windows and Linux), the three OS-native
decoders (ImageIO / WIC / libheif, three unsafe FFI backends), or declaring HEIF out of scope
and dropping it from the scanner so it never appears as an undecodable photo. Until one is
chosen the hole is listed in `docs/parity-checklist.md`, not worked around.

EXIF orientation is applied exactly once, in the decode backend; a develop must not
re-apply it. This rule is a named test.

### 4.3 Every action is a command

The registry: every user-visible action has a stable string id and typed parameters
(JSON-serializable), one dispatch entry point, and is undo-able (a burst — slider drag,
scripted loop, multi-photo reject — is one undo step). Menus, the shortcut cheat-sheet and
the MCP tool list are **generated** from the registry; nothing hand-maintains a second list.

**View state is not engine state.** Loupe/grid/compare, panel/overlay/HUD visibility, zoom
and pan are the screen's own fields (v1 kept `hudVisible` and friends on its app model too).
The View menu and the keymap's `view.*` / `panel.*` / `overlay.*` / `hud.toggle` ids change
those fields directly. Everything that changes ratings, navigation, files or settings still
goes through `registry::dispatch`, which is the rule this chapter protects.

Surfaces:
- **UI**: widget ids are stable strings; menus/panels dispatch commands only.
- **CLI**: `firstcut-cli run <command> key=value …` chains commands headlessly, one JSON
  result per command. Convenience subcommands (`verify`, `contact-sheet`, `bench`, `scan`)
  preserved where clearer than `run`.
- **Control channel**: `firstcut --control <port>` — JSON lines on localhost. Methods:
  `engine.execute`, `engine.commands`, `engine.state`, `ui.widgets`, `ui.clickWidget`,
  `ui.pointer`, `ui.screenshot`. Protocol documented in `docs/control-protocol.md`; the agent
  can operate the real UI, not a side door.
- **MCP**: `firstcut-cli mcp` exposes the registry as MCP tools; headless or `--connect
  host:port`; screenshots returned as images. Documented in `docs/mcp.md`.
- **Headless UI snapshots** for tests: render a UI state to an image without a window.

Agent-driven screenshots produce the README images; scripts live under `docs/showcase/`.

### 4.4 Engine owns what v1 kept in "app-logic"

Ported into the engine (chapter by chapter, not all in M1): AppModel/navigation/rating
rules/undo bridging, keymap (DefaultKeymap.json + recorder + import/export), settings model
and persistence, session backend calls (the v1 UniFFI conversion layer collapses into plain
Rust), Finish Cull flow (summary → options → dry run → typed DELETE → report → undo),
live folder watching, flush on quit, welcome/recents/resume.

- **Keymap (M3).** `firstcut-engine::keymap` owns the map: the shipped
  `crates/firstcut-engine/resources/DefaultKeymap.json` compiled in with `include_str!`, plus
  `keymap.json` overrides beside `settings.json`, read at start and written atomically by the
  recorder. Layering is v1's exactly: a user binding replaces the default on the same chord and
  every default chord of the same command; mode-specific rows (`flag.pick` in Stars,
  `keep.toggle` in Keep) beat a global row on the same chord. The window routes every key
  through `Keymap::binding`, and menus, the cheat sheet and Settings → Keyboard read the same
  map, so a rebound key is never shown twice. Two honest gaps from the platform: egui has no
  Caps Lock transition, so v1's default Caps Lock binding for `settings.autoAdvance.toggle`
  round-trips through the file but never fires (record another key); egui reports keypad Enter
  as Return, so both default `cull.finish` chords resolve to the same event.
- **Finish Cull (M4) is a typed stage machine in the engine.** `firstcut-engine::finish::FinishStage`
  carries confirm → summary → options → dry run → report/failed; the model builds the dry run with
  `Session::plan_finish` and walks it with the logged `Session::execute_finish`, so the UI cannot
  re-decide what a keep is or skip the typed gate. Two deliberate differences from a literal v1
  transcription: there is no painted `executing` screen (execution is synchronous, as v1's backend
  call was, so the screen would never render), and the typed `DELETE` is enforced inside
  `Model::execute_finish` as well as in the sheet, because `finish.execute` is reachable from the
  CLI and the control channel. The sheet's defaults live in Settings → General; a run that moves
  every photo out returns to Welcome when the sheet closes.
- **Recent shoots (M3) have no `recents.json`.** Each shoot's session database already carries its
  folder, photo count and last-open time, and reopening it restores the cursor, so the Welcome
  list is a read-only sweep of `Sessions/*.sqlite` (`firstcut-catalog/src/recents.rs`). A folder
  that has gone is not offered and an unreadable database is skipped; choosing a row is the same
  asynchronous `Session::open_in` path as the File → Open dialog.

### 4.5 Safety rules (AGENTS.md carries these verbatim in spirit)

- **Never crash outranks feature work.** Non-test code: no `unwrap()`, `expect()`, `panic!`,
  `todo!`, no `unsafe` (workspace lints: `unwrap_used`, `expect_used`, `panic`,
  `indexing_slicing`, `unsafe_code = "forbid"`). The **single exception** is
  `firstcut-decode-libraw`, the approved LibRaw bindings crate: `unsafe is allowed there and
  nowhere else`, enforced by a documented lint exception on that one crate in
  `xtask/src/layers.rs` and the workspace CI task.
- Checked indexing/arithmetic on anything derived from input or files. Every crash fix ships
  a regression test.
- Existing core code is swept to these lints early (M0/M1), not flagged "later".
- Clean-room: no proprietary binaries read, no copyleft sources for what we reimplement —
  work from public specs (this is already how the v1 core metadata readers were built and
  their sources recorded).
- No proprietary assets ever; every icon/font/image has an `assets/ATTRIBUTION.md` entry in
  the commit that adds it.

## 5. Numbers the port must meet (measured, never estimated)

The v1 §7.3 practice carries over: a performance claim needs a measurement on this machine
(M1 Pro) against the real photos. The port is not "done" on a moving target — it is done when
these rows, measured with `firstcut-cli bench` (extended for v2), are within 1.5× of the v1
baselines or better, on macOS:

| Row | v1 measured | v2 gate |
| --- | --- | --- |
| First photo on screen after folder open | ~0.5 s× | ≤ v1 |
| Arrow → next photo (embedded preview) | ~0.09 s× | ≤ v1 (zune-jpeg path) |
| Exact RAW develop, first frame | ~0.079 s× | ≤ 1.5× v1 (LibRaw's first run on new hardware is cold; warm-cycle measured like v1's alternating-rounds protocol) |
| 100% zoom drag on a 6000×4000 frame | scroll-to-pixel latency, no drops reported | v1-parity or better on the GPU path |

The cold-file-cache trap is the standing rule: **whichever decode runs first pays for the
read**. Warm or alternate before believing a ratio. `waitUntilIdle`-class invariants get
dedicated tests (v1's biggest gap).

### 5.1 Measured on this machine (v2, 2026-10-30)

`firstcut-cli bench --decode --folder <game> --repeat 3`, release build, Apple M1 Pro,
idle machine, real CR3s. These are decode-only timings (no GPU upload), which is why the
zoom row is labelled as a lower bound. The five throughput rows are the median of three
runs of 48 decodes each. The cold first-touch row is the one row that depends on the page
cache rather than on the code, so it keeps its cold reading (a warm-cache rerun of the same
build measures 0.081 s, of which 0.021 s is the scan).

| Row | Game1JENKS (708) | Game3KC (920) | Gate | Verdict |
| --- | --- | --- | --- | --- |
| First photo after open (cold first-touch) | 0.240 s (scan 0.179 + display 0.061) | 0.294 s (scan 0.233 + display 0.061) | ≤ 0.5 s | **OK** |
| Arrow → next photo (7 files, median) | 0.057 s | 0.068 s | ≤ 0.09 s | **OK** |
| Arrow → navigation preview (7 files, median) | **0.006 s** | **0.008 s** | ≤ 0.033 s (a 30 Hz held key) | **OK** |
| Display throughput, 2048 px (8 workers, 48 decodes) | **89.4 frames/s** | **87.7 frames/s** | ≥ 20 frames/s (a 20 Hz held key) | **OK** |
| Display throughput, 1350 px laptop fit (PRVW path) | **638 frames/s** | **581 frames/s** | ≥ 20 frames/s | **OK** |
| Display throughput, 3240 px 4K 3:2 fit (full preview) | **82.4 frames/s** | **82.3 frames/s** | ≥ 20 frames/s | **OK** |
| Display throughput, 4320 px 5K 3:2 fit (full preview) | **74.9 frames/s** | **73.6 frames/s** | ≥ 20 frames/s | **OK** |
| Display throughput, 5076 px 6K 3:2 fit (full preview) | **62.4 frames/s** | **53.2 frames/s** | ≥ 20 frames/s | **OK** |
| 1:1 zoom re-decode (embedded full preview) | 0.068 s | 0.069 s | v1 parity on GPU | decode half only |
| Exact RAW develop (fused plan, warm) | **0.257 s** (decode 0.12 + demosaic 0.094 + pack 0.024 + orientation 0.015 + rescale 0.004) | **0.257 s** | ≤ 0.119 s (1.5× v1) | **OVER TARGET, owner decision** |
| Exact RAW first pixels (progressive display) | 0.057 s (the display decode the loupe already paints) | 0.068 s | ≤ 0.119 s | **OK for first frame** |

Held-arrow navigation now has two bounds. The 6–8 ms preview is prefetched four
behind/eight ahead, so every 33–50 ms repeat paints real pixels even before the sharp
frame is ready. The 57–68 ms display decode is still slower than a 50 ms 20 Hz frame, but
eight workers measure 89.4 frames/s at the 2048 px baseline and 82.4 at the 3240 px 4K
fit, so the display tier is prefetched 32 ahead/12 behind inside the current batch and
eight-frame heads of the neighbouring batches; after roughly half a second of lead-in the
sharp frame is cached before the cursor reaches it. The exact develop still waits for a
250 ms settle. That is the D13 behavior; the arrow row above remains the single
display-tier number the §5 gate names.

The window's pixel handoff is now zero-copy. `firstcut-decode` allocates its RGBA buffer
4-byte aligned (`RgbaBuffer`, backed by `Vec<u32>`) and, on the JPEG path, decodes straight
into it instead of accepting the decoder's own `Vec<u8>`. The decode worker reinterprets
that buffer as egui's `Color32` with `bytemuck::try_cast_vec` (no per-pixel conversion; a
mismatched buffer falls back to an empty image rather than panicking), and
`load_texture` passes the decoder's `Arc` through when the frame already fits the device
limit instead of cloning it. On a 3240×2160 frame that removes a measured 4.1 ms
conversion plus a 0.6 ms copy from the paint thread; an end-to-end 32-decode worker probe
measured 83.1 fps at 2048 and 74.9 at 3240 (77.7/68.8 with the old convert-and-clone path),
and the texture cache now holds the same `Arc` the decode cache holds rather than a second
full copy.

The display target is now a fit, not a fixed long edge. A 3:2 photograph on a 16:9 4K pane
occupies 3240×2160; asking for 3840 was 15.6% wider than the pane and, under the old
integer-stride reduction, actually produced 3000×2000, which the loupe then upscaled by 8%.
The exact 3240 target removes that softness and makes the budget estimate honest. A
1440×900 1x laptop pane asks for 1350 px, which the camera's 1620 px `PRVW` already covers,
so the display tier never pays the 6000×4000 entropy decode. That path measures 638
frames/s with eight workers versus 82.4 frames/s at the 4K fit, and the 5K/6K fits
(4320/5076 px) still measure 74.9/62.4 frames/s on Game1JENKS (73.6/53.2 on Game3KC). All
are above the 20/s held-key budget. The GPU texture cache is capped at 768 MB and trims
least-recently painted frames, because 28 MB per 4K texture would otherwise grow without
bound on a long hold even while the CPU pipeline stays inside its budget.

**The Exact RAW row is now 0.257 s, and 0.119 s remains unreachable on this CPU.**
`bench --decode --stages` breaks the develop into its own passes on one file in one process;
warm on a 24 MP CR3 at 6000×4000, 8 threads, it measures:

| Stage | Seconds | Share |
| --- | --- | --- |
| CRX decode (`decode_file`) | 0.120 | 47 % |
| demosaic (fused PPG) | 0.094 | 37 % |
| sRGB encode + RGBA pack | 0.024 | 9 % |
| orientation | 0.015 | 6 % |
| rescale u16 → f32 | 0.004 | 1 % |

The tail went first (0.343 s → 0.286 s): rawler's `SRgb` step (a `powf` per channel, 44 ms),
the f32→RGBA pack (12 ms), the orientation rotate (32 ms) and rawler's `CropDefault` copy
(96 MB) are all gone — the sRGB curve is a 16-bit-indexed table built from rawler's own
`srgb_apply_gamma`, the crop rectangle is read in place, and the encode and the pack happen
once, destination-row parallel.

The demosaic went next (0.286 s → 0.257 s). rawler expands the CFA into a 288 MB interleaved
f32 RGB buffer and then walks it four times; `crates/firstcut-decode-libraw/src/fused.rs`
keeps only the three planes the demosaic produces, as three row-parallel sweeps — green, then
the pixel's own colour plus red/blue at the green pixels, then the channel missing at every
red/blue pixel — and each sweep fetches the five rows it reads once and walks its half of the
row with a stride of two, so the inner loops do no index arithmetic and no per-pixel CFA
lookup. The output pass then reads three complete rows, applies white balance and the camera
matrix, and encodes and packs in the same loop. The rotate is now one affine walk of the
source per destination row instead of a match and two checked multiplications per pixel
(0.033 s → 0.015 s). Demosaic 0.099 → 0.094, tail 0.055 → 0.039.

That equality is not approximate: the unit tests compare the fused demosaic against rawler's
own PPG pixel for pixel for all four Bayer orders and for an active area that does not start
at the frame origin, and the opt-in real-photo gate still holds at one 8-bit level (measured:
max difference 1, 238 463 of 24 M pixels). The fused plan accepts a file only when it has been
proven on it — single-component Bayer, no Fuji rotation, a matrix it can size, a crop inside
the active area — and returns `None` otherwise, so a refusal falls back to rawler's pipeline
instead of erroring.

What is left is one row and a floor. The 0.120 s CRX decode is now the largest single stage,
and it is rawler's own scalar entropy/wavelet decode: only a custom CRX decoder attacks it.
The demosaic is at parity with rawler's algorithm but is now division-bound — PPG's hue
transit does a real division per interpolated pixel, roughly 36 M of them across a 24 MP
frame — so no amount of tiling or fusion removes it. The tail is 0.039 s. The floor with this
algorithm on this CPU is therefore ~0.24 s; 0.119 s needs the decoder, not the pipeline.
LibRaw measured 1.03–1.05 s (unpack 0.20 + AHD 0.66 + RGBA 0.02 + orientation 0.14), and
Apple's `CIRAWFilter` measured ~0.44 s for the first full render (~0.18 s warm `toBitmap`) —
both slower, and the v1 0.079 s comment did not reproduce.

What ships is the fastest honest pair: 0.257 s for the full sensor develop (LTO fat in the
release profile), and progressive first paint so the navigation preview is on screen in
0.006–0.008 s, the sharp display in 0.057–0.068 s, and the sensor frame replaces it in place
when it lands. Faster LibRaw demosaics (PPG 0.30 s, linear 0.34 s) are deliberately not chosen:
they visibly degrade pixels, which trades the point of the Exact RAW tier for a benchmark.
Owner options: accept 0.257 s with progressive first paint, fund the custom CRX decoder above,
or redefine the row as first-frame latency. Recorded here so the gate stays honest.

## 6. Milestones

Branch `v2`. Commits are small and one-thing; pushed right after. **Tests run when they
matter, not after every commit** (§9): the rules in this section name the gate where a suite
runs; do not re-run the whole workspace between gates. GitNexus `detect_changes` before
committing while the index is fresh (re-index after big moves; the index is machine-local,
never committed).

### M0 — Scaffold (branch setup, this file landed first)

- Workspace in the layout of §4.1; lints + toolchain + `rustfmt` + `clippy`.
- `xtask` with these tasks, live and green on this machine:
  - `ci` — the full gate: fmt check, clippy (deny warnings), workspace tests, layering
    check, asset-attribution check. Writes `target/ci/report.json` + `summary.md`
    (hardware, skipped tests with reasons, a PR-pasteable block). Picks job/thread counts
    from the machine's CPUs and RAM so it never OOMs. Nothing is uploaded.
  - `layers` — reads workspace metadata, fails on any upward/sideways edge.
- GitHub Actions matrix green (3 OS), calling `cargo xtask ci`.
- App opens a native window (egui shell, empty) on **macOS, Windows, Linux** — the matrix
  runner is where cross-OS window creation is actually proven, not by hand here first.
- `AGENTS.md` rewritten (rules verbatim-in-spirit from the kickoff prompt + the §4.5
  exception for `firstcut-decode-libraw`); `CLAUDE.md` points at it; GitNexus re-indexed
  (`analyze --embeddings --pdg`) right after the restructure.
- `main` untouched; branch pushes to `origin/v2`.

### M1 — Vertical slice through every surface

One real feature, end to end: **open a real folder → scan/batch → filmstrip → loupe of the
current photo → rate 1–5 → undo**, through: UI, CLI `run`, control channel, MCP, undo, tests
per leg, one headless-UI snapshot, one README screenshot. Also lands in M1 because everything
downstream needs them: settings persistence (read v1's settings JSON and translate), and the
parity checklist file live in `docs/parity-checklist.md`.

### M2 — Full pipeline

Decode priority window, cancellation, tiers incl. Exact RAW (LibRaw), memory budget +
per-tier LRU, visual signature sharing, reconcile-on-watch with `notify`, histogram (EXIF
orientation test), `bench` extension for the §5 rows. The four audit defects become engine
tests at this milestone before any of the other code lands.

### M3 — Full screen set

Grid; 2/3/4-up compare with synced zoom; info panel + histogram; progress HUD with the
current rating; both rating modes (stars / keep with the 1-star keep semantics and the
split `keep_rating` / `keep_import_rating` thresholds); batch navigation incl. double-click
batch jump and arrows crossing batch edges; zoom lock; AF + clipping overlays with the
v1 thresholds (250/255, 5/255); keyboard recorder + keymap import/export; menus;
Welcome with recents + resume; Settings (General, Keyboard, Viewer, Metadata, Performance);
error alerts; live folder watching; flush on quit.

### M4 — Finish + persistence edge cases

Finish Cull sheet end-to-end with undo; Reject Whole Batch (opt-in, off by default);
rename reconciliation; sidecar import at every open + rescan; shoot-rename/recovery paths;
session DB carried forward (v1 schemas v3 history.group_id included); cross-volume copy
safety; the §0.5 2026-10-05 audit fixes re-verified by their tests.

### M5 — Cross-platform hardening

Windows (paths, long paths, watcher, DPI, fonts, signers-optional), Linux (Wayland + X11,
old-driver GL fallback for wgpu, fonts, glibc floor for the AppImage), mac (document-folder
access rules — same `NSOpenPanel`-equivalent via `rfd`), file dialogs on all three, bundle
selection behavior for culling a card.

Landed: Inter is bundled and installed on the first culling frame (D10); `rfd` provides the
native folder picker and the UI only raises a `browse_request`; a camera-card root with
several photo-bearing rolls asks “Choose a Shoot”; `FIRSTCUT_WGPU_BACKEND=gl|vulkan|metal|dx12`
is the Linux old-driver escape hatch; `firstcut-meta` has a scan test past 260 characters;
the macOS `Info.plist` (M6) carries the document-folder usage strings; and rawler closes
Windows' Exact RAW gap. Windows DPI is winit's per-monitor awareness; the watcher and the
long-path test are exercised by the 3-OS matrix, not by hand on this Mac.

Wayland is the one platform path the headless matrix cannot prove. `winit` selects Wayland
when `WAYLAND_DISPLAY` is set and X11 otherwise, and `FIRSTCUT_WGPU_BACKEND=gl` is the
old-driver escape hatch, but a GitHub runner has no DRM device: Weston starts headless, then
Mesa cannot create a Wayland EGL surface and lavapipe exposes no Wayland WSI, so wgpu fails
with `Failed to create surface for any enabled backend` (attempted in run 39894441419 and
removed from CI). The X11 smoke is the automated proof; a real Wayland desktop and the GL
fallback are owner checks. This is recorded rather than silently claimed.

### M6 — Packaging + docs + merge gate

- `cargo xtask package` per OS: mac `.app` (+ release zip, cask stamp), Windows zip, Linux
  AppImage + `.deb`.
- Release workflow: 3-OS matrix, artifacts, SHA-256 sums; tag `v2.0.0` when the owner says.
- Docs re-land: new `README.md` (short, honest, feature-status table), `ROADMAP.md`,
  `docs/control-protocol.md`, `docs/mcp.md`,
  `docs/parity-checklist.md` (§7), `todos carry-forward`. GitNexus wiki regenerated.
- `CHANGELOG.md` gets the v2 entry; README claims only what a test proves.

Landed: `cargo xtask package` writes `target/package` (macOS app/zip/cask/SHA-256, Windows
zip, Linux AppImage + `.deb` with a `.tar.gz` fallback) and `.github/workflows/v2-release.yml`
runs that on the 3-OS matrix for an owner-pushed tag; the README, roadmap, changelog,
control-protocol, MCP and parity docs are rewritten. The package task is not part of
`cargo xtask ci` (it is a release step, not a per-commit gate). The workflow was exercised on
all three OSes by a `v2` push (`v2-release` run 39889169123): macOS produced a signed-ad-hoc
`Firstcut.app` zip and cask, Windows a zip, Linux an AppImage and a `.deb`, each with SHA-256
and a `SHA256SUMS-*`. Linux builds on `ubuntu-22.04` so the promised glibc 2.35 floor is the
binary's real floor, not the runner's.

### Merge gate (owner does this)

1. Parity checklist fully checked (§7), §5 rows measured and recorded.
2. All three OS packages build and run from artifacts.
3. `cargo xtask ci` green on the 3-OS matrix; branch has no red commits. (History note:
   `c77f97d` and `df96acc` are CI-only Wayland experiments that hung/failed on the headless
   runner; `70b3881` removed the step and is green, and neither touched product code. Squashing
   them out needs an owner force-push, which agents do not do. The tip is the gate.)
4. Then fast-forward `main` to `v2` (owner; merge happens with a green suite, repo rule) and
   the branch is deleted.

## 7. Feature parity checklist (the merge gate, mirrored into docs/parity-checklist.md)

Struck-through = proven by a named test; "owner" = needs the owner's eyes (the §0.2 v1 items,
carried over, unchanged in substance):

- [ ] Welcome: recents, resume (autorecognizes a shoot's folder and restores cursor)
- [ ] Open folder: scan + first-photo fast path
- [ ] Loupe: pinch/scroll zoom, click zoom, pan, zoom lock, double-click batch jump
- [ ] AF point overlay; clipping overlay (250/255, 5/255 settings wired)
- [ ] Filmstrip; grid
- [ ] 2/3/4-up compare with synced zoom
- [ ] Info panel with histogram (orientation-safe)
- [ ] Progress HUD with current rating
- [ ] Both rating modes (stars / keep), undo/redo
- [ ] Finish Cull flow: summary → options → dry run → typed DELETE → report → undo
- [ ] Settings: General, Keyboard (recorder/import/export), Viewer, Metadata, Performance
- [ ] Menus + shortcut cheat-sheet generated from the registry
- [ ] Live folder watching + reconcile; flush on quit
- [ ] Rename reconciliation; sidecar import rules
- [ ] Reject Whole Batch (opt-in, off by default)
- [ ] Embedded-JPEG fallback for RAW the backend cannot decode
- [ ] Every format raw.pixls.us lists opens (metadata side unchanged; decode per §4.2)
- [ ] §5 rows measured
- Owner (carried from todo.md §0.2, still needs a person):
  - [ ] Boundary ground truth files (`tests/fixtures/ground-truth/`) — never synthesised
  - [ ] Preview vs Exact RAW default judgement
  - [ ] Game1JENKS re-press pause decision
  - [ ] Lightroom / Capture One sidecar round-trip check
  - [ ] Visual sign-off against Finder-class file galleries (each OS pattern)
  - [ ] Homebrew tap secret; tag `v2.0.0`

## 8. Deferred to post-merge (documented, not forgotten)

- Sparkle-class updaters for desktop.
- Post-v1 features (todo.md §16): manual batch split/merge, importable presets
  (`.xmp`, `.lrtemplate`, `.costyle`).
- Installer signing on Windows; upstream nix flake polish (the flake ships in M0 but is not a
  supported-build lane yet).

## 9. Working rules on this branch

- Read this file + todo.md §0.5 handoff first every session; todo.md stays the handoff
  mechanism (§0.5 rewritten before stopping, in the last commit).
- **Tiered test discipline (owner amendment: tests run when they matter, not every commit):**
  - Per commit: `cargo fmt --check` + `cargo clippy -D warnings` restricted to touched
    crates, plus the touched crates' own unit tests, plus `xtask layers` when crate edges
    could have moved. Seconds to a couple of minutes — this is the commit gate.
  - Milestone gate (before declaring a milestone done, and before any push of milestone-
    boundary commits): the full `cargo xtask ci`, and only then is the milestone "done".
  - Integration tests over the real-photo corpus (`FIRSTCUT_ALLOW_PHOTO_TESTS=1`) are
    milestone-only on this machine by hand/agent choice — never part of the per-commit gate.
  - Never skip a failing test: a failure is either diagnosed before the next commit, or the
    milestone report names it with the reason. Push after every commit regardless — a red
    fix follows as its own small commit.
  - UI snapshots compare with a **2.0 weighted-YIQ per-pixel tolerance** (`kittest.toml`):
    macOS Metal and Linux rasterise the same layout with different antialiasing, which was a
    handful of pixels at ≤ 2.0 after M3. M4's Settings → General snapshot (the Finish section)
    found three glyph-edge pixels just past 2.0 on Linux only, so Linux carries a **3.0**
    ceiling; macOS and Windows stay at 2.0. It is a colour tolerance, not a failed-pixel
    budget — a moved edge still fails, and `max_failed_pixels` is zero on every OS.
- GitNexus `detect_changes` (`--scope all`) before committing; `detect_changes --scope
  compare --base-ref main` before qualifying the merge.
- Real-photo tests stay opt-in (`FIRSTCUT_TEST_PHOTOS`, `FIRSTCUT_ALLOW_PHOTO_TESTS=1`);
  photos never enter the repo.
- A task is not done until something asserts it; a performance number is not done until it is
  measured (§5).
- Prompt rule carried over: when a rule here conflicts with what a task needs, **stop and
  ask** instead of quietly breaking the rule.
