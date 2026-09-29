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

        CommandGroup(replacing: .textEditing) {
            Button("Auto-Advance") { environment.send(.toggleAutoAdvance) }
        }

        CommandMenu("Photo") {
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

            Menu("Rate") {
                ForEach((1...5).reversed(), id: \.self) { stars in
                    Button("\(stars) Star\(stars == 1 ? "" : "s")") {
                        environment.send(.setRating(stars: UInt8(stars)))
                    }
                    .keyboardShortcut(KeyEquivalent(Character("\(stars)")), modifiers: [])
                }
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

        CommandGroup(after: .toolbar) {
            Button("As Loupe") { environment.send(.setViewMode(.loupe)) }
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
