// Owner: app-logic.
//
// Every user action in Firstcut is a `Command`. Menus, toolbar buttons, key chords and gestures all
// funnel into `AppModel.perform(_:)`; no view mutates the model directly. That single funnel is what
// makes the whole app testable without a window (docs/contracts/app-model.md).
//
// `id` is the stable string used in `App/Resources/DefaultKeymap.json`, in the user's keymap.json
// and as a menu-item tag. `argument` carries the numeric payload of parameterized commands.

import CoreGraphics
import Foundation

/// A point on the photo in normalized 0...1 image space, origin top-left, EXIF orientation applied.
public struct NormalizedPoint: Hashable, Sendable, Codable {
    public var x: Double
    public var y: Double

    public init(x: Double, y: Double) {
        self.x = min(max(x, 0), 1)
        self.y = min(max(y, 0), 1)
    }
}

public enum Command: Hashable, Sendable {
    // Navigation (todo.md §9.4)
    case photoPrevious
    case photoNext
    case batchPrevious
    case batchNext

    // Rating (todo.md §6)
    case setStars(Int)  // 0...5; 0 clears
    case setStarsAndAdvance(Int)  // 1...5, ⇧1…⇧5 in Lightroom
    case togglePickFlag
    /// Switch rating mode from anywhere — the Settings picker, or `-FirstcutRatingMode` at launch.
    ///
    /// It is a command rather than a direct `model.updateSettings` call so that the flag path goes
    /// through the *same* funnel every keystroke and menu item uses (`state.send` → `perform`). That
    /// matters: reaching for `activeModel` from the environment looked equivalent and was not,
    /// because with `-FirstcutMockShoot` the cull state is replaced wholesale and the model the
    /// environment holds is not the one on screen. It has no default chord — nothing is bound to it —
    /// so it cannot shadow a photographer's keys.
    case setRatingMode(RatingMode)

    case toggleKeep  // keep mode only
    /// The two halves of `toggleKeep`, as separate commands, so keep mode can offer a **Keep**
    /// button and a **Not keep** button rather than one key that flips (owner request, 2026-10-02).
    ///
    /// The toggle stays, and stays bound to P: it is the fastest thing a photographer does and one
    /// chord is still right for it. What changes is that the *on-screen* controls can now say what
    /// they do instead of "toggle" — which a button label cannot honestly say, and which is the
    /// whole reason a toggle is the wrong shape for a button. A custom keymap can bind these too.
    case setKeep  // keep mode only; idempotent, unlike the toggle
    case setNotKeep  // keep mode only; idempotent
    case rejectFlag
    case unflag
    case toggleFlag
    case setLabel(ColorLabel?)

    // Toggles
    case toggleAutoAdvance  // Caps Lock
    case toggleInfoPanel
    case toggleClippingOverlay
    case toggleAFOverlay
    case toggleHUD
    case toggleZoomLock

    // Views
    case showLoupe
    case showGrid
    case showCompare(Int)  // 2, 3 or 4

    // Zoom: gestures only, no default key (todo.md §2, §9.2, §10)
    case toggleZoom(at: NormalizedPoint?)
    case magnify(by: Double, at: NormalizedPoint?)

    // Session
    case undo
    case redo
    case openFolder
    case finishCull
    case toggleFullScreen
}

extension Command {
    /// Stable identifier; also the key used by the keymap JSON.
    public var id: String {
        switch self {
        case .photoPrevious: "photo.previous"
        case .photoNext: "photo.next"
        case .batchPrevious: "batch.previous"
        case .batchNext: "batch.next"
        case .setStars: "rate.stars"
        case .setStarsAndAdvance: "rate.starsAndAdvance"
        case .togglePickFlag: "flag.pick"
        case .setRatingMode: "mode.rating"
        case .toggleKeep: "keep.toggle"
        case .setKeep: "keep.set"
        case .setNotKeep: "keep.clear"
        case .rejectFlag: "flag.reject"
        case .unflag: "flag.unflag"
        case .toggleFlag: "flag.toggle"
        case .setLabel: "label"
        case .toggleAutoAdvance: "autoAdvance.toggle"
        case .toggleInfoPanel: "panel.info"
        case .toggleClippingOverlay: "overlay.clipping"
        case .toggleAFOverlay: "overlay.af"
        case .toggleHUD: "hud.toggle"
        case .toggleZoomLock: "zoom.lock.toggle"
        case .showLoupe: "view.loupe"
        case .showGrid: "view.grid"
        case .showCompare: "view.compare"
        case .toggleZoom: "zoom.toggle"
        case .magnify: "zoom.magnify"
        case .undo: "edit.undo"
        case .redo: "edit.redo"
        case .openFolder: "file.open"
        case .finishCull: "cull.finish"
        case .toggleFullScreen: "app.fullScreen"
        }
    }

    /// The numeric payload a keymap entry carries, or nil for plain commands.
    public var argument: Int? {
        switch self {
        case .setStars(let stars), .setStarsAndAdvance(let stars): stars
        case .setLabel(let label): label.flatMap(\.ordinal)
        // 0 = stars, 1 = keep: a keymap entry can only carry an integer, and the mode is an enum.
        case .setRatingMode(let mode): mode == .stars ? 0 : 1
        case .showCompare(let count): count
        default: nil
        }
    }

    /// Inverse of `id` + `argument`. Returns nil for ids this build doesn't know, so an old or
    /// hand-edited keymap can't crash the app — the entry is skipped with a warning instead.
    public init?(id: String, argument: Int? = nil) {
        switch id {
        case "photo.previous": self = .photoPrevious
        case "photo.next": self = .photoNext
        case "batch.previous": self = .batchPrevious
        case "batch.next": self = .batchNext
        case "rate.stars": self = .setStars(min(max(argument ?? 0, 0), 5))
        case "rate.starsAndAdvance": self = .setStarsAndAdvance(min(max(argument ?? 1, 1), 5))
        case "flag.pick": self = .togglePickFlag
        case "mode.rating":
            // Two spellings so a hand-written keymap entry can be either.
            switch argument {
            case 0: self = .setRatingMode(.stars)
            default: self = .setRatingMode(.keep)
            }
        case "keep.toggle": self = .toggleKeep
        case "keep.set": self = .setKeep
        case "keep.clear": self = .setNotKeep
        case "flag.reject": self = .rejectFlag
        case "flag.unflag": self = .unflag
        case "flag.toggle": self = .toggleFlag
        case "label": self = .setLabel(argument.flatMap(ColorLabel.init(ordinal:)))
        case "autoAdvance.toggle": self = .toggleAutoAdvance
        case "panel.info": self = .toggleInfoPanel
        case "overlay.clipping": self = .toggleClippingOverlay
        case "overlay.af": self = .toggleAFOverlay
        case "hud.toggle": self = .toggleHUD
        case "zoom.lock.toggle": self = .toggleZoomLock
        case "view.loupe": self = .showLoupe
        case "view.grid": self = .showGrid
        case "view.compare": self = .showCompare(min(max(argument ?? 2, 2), 4))
        case "zoom.toggle": self = .toggleZoom(at: nil)
        case "zoom.magnify": self = .magnify(by: Double(argument ?? 0), at: nil)
        case "edit.undo": self = .undo
        case "edit.redo": self = .redo
        case "file.open": self = .openFolder
        case "cull.finish": self = .finishCull
        case "app.fullScreen": self = .toggleFullScreen
        default: return nil
        }
    }

    /// Whether holding the key down should keep firing this command.
    ///
    /// Repeat is welcome while navigating and rating — that's the whole point of holding → — but a
    /// modal or file-affecting action must fire exactly once per physical press.
    public var isRepeatable: Bool {
        switch self {
        case .photoPrevious, .photoNext, .batchPrevious, .batchNext,
            .setStars, .setStarsAndAdvance, .togglePickFlag, .toggleKeep, .setKeep, .setNotKeep,
            .rejectFlag, .unflag, .toggleFlag, .setLabel:
            true
        case .openFolder, .finishCull, .undo, .redo, .toggleFullScreen,
            .toggleAutoAdvance, .showLoupe, .showGrid, .showCompare:
            false
        case .toggleInfoPanel, .toggleClippingOverlay, .toggleAFOverlay, .toggleHUD,
            .toggleZoomLock, .toggleZoom, .magnify, .setRatingMode:
            // Switching mode is not something to repeat while a key is held: it recomputes every
            // photo's tier, so holding the chord would do the whole shoot's work several times.
            false
        }
    }

    /// True for commands that change a photo's rating. Used for auto-advance and XMP flush timing.
    public var isRatingChange: Bool {
        switch self {
        case .setStars, .setStarsAndAdvance, .togglePickFlag, .toggleKeep,
            .rejectFlag, .unflag, .toggleFlag, .setLabel:
            true
        default:
            false
        }
    }
}

extension ColorLabel {
    /// 6…9 in todo.md §10, or nil for "no label".
    public var ordinal: Int? {
        switch self {
        case .red: 6
        case .yellow: 7
        case .green: 8
        case .blue: 9
        case .purple: nil  // palette-only (§6.3 lists 6–9; purple isn't a shortcut)
        }
    }

    public init?(ordinal: Int) {
        switch ordinal {
        case 6: self = .red
        case 7: self = .yellow
        case 8: self = .green
        case 9: self = .blue
        default: return nil
        }
    }
}
