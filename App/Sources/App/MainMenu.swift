// Owner: ui.
//
// Menu bar (todo.md §9.1, §10): every command listed, with its default shortcut shown. Remapping is
// app-logic's keymap; these values are the Lightroom Classic defaults and move to the keymap as soon
// as it lands (REQ-ui-3). Caps Lock and zoom deliberately have no menu shortcut — zoom is pointer
// only by design (§9.2) and Caps Lock is a `flagsChanged` event, not a key (REV-47).
//
// REV-75: commands that need a photo are disabled outside `.culling` rather than silently acting on
// nothing, and the photo-only menus are not built at all on the welcome screen.

import AppKit
import SwiftUI

struct FirstcutCommands: Commands {
    let environment: AppEnvironment

    /// Photo-only commands do nothing useful without a loaded session, so they are disabled rather
    /// than left enabled against an empty model.
    private var isCulling: Bool {
        switch environment.state.phase {
        case .culling, .finishing: true
        case .welcome, .loading: false
        }
    }

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("Open Folder…") { environment.send(.openFolder) }
                .keyboardShortcut(shortcut(.openFolder))
        }

        CommandGroup(replacing: .saveItem) {
            Button("Finish Cull…") { environment.send(.finishCull) }
                .keyboardShortcut(shortcut(.finishCull))
                .disabled(!isCulling)
        }

        CommandGroup(replacing: .undoRedo) {
            Button("Undo Rating") { environment.send(.undo) }
                .keyboardShortcut(shortcut(.undo))
                .disabled(!isCulling)
            Button("Redo Rating") { environment.send(.redo) }
                .keyboardShortcut(shortcut(.redo))
                .disabled(!isCulling)
        }

        // Find and spelling have nothing to act on here; Auto-Advance lives in the View menu.
        CommandGroup(replacing: .textEditing) {}

        CommandMenu("Photo") {
            Button("Previous Photo") { environment.send(.photoPrevious) }
                .keyboardShortcut(shortcut(.photoPrevious))
                .disabled(!isCulling)
            Button("Next Photo") { environment.send(.photoNext) }
                .keyboardShortcut(shortcut(.photoNext))
                .disabled(!isCulling)

            Divider()

            Button("Previous Batch") { environment.send(.batchPrevious) }
                .keyboardShortcut(shortcut(.batchPrevious))
                .disabled(!isCulling)
            Button("Next Batch") { environment.send(.batchNext) }
                .keyboardShortcut(shortcut(.batchNext))
                .disabled(!isCulling)

            Divider()

            Menu("Rate") {
                ForEach((1...5).reversed(), id: \.self) { stars in
                    Button("\(stars) Star\(stars == 1 ? "" : "s")") {
                        environment.send(.setRating(stars: UInt8(stars)))
                    }
                    .keyboardShortcut(shortcut(.setStars(stars)))
                }
                Button("No Stars") { environment.send(.setRating(stars: 0)) }
                    .keyboardShortcut(shortcut(.setStars(0)))
            }
            .disabled(!isCulling)

            Menu("Flag") {
                Button("Reject Flag") { environment.send(.setFlag(.reject)) }
                    .keyboardShortcut(shortcut(.rejectFlag))
                Button("Unflag") { environment.send(.setFlag(.none)) }
                    .keyboardShortcut(shortcut(.unflag))
            }
            .disabled(!isCulling)

            Button(pickOrKeepTitle) { environment.send(pickOrKeepAction) }
                .keyboardShortcut(shortcut(pickOrKeepCommand))
                .disabled(!isCulling)

            Menu("Color Label") {
                Button("Red") { environment.send(.setLabel(.red)) }
                    .keyboardShortcut(shortcut(.setLabel(.red)))
                Button("Yellow") { environment.send(.setLabel(.yellow)) }
                    .keyboardShortcut(shortcut(.setLabel(.yellow)))
                Button("Green") { environment.send(.setLabel(.green)) }
                    .keyboardShortcut(shortcut(.setLabel(.green)))
                Button("Blue") { environment.send(.setLabel(.blue)) }
                    .keyboardShortcut(shortcut(.setLabel(.blue)))
            }
            .disabled(!isCulling)
        }

        CommandGroup(after: .toolbar) {
            Button("as Loupe") { environment.send(.setViewMode(.loupe)) }
                .keyboardShortcut(shortcut(.showLoupe))
                .disabled(!isCulling)
            Button("as Grid") { environment.send(.setViewMode(.grid)) }
                .keyboardShortcut(shortcut(.showGrid))
                .disabled(!isCulling)
            Button("as Compare (2-up)") { environment.send(.setViewMode(.compare(count: 2))) }
                .keyboardShortcut(shortcut(.showCompare(2)))
                .disabled(!isCulling)
            Button("as Compare (3-up)") { environment.send(.setViewMode(.compare(count: 3))) }
                .keyboardShortcut(shortcut(.showCompare(3)))
                .disabled(!isCulling)
            Button("as Compare (4-up)") { environment.send(.setViewMode(.compare(count: 4))) }
                .keyboardShortcut(shortcut(.showCompare(4)))
                .disabled(!isCulling)

            Divider()

            Toggle("Zoom Lock", isOn: zoomLockBinding)
                .disabled(!isCulling)

            Divider()

            Toggle("Info Panel", isOn: toggle({ $0.isInfoPanelVisible }, sends: .toggleInfoPanel))
                .keyboardShortcut(shortcut(.toggleInfoPanel))
                .disabled(!isCulling)
            Toggle("Progress HUD", isOn: toggle({ $0.isHUDVisible }, sends: .toggleHUD))
                .keyboardShortcut(shortcut(.toggleHUD))
                .disabled(!isCulling)

            Divider()

            Toggle(
                "Clipping Overlay", isOn: toggle({ $0.showsClippingOverlay }, sends: .toggleClippingOverlay)
            )
            .keyboardShortcut(shortcut(.toggleClippingOverlay))
            .disabled(!isCulling)
            Toggle("AF Point Overlay", isOn: toggle({ $0.showsAFOverlay }, sends: .toggleAFOverlay))
                .keyboardShortcut(shortcut(.toggleAFOverlay))
                .disabled(!isCulling)

            Divider()

            Toggle("Auto-Advance", isOn: autoAdvanceBinding)
        }

        CommandGroup(replacing: .help) {
            Button("Firstcut Help") {
                if let url = URL(string: "https://github.com/Kathir-D/Firstcut#readme") {
                    NSWorkspace.shared.open(url)
                }
            }
        }
    }

    /// A menu checkmark that shows a state and flips it through the model, never set directly.
    private func toggle(
        _ value: @escaping @MainActor (any CullViewState) -> Bool, sends action: CullAction
    ) -> Binding<Bool> {
        Binding(
            get: { value(environment.state) },
            set: { _ in environment.send(action) }
        )
    }

    private var zoomLockBinding: Binding<Bool> {
        Binding(
            get: { environment.state.isZoomLocked },
            set: { _ in environment.send(.toggleZoomLock) }
        )
    }

    /// The menu shows the keymap's shortcut, so a remapped key is what the menu says, and it can
    /// never disagree with what the key does. Only chords with ⌘, ⌃ or ⌥ become menu key
    /// equivalents: an *unmodified* one (P, 1–5, the arrows) would fire from the menu even while a
    /// text field is being typed in. Those keys are handled by the key router alone, which knows.
    private func shortcut(_ command: Command) -> KeyboardShortcut? {
        let model = environment.model
        guard let chord = model.keymap.primaryChord(for: command, mode: model.ratingMode),
            !chord.modifiers.isDisjoint(with: [.command, .control, .option])
        else { return nil }
        return KeyboardShortcut(chord: chord)
    }

    private var pickOrKeepCommand: Command {
        environment.state.ratingMode == .keep ? .toggleKeep : .togglePickFlag
    }

    private var autoAdvanceBinding: Binding<Bool> {
        Binding(
            get: { environment.state.autoAdvanceEnabled },
            set: { _ in environment.send(.toggleAutoAdvance) }
        )
    }

    /// One `P` for both meanings, as the contract says: pick flag in stars mode, keep toggle in
    /// keep mode (app-model.md `flag.pick` / `keep.toggle`).
    private var pickOrKeepTitle: String {
        environment.state.ratingMode == .keep ? "Toggle Keep" : "Pick Flag"
    }

    private var pickOrKeepAction: CullAction {
        environment.state.ratingMode == .keep ? .toggleKeep : .setFlag(.pick)
    }
}

extension KeyboardShortcut {
    /// A keymap chord as a menu key equivalent, or nil for keys a menu cannot express.
    init?(chord: KeyChord) {
        guard let equivalent = chord.key.keyEquivalent else { return nil }
        var modifiers: EventModifiers = []
        if chord.modifiers.contains(.command) { modifiers.insert(.command) }
        if chord.modifiers.contains(.shift) { modifiers.insert(.shift) }
        if chord.modifiers.contains(.option) { modifiers.insert(.option) }
        if chord.modifiers.contains(.control) { modifiers.insert(.control) }
        self.init(equivalent, modifiers: modifiers)
    }
}

extension Key {
    var keyEquivalent: KeyEquivalent? {
        switch self {
        case .leftArrow: .leftArrow
        case .rightArrow: .rightArrow
        case .upArrow: .upArrow
        case .downArrow: .downArrow
        case .escape: .escape
        case .return: .return
        case .tab: .tab
        case .space: .space
        case .delete: .delete
        case .forwardDelete: .deleteForward
        case .home: .home
        case .end: .end
        case .pageUp: .pageUp
        case .pageDown: .pageDown
        case .zero: "0"
        case .one: "1"
        case .two: "2"
        case .three: "3"
        case .four: "4"
        case .five: "5"
        case .six: "6"
        case .seven: "7"
        case .eight: "8"
        case .nine: "9"
        case .backtick: "`"
        case .minus: "-"
        case .equal: "="
        case .leftBracket: "["
        case .rightBracket: "]"
        case .backslash: "\\"
        case .semicolon: ";"
        case .quote: "'"
        case .comma: ","
        case .period: "."
        case .slash: "/"
        case .keypadEnter, .capsLock, .function: nil
        case .a, .b, .c, .d, .e, .f, .g, .h, .i, .j, .k, .l, .m, .n, .o, .p, .q, .r, .s, .t, .u, .v,
            .w, .x, .y, .z:
            KeyEquivalent(Character(rawValue))
        }
    }
}
