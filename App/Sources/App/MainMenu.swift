// Owner: ui.
//
// Menu bar (task.md §9.1, §10): every command listed, with its default shortcut shown. Remapping is
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
                .keyboardShortcut("o", modifiers: .command)
        }

        CommandGroup(replacing: .saveItem) {
            Button("Finish Cull…") { environment.send(.finishCull) }
                .keyboardShortcut(.return, modifiers: .command)
                .disabled(!isCulling)
        }

        CommandGroup(replacing: .undoRedo) {
            Button("Undo Rating") { environment.send(.undo) }
                .keyboardShortcut("z", modifiers: .command)
                .disabled(!isCulling)
            Button("Redo Rating") { environment.send(.redo) }
                .keyboardShortcut("z", modifiers: [.command, .shift])
                .disabled(!isCulling)
        }

        SwiftUI.CommandMenu("Photo") {
            Button("Auto-Advance") { environment.send(.toggleAutoAdvance) }

            Divider()

            Button("Previous Photo") { environment.send(.photoPrevious) }
                .keyboardShortcut(.leftArrow, modifiers: [])
                .disabled(!isCulling)
            Button("Next Photo") { environment.send(.photoNext) }
                .keyboardShortcut(.rightArrow, modifiers: [])
                .disabled(!isCulling)

            Divider()

            Button("Previous Batch") { environment.send(.batchPrevious) }
                .keyboardShortcut(.leftArrow, modifiers: .command)
                .disabled(!isCulling)
            Button("Next Batch") { environment.send(.batchNext) }
                .keyboardShortcut(.rightArrow, modifiers: .command)
                .disabled(!isCulling)

            Divider()

            // Listed explicitly rather than with a `ForEach`. `Commands` is a result builder, not
            // a view builder: a `ForEach` there is not a menu item, and the failure mode is a
            // baffling "extra trailing closure passed in call" reported at the *next* `CommandMenu`,
            // which is how this file lost time. Five buttons, written out, read better anyway.
            Menu("Rate") {
                Button("5 Stars") { environment.send(.setRating(stars: 5)) }
                    .keyboardShortcut("5", modifiers: [])
                Button("4 Stars") { environment.send(.setRating(stars: 4)) }
                    .keyboardShortcut("4", modifiers: [])
                Button("3 Stars") { environment.send(.setRating(stars: 3)) }
                    .keyboardShortcut("3", modifiers: [])
                Button("2 Stars") { environment.send(.setRating(stars: 2)) }
                    .keyboardShortcut("2", modifiers: [])
                Button("1 Star") { environment.send(.setRating(stars: 1)) }
                    .keyboardShortcut("1", modifiers: [])
                Divider()
                Button("No Stars") { environment.send(.setRating(stars: 0)) }
                    .keyboardShortcut("0", modifiers: [])
            }
            .disabled(!isCulling)

            Menu("Flag") {
                Button("Reject Flag") { environment.send(.setFlag(.reject)) }
                    .keyboardShortcut("x", modifiers: [])
                Button("Unflag") { environment.send(.setFlag(.none)) }
                    .keyboardShortcut("u", modifiers: [])
            }
            .disabled(!isCulling)

            Button(pickOrKeepTitle) { environment.send(pickOrKeepAction) }
                .keyboardShortcut("p", modifiers: [])
                .disabled(!isCulling)

            Menu("Color Label") {
                Button("Red") { environment.send(.setLabel(.red)) }
                    .keyboardShortcut("6", modifiers: [])
                Button("Yellow") { environment.send(.setLabel(.yellow)) }
                    .keyboardShortcut("7", modifiers: [])
                Button("Green") { environment.send(.setLabel(.green)) }
                    .keyboardShortcut("8", modifiers: [])
                Button("Blue") { environment.send(.setLabel(.blue)) }
                    .keyboardShortcut("9", modifiers: [])
            }
            .disabled(!isCulling)
        }

        // `SwiftUI.` is required, not decoration: app-logic declares its own `CommandMenu` enum
        // in `App/Sources/Input/CommandCatalog.swift` (which menu a command lives in), and because
        // that one is `public` in the same module it shadows `SwiftUI.CommandMenu` for every file.
        // The bare `CommandMenu("Photo")` then resolved to the enum -- which takes no arguments --
        // and the compiler reported "extra trailing closure passed in call" pointing at the *next*
        // menu, with no note naming the shadowing type. Qualifying it is the smallest fix that
        // keeps both names, and `CommandMenu` is the right name for the catalog side.
        //
        // View commands also live in their own submenu rather than `CommandGroup(after: .toolbar)`,
        // which does not compile: `.toolbar` is a placement you can replace, not one you can put
        // things after.
        SwiftUI.CommandMenu("View") {
            Button("as Loupe") { environment.send(.setViewMode(.loupe)) }
                .keyboardShortcut("e", modifiers: [])
                .disabled(!isCulling)
            Button("as Grid") { environment.send(.setViewMode(.grid)) }
                .keyboardShortcut("g", modifiers: [])
                .disabled(!isCulling)
            Button("as Compare") { environment.send(.setViewMode(.compare(count: 2))) }
                .keyboardShortcut("c", modifiers: [])
                .disabled(!isCulling)

            Divider()

            Button("Info Panel") { environment.send(.toggleInfoPanel) }
                .keyboardShortcut("i", modifiers: [])
                .disabled(!isCulling)
            Button("Progress HUD") { environment.send(.toggleHUD) }
                .keyboardShortcut("h", modifiers: [])
                .disabled(!isCulling)

            Divider()

            Button("Clipping Overlay") { environment.send(.toggleClippingOverlay) }
                .keyboardShortcut("j", modifiers: [])
                .disabled(!isCulling)
            Button("AF Point Overlay") { environment.send(.toggleAFOverlay) }
                .keyboardShortcut("a", modifiers: [])
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
