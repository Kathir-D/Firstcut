# Manual QA checklist

- **Owner:** qa
- **Scope:** task.md §9 (every screen), §12 (full cull, finish step), plus the safety rules of §9.7.
- **Sign-off:** required for v0.1.0 on **macOS 26 and macOS 15** (docs/agents/qa.md, wave 4).

This is the human pass. The automated suites (`App/Tests/Integration/`, `App/Tests/Performance/`)
prove behaviour; this proves the app *feels* like a first-party Apple app, which no assertion can.

## How to run it

- **Never cull the originals in `~/Documents/testing`.** Copy or hard-link one game to a scratch
  folder first and work there. task.md §9.7 says originals are only moved at the explicit finish step,
  and that step is exactly what this checklist exercises — on a *copy*.
  `FIRSTCUT_TEST_PHOTOS` can point at a scratch root; qa's harness uses it for the automated runs.
- **Both rating modes, all four games**, at least one game start to finish per mode.
- Reference for the look: **Finder's gallery view**, side by side, same window size.
- Record every failure in [`bugs.md`](bugs.md) with a screenshot, then keep going. A checklist that
  stops at the first failure is not a checklist.

Legend: `[ ]` not run · `[x]` passed · `[!]` failed → bug id · `[-]` not applicable on this OS.

## 0. Preconditions

- [ ] `xcodebuild … test` green on the branch under test (unit + integration).
- [ ] Scratch copy of one game exists and the originals' checksums are recorded, so "we did not
      touch the originals" is verifiable afterwards.
- [ ] `~/Documents/testing` permissions: at least one folder made read-only, for the §9.7 safety rows.

## 1. First run and session management

- [ ] Welcome window appears on launch: Open Folder…, recent sessions, drag-and-drop target.
- [ ] Opening a 708-photo game reaches the first photo in under a second, with **no spinner** and no
      progressive sharpening (task.md §7.3).
- [ ] Metadata scan completes in the background; the UI never blocks on it.
- [ ] Provisional batches appear before thumbnails finish; the count settles without the batch the
      user is in ever being re-split under them.
- [ ] Quit mid-cull, relaunch, pick the recent session: cursor, visited batches, and ratings all
      come back (task.md §11).
- [ ] Quit immediately after rating (inside the 1 s XMP debounce window), relaunch: **no rating is
      lost** (session-api.md guarantee: zero DB writes lost).
- [ ] Drag a folder onto the Dock icon opens it.
- [ ] Opening the same folder twice in a row does not duplicate anything.

## 2. Window, toolbar, menu bar (§9.1)

- [ ] Single window, unified toolbar, full-size content view.
- [ ] macOS 26: real Liquid Glass on the toolbar items and on the floating controls (HUD, overlays).
- [ ] macOS 15: `NSVisualEffectView` / `.ultraThinMaterial` fallback, **same layout**, visually
      compared side by side against the macOS 26 build.
- [ ] Menu bar has File, Edit, View, Photo, Window, Help; every command listed with its **current**
      shortcut, which changes after a remap.
- [ ] Toolbar left: `‹ ›` batch navigation in a glass capsule.
- [ ] Toolbar title: `Batch 12 of 148 — IMG_8231`, updating correctly at the ends of the shoot.
- [ ] Toolbar right: Loupe / Grid / Compare segmented control, Info toggle, Finish Cull.
- [ ] Always dark, including when the system is in light mode. Neutral gray photo background.
- [ ] Full screen (⌃⌘F): chrome hides, filmstrip auto-hides at the bottom edge, comes back on move.

## 3. Loupe / viewer (§9.2)

- [ ] Photo fills the area above the filmstrip, aspect-fit, rounded corners like Finder's gallery.
- [ ] **Pinch** zooms smoothly, anchored at the pinch point.
- [ ] **Click** a spot → jumps to 100% centred on that spot, one step. **Click again** → back to fit.
- [ ] A click that becomes a drag pans and does **not** toggle zoom.
- [ ] Zoom in/out uses the system spring, like Photos/Preview.
- [ ] **Zoom lock** on: arrowing keeps zoom level *and* position across frames, and the image stays
      sharp (prefetched neighbours). Toggle in View menu; **no keyboard shortcut** exists (§2).
- [ ] **AF overlay** (A): drawn through the orientation transform, for a frame with rotation and for
      one without. Frames where the AF data says nothing show nothing rather than a wrong point
      (REV-21 — if this row is ever wrong, that is a P1).
- [ ] **Clipping overlay** (J): highlights and shadows highlighted like Lightroom's.
- [ ] **Histogram** (I): updates per frame, matches a known-good reference on a black and a white
      frame.
- [ ] Rotated frames (EXIF orientation 8 — 26/33/166/256 of them across the set) are displayed the
      right way up in every view, with the info panel showing the sensor-space dimensions.

## 4. Filmstrip (§9.3)

- [ ] Shows the **current batch only**; the strip changes on batch navigation.
- [ ] Selected frame on a rounded gray plate; the selection stays visible while arrowing.
- [ ] Scrolls smoothly at 120 Hz with 60+ frames — no dropped frames, no rubber-banding.
- [ ] Stars mode: stars, flags and colour labels all visible.
- [ ] Keep mode: green/red rings, and they read at a glance in a 40-frame strip.
- [ ] Clicking a thumbnail selects it. No drag-reordering (there must be none).

## 5. Navigation and rating (§9.4, §6, §10)

- [ ] ← / → move within the batch; at the ends they stop (default setting).
- [ ] With "continue into next batch" enabled, → at the end crosses into the next batch and selects
      its first photo.
- [ ] ⌘← / ⌘→ and the toolbar `‹ ›` both change batch; **arrow keys never change batch**.
- [ ] Entering a batch selects its first photo (default) / the last one you viewed (setting).
- [ ] Every §10 shortcut works, in both rating modes.
- [ ] ⌘Z / ⇧⌘Z undo and redo rating, flag and label changes; undoing in another batch **navigates
      there first**.
- [ ] **Only the current batch can be rated**: an attempt to rate a photo in another batch is
      impossible from the UI (task.md §2, rating scope).
- [ ] Auto-advance (Caps Lock) moves on after rating; off by default, toggle persists.
- [ ] Stars mode: 5–4 = keep, 3 = good, 2–1 = maybe, 0 = unrated.
- [ ] Keep mode: `P` toggles keep, `X` rejects, and the tiers map to keep / unrated / rejected.
- [ ] A frame with the EXIF orientation applied is rated and undone like any other.

## 6. Grid, compare, HUD, info (§9.5, §9.6)

- [ ] Grid (G): current batch as a grid, ratings/rings visible, `E` or Return returns to loupe.
- [ ] Compare (C): 2-up / 3-up / 4-up, **synchronised zoom and pan**, arrows change the candidate.
- [ ] Progress HUD: batch X of Y, photos left, keeps/good/maybe counts, elapsed time; auto-hides;
      toggles with H.
- [ ] Info panel (I): every field of §9.5 present and correct on a frame with rotation, with a
      lens, and with exposure compensation.
- [ ] Info panel field selection from Settings is honoured and persists across launches.

## 7. Finish Cull (§9.7) — **on a copy, never the originals**

- [ ] Finish is offered automatically after the last batch, and warns about batches not visited.
- [ ] Summary sheet totals match the ratings actually recorded (spot-check three photos by hand).
- [ ] Unkept action: Leave files / mark rejected in XMP / move to `_Not kept` / Trash / delete /
      do nothing — each behaves as named.
- [ ] Kept action: copy / move / split by tier / split by stars / write list / reveal in Finder.
- [ ] **Whole-group moves**: a RAW + paired JPEG + `.xmp` sidecar set moves together, in every mode.
- [ ] Dry-run preview lists exactly the file operations that will happen, before anything happens.
- [ ] Progress and **cancel** work; a cancelled finish leaves a consistent state and is still
      undoable (session-api.md, needs `Progress::cancelled` — REQ-qa-2).
- [ ] Final report matches reality: counts, failures with reasons.
- [ ] **Undo Finish** restores every file to its original path.
- [ ] Trash is recoverable in Finder; permanent delete is not, and the UI says so **before** the
      confirmation dialog.
- [ ] A file already present at the destination is **suffixed, never overwritten**.
- [ ] A volume with insufficient free space is refused with a clear message, before copying.
- [ ] A **read-only volume** produces a graceful error, not a crash.
- [ ] After the finish step: `diff` the scratch copy's file list against the expected result, and
      re-check the checksums of the **originals** — unchanged.

## 8. XMP interoperability

- [ ] A sidecar written by Firstcut is read by Lightroom or Capture One with the same rating, flag,
      and label (spot-check one 5-star keep and one rejected).
- [ ] An existing Lightroom `.xmp` is **merged**, not clobbered (Settings: overwrite vs merge).
- [ ] Import from XMP on open restores ratings for files the DB does not know about.

## 9. Settings (§9.8)

- [ ] Every setting in the four tabs exists, changes something observable, and survives a relaunch.
- [ ] Keyboard tab: record a new shortcut, conflicts are detected, reset restores Lightroom
      defaults, import/export keymap JSON round-trips.
- [ ] Performance tab: memory budget slider changes what the cache keeps; decode threads "auto"
      does not pin a bad count.

## 10. Robustness (each of these is a test, not a vibe)

- [ ] A corrupt or truncated RAW in the folder: the shoot still opens, that photo shows a reason,
      nothing else is affected (photo-meta.md, REV-20 / REV-33).
- [ ] A folder with a file that is not a photo: ignored or reported, never a crash.
- [ ] An empty folder: a clear message, not an empty window with no explanation.
- [ ] A folder with 1 photo, and a folder with 1 batch: the ends of the shoot behave.
- [ ] Renaming a rated photo between sessions: **the rating survives** (REV-15 — a rename must not
      lose work; this is a P1 if it fails).
- [ ] Adding and deleting files in the folder while the shoot is open: `files_changed` fires and the
      view updates.
- [ ] Kill the app with `kill -9` during a cull, relaunch: no DB corruption, no lost DB writes.
- [ ] Kill the app *during* a finish: undo is still offered and still works.

## 11. Performance, on both OS versions

Full numbers and the regression rule: [`perf-baselines.md`](perf-baselines.md).

- [ ] Every §7.3 target met on macOS 26 and macOS 15 (note: macOS 15 may be slower — record both,
      do not silently pass on one).
- [ ] Zero focus misses holding → through a whole game at key-repeat rate.
- [ ] Idle memory after a full cull of 1,500 photos: no growth, no leaks.
- [ ] Arrow key → sharp photo in ≤ 8 ms at 120 Hz, with a ProMotion display if available.

## 12. Accessibility and appearance

- [ ] Every control is reachable by keyboard and has a sensible focus ring.
- [ ] VoiceOver reads the photo name, batch position, and rating in Loupe, Grid, Compare and the
      Finish sheet.
- [ ] Contrast of text on the glass surfaces meets WCAG AA; the histogram and HUD are legible over a
      black, a white, and a mid-grey photo.
- [ ] Full Keyboard Access works with no mouse.
- [ ] Reduce Motion respected: zoom and sheet animations do not move.

## 13. Final sign-off

- [ ] `docs/qa/bugs.md`: no open P0/P1.
- [ ] `docs/qa/perf-baselines.md`: baselines recorded on both OS versions, no metric over budget.
- [ ] Originals in `~/Documents/testing`: checksums unchanged (re-verified).
- [ ] Checklist above: every row `[x]`, `[-]`, or `[!]` with a closed bug.
- [ ] Wave-4 sign-off written in [`docs/agents/qa.md`](../agents/qa.md) and countersigned by
      senior-dev in `docs/review.md`.
