# Plan: fixes and features from the 2026-10-14 review

Everything the owner asked for after the review on 2026-10-14: the review page results, the
screenshots and the answers to the follow-up questions. Nothing here is started yet. Work begins
only when the owner says go, in the order they choose.

Every item follows the project rules in `CLAUDE.md`:
- Run GitNexus `impact` before editing a symbol, and `detect_changes` before every commit.
- Commit small and push after each commit.
- Before every push, run the local checks (`cargo fmt/clippy/test`, `scripts/generate-project.sh` +
  xcodebuild test, `swift-format lint --strict`).
- Update todo.md §0.5 and the contract changelogs when a surface changes.

Release work (README screenshots and GIF, the v0.1.0 tag, Homebrew) is **out of scope** until the
owner says everything is done and ready.

---

## Phase 1: Bugs that break basic use

### 1.1 The key router passes events the wrong way round
**Seen:** ⌘← / ⌘→ skip a batch (92 → 90). ⌘Z after ⇧X also undid the earlier 3-star rating. The
owner couldn't leave a session or finish properly.

**Cause:** `App/Sources/Input/KeyRouter.swift`, in `start()`, uses
`self?.handle(event) == true ? event : nil`. An `NSEvent` local monitor returns the event to let it
continue and `nil` to swallow it, so this is inverted:
- **A handled key carries on to the menus.** Every menu item with a ⌘/⌃/⌥ shortcut runs twice:
  ⌘←, ⌘→, ⌘Z, ⇧⌘Z, ⌘O and ⌘↩.
- **An unhandled key is swallowed.** That breaks:
  - ⌘, (Settings), ⌘Q and ⌘W;
  - every key in sheets and in the Open dialog: typing, Escape and Return;
  - ⌘⇧G in the Open dialog.

Confirmed with a standalone AppKit program: one ⌘← ran the router once *and* the menu once, and
⌘, never reached the menu.

**Fix:**
- Return `nil` when the router handled the event, and the event otherwise.
- Correct the comment in `start()`.
- Add a test that drives the real monitor closure, not just `handle(_:)`.
  - It must prove that a handled key does not also reach the menu.
  - It must prove that an unhandled key (⌘, / Escape in a sheet) does.

**Done when:**
- ⌘← moves exactly one batch.
- One ⌘Z undoes exactly one step.
- ⌘, opens Settings.
- In the Finish sheet you can type, Escape cancels and Return confirms.
- ⌘⇧G works in the Open dialog.

### 1.2 Finish floods the sheet with warnings and loses its buttons
**Seen:** Hundreds of "…/_Not kept does not exist yet, it will be created" lines, and Finish
couldn't be completed.

**Cause:**
- `core/firstcut-core/src/fileops/mod.rs` pushes that warning once **per photo**, in both the kept
  and the not-kept branches.
- `FinishSheet` has no scroll view, so the list pushes Back, Cancel and Finish off screen.

**Fix:**
- Warn once per destination folder: deduplicate in the plan, keyed by folder.
- Make the sheet's middle scroll (warnings and op list), with the header and button row pinned.
- Give the sheet a maximum height that fits the window.

**Done when:** a 700-photo Finish into a new subfolder shows one warning, and the buttons are always
visible.

### 1.3 Leaving a cull session and coming back
**Asked:** "I should be able to do some of a cull session then stop half way through, go to another
one, come back to the original and pick up where I left off."

**Today:**
- Resume already works when a folder is reopened: the cursor and ratings are stored per folder.
- But there is no user-facing way out: `closeSession()` is only called internally.
- The toolbar has no Open button while culling, and 1.1 blocked ⌘W.

**Fix:**
- File → **Close Folder** (⌘W): flush, record the folder in Recents, go back to Welcome.
- File → **Open Recent** submenu, listing the same entries as Welcome with "412 of 708 rated".
- An **Open** button in the culling toolbar.
- Switching folders never requires Finish, and always saves first.

**Done when:**
1. Cull part of folder A, then open folder B and cull part of it.
2. Reopen A: it resumes on the same batch and photo with every rating intact.

### 1.4 Filmstrip never shows Keep-mode rings
**Seen:** In Keep mode the filmstrip showed star badges, and selecting Keep or Not keep didn't
colour anything.

**Cause:**
- `Filmstrip` (`Views/Filmstrip/Filmstrip.swift`) receives `ratingMode` but never passes it on to
  `FilmstripScrollView`.
- So the strip's `ratingMode` stays `.stars` forever.

**Fix:**
- Pass the mode through in `makeNSView` and `updateNSView`, and redraw the badges when it changes.
- Ring rules (owner's choice):
  - green = Keep, red = Not keep, **no ring when unrated**;
  - the current photo keeps its gray plate behind the ring.
- Change `RatingVisuals.keepRingColor` so unrated is "no ring". Today it is red for anything not
  kept.

### 1.5 The HUD grows huge when Reject Whole Batch is on
**Seen:** The HUD became a tall box with a dashed red line down its right edge.

**Cause:**
- The HUD is capped at `Appearance.hudMaxWidth` (520).
- With the extra "Reject batch" button the row overflowed, and the label wrapped one letter per line.

**Fix:**
- The HUD sizes to its content (`fixedSize` horizontally, `lineLimit(1)` on every metric).
- The reject control is a compact icon button with a tooltip and an accessibility label.
- Raise or remove the width cap so it still fits the narrowest window.

### 1.6 Toolbar pills look clipped
**Seen:** "Batch 5 of 5 — Leaf-Aptus22.mos" fills its pill edge to edge, and the ‹ › and sidebar
buttons look cramped.

**Cause:**
- On macOS 26+ the system draws a glass pill around every toolbar item.
- `BatchNavigationCapsule` draws its own `GlassCapsule` inside it (glass on glass).
- `BatchTitle` has no horizontal padding.

**Fix, following Apple's Liquid Glass toolbar guidance:**
- On macOS 26+, drop the custom capsule in toolbar items and let the system group them.
- Separate groups with `ToolbarSpacer`.
- Give the title horizontal padding and a sensible minimum width.
- Keep the current look on macOS 15 (the deployment target).

**Done when:** no text or icon touches a pill edge, at the default and the narrowest window width.

---

## Phase 2: Culling features

### 2.1 Batching by play (Plays mode, the new default) and Bursts mode
**Asked:** "The batcher is supposed to be by play, not burst of photos." Keep today's behaviour as
a second mode.

**Definition (owner):**
- A play runs from the snap to the whistle, including the celebration right after.
- Sideline, crowd and band shots between plays are their own small batches.

**What the data says:**
- 88 of the 91 boundaries where the owner disagreed with the batcher were "batcher split, owner said
  same play".
- Today any gap over 2 s is a hard split (`BatchParams.hard_split_floor_ms`).
- No single time threshold separates plays: Game4VRE goes 126 → 101 → 77 → 68 → 59 → 53 → 47
  batches at 2/3/5/8/12/20/30 s.

**Approach:**
- Keep the current burst batcher as stage 1.
- Stage 2 merges neighbouring bursts into plays with a small scorer over:
  - the time gap relative to the shoot's own rhythm;
  - focal-length change;
  - orientation change;
  - **image similarity from Apple Vision's feature print** (`VNGenerateImageFeaturePrintRequest`:
    a pretrained network, on device, no training data needed).
- Fit the weights and threshold on the owner's labels, validated leave-one-game-out.
- No neural network trained from scratch: four games (≈2,900 photos) is too little data, and it
  would overfit.

**Surfaces:**
- `BatchParams` and a `BatchingMode { plays, bursts }` in the core, saved per folder.
- An FFI to set it, and the contract changelog in `docs/contracts/batching.md`.
- Settings → General: "Batch by: Plays / Bursts". Plays is the default.
- Locks on visited batches still apply: switching mode never splits a batch the user has been in.

**Done when:**
- With round-2 labels (2.2), Plays mode's boundary F1 is reported per game in the CLI's
  `ground-truth` evaluation.
- It beats Bursts mode on the owner's play labels.

### 2.2 Review round 2: labels for the 3–60 s gaps
- Extend `~/Documents/firstcut-review` (`tools/build.py` and `review.html`) with a task covering
  every boundary between 3 s and 60 s, about 300 across the four games.
- Same J / S / Enter keys, with "same play" / "new play" wording that matches 2.1's definition.
- Results go into the same download, as `correctedBatches` per game.
- **Build it, but the owner does it only when they say so.**

### 2.3 Ask how to cull when a folder is opened for the first time
**Asked:** after choosing a folder, ask which batching method and which culling method to use.

- A short sheet shown only the **first** time a folder is opened:
  - **Batch by:** Plays / Bursts;
  - **Rate with:** Stars / Keep–Not keep.
- Both default to the values in Settings.
- Reopening a folder resumes with its saved choices and doesn't ask.
- Both can be changed later in Settings.

### 2.4 Reject Whole Batch (⇧X) skips photos already rated
- `AppModel.rejectCurrentBatch()` only flags photos that have no rating yet.
- Rated photos keep their stars or Keep, and are left unflagged.
- Undo still restores the batch in one step.
- Update the HUD tooltip, the Settings text and the README line.

### 2.5 Batches overview and Go to Batch
**Asked:** a way to reach one of the last batches without holding the arrow key for minutes.

- **Batches overview:** a grid with one card per batch, showing:
  - the cover thumbnail;
  - the photo count;
  - rated/visited status;
  - in Plays mode, the time of the play.

  Click or press Return to jump. Reached from the toolbar's view control and a View menu item with
  its own key.
- **Go to Batch… (⌘L):** type a number and press Return.

### 2.6 C cycles compare modes
- C: single → 2-up (this + next) → 3-up (previous, this, next) → single.
- Keep ⌥C / ⌥⇧C working, or drop them if they conflict.

### 2.7 Make E discoverable
- E means "back to the single photo" (the Lightroom convention). The owner couldn't tell what it
  did.
- Label it "Single Photo (E)" in the View menu and the toolbar tooltip.
- Check that it works from the grid and compare views once 1.1 is fixed.

### 2.8 Folder picker in the Finish sheet
**Asked:** make the destination an interactive folder chooser, like Open Folder.

- A **Choose…** button next to each folder field (not kept / kept / tier root).
- It opens an `NSOpenPanel` with directories only and New Folder allowed.
- The text field stays, for typing a name relative to the shoot or a full path.
- Show the resolved full path under the field.

### 2.9 Type a folder path on Welcome
- A path field on the Welcome screen that suggests folders as you type (`~` expands), with an Open
  button.
- An invalid path shows an inline message.
- The app is not sandboxed, so a typed path can be opened directly.

---

## Phase 3: Image accuracy

### 3.1 CR3 always shows the camera's preview
- The owner prefers Canon's own JPEG ("more detailed").
- Make the camera's full-size embedded JPEG the source for the loupe, compare and zoom.
- The RAW decode becomes an opt-in setting.
- Check that 100% zoom uses the full-size preview, not a downscale of it.

### 3.2 Histogram matches the camera
**Seen:** it doesn't look like the R8's histogram.

**Cause:** in `ImageProvider.histogram(of:)`:
- it is measured after drawing into Display P3 or device RGB, not the JPEG's own sRGB / Adobe RGB
  values;
- each channel is scaled to its own peak (`HistogramView`).

**Fix:**
- Measure from the camera JPEG in its own colour space.
- Default to Canon-style **Brightness** (one white graph), with a click to toggle **RGB**.
- Use one shared scale.

### 3.3 Legacy RAWs with tiny previews
- Leaf MOS (24 KB JPEG), Olympus C5050Z ORF (11 KB thumbnail) and the Nikon Coolscan NEF (869×593)
  look blurry because that is all those files contain.
- The owner chose to accept this: no LibRaw.
- Show a small "Low-res preview" badge in the loupe and the info panel when the shown image comes
  from an embedded JPEG much smaller than the sensor.

---

## Phase 4: Review tooling

### 4.1 Fix the timings instructions
- The review page told the owner to copy one game to `/tmp/fcphotos`.
- `scripts/test-with-photos.sh` expects all four, so `RealRawDecodeTests` and
  `FixtureHarnessTests` failed with "Game1JENKS folder is missing".
- Fix the page:
  - copy all four games;
  - or let the harness run the subset that is present and skip the rest with a clear message.
- Rerun when the owner says so.

### 4.2 Owner checks after the fixes
Recheck the failed review-page steps in the app:
- ⌘← / ⌘→;
- ⇧X then ⌘Z;
- Finish into a new folder, then Undo Finish;
- I / C / G / E;
- the HUD with the setting on.

Lightroom and Capture One sidecar checks are still pending; the owner will do them later.

---

## Owner decisions this plan is built on (2026-10-14)

| Topic | Decision |
|---|---|
| What a play is | Snap to whistle + celebration; between-play shots are separate batches |
| Default batching | Plays; Bursts in Settings → General |
| First-open prompt | New folders only: batching mode + rating method |
| Review round 2 | Build it; the owner does it later |
| ⇧X | Skip photos already rated |
| Keep-mode filmstrip | Green Keep, red Not keep, no ring when unrated, gray plate stays |
| C key | 2-up → 3-up → off |
| E key | Make it discoverable; check that it works |
| CR3 display | Always the camera preview; RAW decode is a setting |
| Histogram | Brightness default, RGB toggle, from the camera JPEG |
| Legacy RAWs | Accept small previews and show a badge |
| Batch jump | Batches overview grid + Go to Batch… (⌘L) |
| Release | Not until the owner says everything is ready |
