# Contract: AppModel, commands & keymap

- **Owner:** app-logic
- **Consumers:** ui (binds views to it), qa (drives it in integration tests)
- **Version:** v0.2 (draft; frozen as v1.0 at the end of wave 1)

## State the UI binds to

```swift
@MainActor @Observable final class AppModel {
    // Session
    var phase: Phase                        // .welcome, .loading(progress), .culling, .finishing
    var folderName: String
    var allPhotos: [PhotoVM]                // capture order; batches are contiguous ranges of this
    var batches: [BatchVM]                  // index, photo IDs, visited, provisional, range
    var currentBatchIndex: Int
    var currentPhotoIndex: Int              // within the current batch
    var currentPhoto: PhotoVM?              // nil outside a batch; always inside the current one
    var skippedFiles: [SkippedFile]         // never block the cull (photo-meta.md guarantees)
    var lastError: String?                  // window shows it as an alert until dismissError()
    func photos(inBatch: Int) -> [PhotoVM]

    /// False unless the user can act: `phase == .culling && !isProvisional` (AppModel.swift:556).
    /// While the first-photo fast path's frame is up the backend still belongs to the *previous*
    /// folder, so every mutating command would be written to the wrong shoot.
    var canAct: Bool

    // Modes and toggles
    var ratingMode: RatingMode              // .stars, .keep — read from Settings → General
    var viewMode: ViewMode                  // .loupe, .grid, .compare(2...4)
    var infoPanelVisible: Bool
    var hudVisible: Bool
    var debugHUDVisible: Bool               // Settings → Performance; the view pulls the counters
    var afOverlay: Bool
    var clippingOverlay: Bool
    var autoAdvance: Bool                   // Caps Lock overrides the setting for the session
    var viewer: ViewerState                 // zoomed, zoom anchor (normalized), zoom lock, background grey

    // Finish
    var finish: FinishStage                 // .hidden / .confirm / .summary / .options / .dryRun /
                                            // .executing / .report / .failed
    var summary: FinishSummary

    // Progress (HUD / Finish summary)
    var progress: CullProgress              // batch X of Y, photos left, counts per tier

    // Measured frame latency (debug HUD; the only stopwatch numbers in the app)
    var lastFrameLatencyMs: Double?
    var worstFrameLatencyMs: Double?
    var standInFramesPresented: Int          // must stay 0

    var viewportPixelSize: CGSize           // backing pixels, reported by the viewer
    var recents: [RecentFolder]

    let images: any ImageProviding          // the pipeline seam (setFocus)
    func perform(_ command: Command)        // every user action goes through here
    func frameDidPresent(_ frame: PresentedFrame)   // called by the viewer on a committed frame
}

struct PhotoVM { id: PhotoID; fileName; meta: PhotoMeta; rating: Rating; tier: Tier; isKeep: Bool }
enum Tier { keep, good, maybe, unrated, rejected }   // the generated `FfiTier` under its plain name
```

**Tier** is not an app type: it is the generated `FfiTier` aliased in
`App/Sources/Shared/CoreTypeAliases.swift:52`, so the tier totals come from the core's single
`display_tier` and the views cannot disagree with the Finish summary. The mapping is
`RatingRules.tier` (`App/Sources/Session/RatingRules.swift:43-65`), not a literal: the reject flag
wins in both modes; stars mode gives keep at or above `keepThreshold` (default 4, Settings → General),
good at 3, maybe at 1, unrated at 0; keep mode gives keep or unrated, promoting 4–5 stars to a keep on
**display** only, so switching modes cannot make the user's keeps vanish.

**`Phase`** also answers `isLoading` (`App/Sources/Session/SessionTypes.swift:130`), which is what
tells the first-photo fast path "the open I belong to is still running" from "a different folder is
opening now".

**`FinishStage.confirm`** is its own stage rather than an alert so the sheet is the only place a
Finish decision is made and Escape still cancels: Settings → General → "Confirm before Finish" routes
`finishCull` through it (`AppModel.startFinish`, `AppModel.swift:1083`; answered by
`confirmFinish`, `:1091`). The typed `DELETE` gate is a separate thing and
lives on the settings, not the stage: `FinishSettings.requiresTypedConfirmation` is set by the model at
the dry-run stage from "this run's unkept action is a permanent delete, the setting is on, and the
plan has operations" (`AppModel.runFinishDryRun`, `AppModel.swift:1119-1120`), so the sheet draws the
gate instead of deciding it in a private view no test can reach.

## Commands

Every action is a `Command`. Keys, menus, toolbar buttons, and gestures all call
`AppModel.perform(_:)`. **Zoom has no key by default.**

| Command ID | Default key | Notes |
| --- | --- | --- |
| `photo.previous` / `photo.next` | ← / → | Within the batch; behavior at the ends comes from settings |
| `batch.previous` / `batch.next` | ⌘← / ⌘→ | Also `[` / `]`, and the toolbar ‹ › buttons |
| `rate.stars(0...5)` | 0–5 | Stars mode; 0 clears |
| `rate.starsAndAdvance(1...5)` | ⇧1–⇧5 | Always advances, auto-advance or not |
| `flag.pick` | P | Stars mode only (`"modes": ["stars"]`) |
| `keep.toggle` | P | Keep mode only (`"modes": ["keep"]`); remappable separately |
| `flag.reject` / `flag.unflag` / `flag.toggle` | X / U / ` | Reject wins in both rating modes |
| `label` (red / yellow / green / blue) | 6 / 7 / 8 / 9 | Purple has a command but no default key |
| `autoAdvance.toggle` | Caps Lock | Session-only override of the setting |
| `view.loupe` / `view.grid` / `view.compare` | E / G / C | 2-up on C, 3-up on ⌥C, 4-up on ⌥⇧C |
| `panel.info` | I | |
| `overlay.clipping` / `overlay.af` | J / A | |
| `hud.toggle` | H | |
| `zoom.toggle(at:)` | — (click) | Gesture only |
| `zoom.magnify(by:at:)` | — (pinch) | Gesture only |
| `zoom.lock.toggle` | — | No default binding; View menu / setting |
| `edit.undo` / `edit.redo` | ⌘Z / ⇧⌘Z | Undo in another batch navigates there first |
| `file.open` | ⌘O | |
| `cull.finish` | ⌘↩ | Also ⌘⌤ on the keypad |
| `app.fullScreen` | ⌃⌘F | |

- Default keymap: `App/Resources/DefaultKeymap.json`, one entry per binding:
  `{ "command": "...", "argument": n, "key": "...", "modifiers": [...], "modes": [...] }`. `argument` is
  the command's payload (which star count, which compare count), `modes` restricts a binding to a
  rating mode. The stable IDs are `Command.id` (`App/Sources/Input/Command.swift:68`).
- User keymap: `~/Library/Application Support/Firstcut/keymap.json` — overrides only, layered on the
  defaults, written atomically. A missing or corrupt user file falls back to the defaults rather than
  stopping the app (`App/Sources/Input/KeymapStore.swift:7-24`).
- Rating commands are ignored for photos outside the current batch; the model makes this impossible
  anyway — `rateCurrent` only ever takes `currentPhoto`, which is always inside `currentBatch`
  (`AppModel.swift:885-889`). **And** every mutating command is refused while `canAct` is false.

## The first-photo fast path

Opening a folder does not wait for the scan. `AppModel.open(folder:)` opens the
`openToFirstPhoto` span, sets `.loading`, and calls `startFastPath`
(`AppModel.swift:225, 236, 479`), which reads **one** photograph with `CoreFirstPhoto.read`
(`App/Sources/Session/CoreFirstPhoto.swift:28`) off the main thread: `firstPhotoName` from a
directory listing, then `readPhoto` for that one header. The scan, the batching and the T2 prefetch
follow in parallel behind it.

The frame it shows is **provisional**, and says so structurally: `isProvisional` is set
(`AppModel.swift:519`), `canAct` goes false so nothing can be rated into the previous folder's
session, the photo is the first by **file name** rather than by capture order, and the batch it
lives in is `provisional` with the reserved `BatchID.max` id (`AppModel.swift:503-508, 561`). If the
real open fails, or the user closes the folder while it is being read, `dropProvisionalFrame` puts the
previous shoot back (`AppModel.swift:527`). `Phase.isLoading` is what tells a superseded fast path from
a live one (`AppModel.swift:486`).

## Where the viewer gets its images

Viewer panes do **not** resolve their image source from the environment. `AppEnvironment` registers
one `PhotoViewerHost` factory that reads `state.images` and `state.activeModel` off the **active**
`CullViewState` at the moment the pane is built (`AppEnvironment.swift:99-115`; the protocol's
`activeModel` defaults to nil for the stand-ins, `App/Sources/Views/Model/CullViewState.swift:63-68`).
That is what lets `-FirstcutMockShoot` swap the whole state for a synthetic source
(`AppEnvironment.use(_:)`, `:199`) without the viewer holding a real `ImageProvider` that was never
given a folder and rendering black.

## Mock

`AppModel.preview(game:)` builds a model from `tests/fixtures/exiftool/<game>.json` (or a synthetic
shoot when `game` is nil) plus `MockSession` and `MockImageProvider`
(`App/Sources/Session/Dependencies.swift:88-101`), with persistence pointed at a scratch directory so
a preview cannot disturb the real settings or keymap. The mock provider records every `FocusRequest`
it is given and paints a stable colour per photo, so a navigation is visible in a screenshot
(`App/Sources/Session/PipelineMirror.swift:64-129`). `tests/fixtures/meta/<game>.json` is the shape
core's own tests use; Swift's mock still decodes the exiftool dumps.

## Proposed changes

(none)

## Changelog

- v0.1: initial draft.
- v0.2 (2026-10-01): matched the contract to `AppModel.swift` and `SessionTypes.swift`. Added
  `allPhotos`, `skippedFiles`, `lastError`, `finish` / `FinishStage`, `summary`, `debugHUDVisible`,
  `viewportPixelSize`, `recents`, the measured frame-latency counters, and `BatchVM.range`;
  `currentPhoto` is optional and `images` is `any ImageProviding`. Added **`canAct`** (false while the
  first-photo provisional frame is up, because the backend still belongs to the previous folder),
  **`Phase.isLoading`**, **`FinishStage.confirm`** and **`FinishSettings.requiresTypedConfirmation`**,
  and two new sections: **The first-photo fast path** (`CoreFirstPhoto`, the provisional frame, the
  reserved `BatchID.max`, and how a superseded frame is told from a live one) and **Where the viewer
  gets its images** (panes resolve from the *active* `AppEnvironment` state, not the environment's
  own provider). `Tier` is now the generated `FfiTier` and its mapping is `RatingRules.tier` with a
  configurable keep threshold rather than fixed 5–4/3/2–1. Commands: `batch.previous/next` also have
  `[` / `]`, compare has ⌥C and ⌥⇧C, finish has ⌘⌤, purple has no default key, and rating-mode
  restrictions are declared in the keymap's `"modes"`.
