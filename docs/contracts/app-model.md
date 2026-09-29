# Contract: AppModel, commands & keymap

- **Owner:** app-logic
- **Consumers:** ui (binds views to it), qa (drives it in integration tests)
- **Version:** v0.1 (draft; frozen as v1.0 at the end of wave 1)

## State the UI binds to

```swift
@MainActor @Observable final class AppModel {
    // Session
    var phase: Phase                        // .welcome, .loading(progress), .culling, .finishing
    var folderName: String
    var batches: [BatchVM]                  // index, photo IDs, visited, provisional
    var currentBatchIndex: Int
    var currentPhotoIndex: Int              // within the current batch
    var currentPhoto: PhotoVM               // meta for the info panel, rating, tier
    func photos(inBatch: Int) -> [PhotoVM]

    // Modes and toggles
    var ratingMode: RatingMode              // .stars, .keep
    var viewMode: ViewMode                  // .loupe, .grid, .compare(count)
    var infoPanelVisible: Bool
    var hudVisible: Bool
    var afOverlay: Bool
    var clippingOverlay: Bool
    var autoAdvance: Bool
    var viewer: ViewerState                 // zoomed, zoom anchor (normalized), zoom locked

    // Progress (HUD / Finish summary)
    var progress: CullProgress              // batch X of Y, photos left, counts per tier

    let images: ImageProvider               // from pipeline
    func perform(_ command: Command)        // every user action goes through here
}

struct PhotoVM { id; fileName; meta: PhotoMeta; rating: Rating; tier: Tier; isKeep: Bool }
enum Tier { keep, good, maybe, unrated, rejected }   // stars: 5–4 keep, 3 good, 2–1 maybe; keep mode: keep / unrated
```

## Commands

Every action is a `Command`. Keys, menus, toolbar buttons, and gestures all call
`AppModel.perform(_:)`. **Zoom has no key by default.**

| Command ID | Default key | Notes |
| --- | --- | --- |
| `photo.previous` / `photo.next` | ← / → | Within the batch; behavior at the ends comes from settings |
| `batch.previous` / `batch.next` | ⌘← / ⌘→ | Also the toolbar ‹ › buttons |
| `rate.stars(0...5)` | 0–5 | Stars mode |
| `rate.starsAndAdvance(1...5)` | ⇧1–⇧5 | |
| `flag.pick` | P | In keep mode: `keep.toggle` |
| `keep.toggle` | P (keep mode) | Remappable separately |
| `flag.reject` / `flag.unflag` / `flag.toggle` | X / U / ` | |
| `label.red/yellow/green/blue` | 6 / 7 / 8 / 9 | |
| `autoAdvance.toggle` | Caps Lock | |
| `view.loupe` / `view.grid` / `view.compare` | E / G / C | |
| `panel.info` | I | |
| `overlay.clipping` / `overlay.af` | J / A | |
| `hud.toggle` | H | |
| `zoom.toggle(at:)` | — (click) | Gesture only |
| `zoom.magnify(by:at:)` | — (pinch) | Gesture only |
| `zoom.lock.toggle` | — | View menu / setting |
| `edit.undo` / `edit.redo` | ⌘Z / ⇧⌘Z | Undo in another batch navigates there first |
| `file.open` | ⌘O | |
| `cull.finish` | ⌘↩ | |
| `app.fullScreen` | ⌃⌘F | |

- Default keymap: `App/Resources/DefaultKeymap.json` (`{ "command": "...", "key": "...", "modifiers": [...] }`).
- User keymap: `~/Library/Application Support/Firstcut/keymap.json`.
- Rating commands are ignored for photos outside the current batch; the model makes this impossible anyway.

## Mock

`AppModel.preview(game:)` builds a model from `tests/fixtures/meta/<game>.json` plus a mock
`ImageProvider` (solid colors or thumbnails), so ui can build every screen before the core and
pipeline exist.

## Proposed changes

(none)

## Changelog

- v0.1: initial draft.
