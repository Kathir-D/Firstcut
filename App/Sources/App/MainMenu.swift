// Owner: ui.
//
// Menu bar (task.md §9.1, §10): every command listed, with its default shortcut shown. Remapping is
// app-logic's keymap; these values are the Lightroom Classic defaults and move to the keymap as soon
// as it lands (REQ-ui-3). Caps Lock and zoom deliberately have no menu shortcut (zoom is pointer
// only by design, §9.2).

import AppKit
import SwiftUI

struct FirstcutCommands: Commands {
  let environment: AppEnvironment

  var body: some Commands {
    CommandGroup(replacing: .newItem) {
      Button("Open Folder…") { environment.send(.openFolder) }
        .keyboardShortcut("o", modifiers: .command)
    }

    CommandGroup(replacing: .saveItem) {
      Button("Finish Cull…") { environment.send(.finishCull) }
        .keyboardShortcut(.return, modifiers: .command)
    }

    CommandGroup(replacing: .undoRedo) {
      Button("Undo Rating") { environment.send(.undo) }
        .keyboardShortcut("z", modifiers: .command)
      Button("Redo Rating") { environment.send(.redo) }
        .keyboardShortcut("z", modifiers: [.command, .shift])
    }

    CommandGroup(replacing: .textEditing) {
      Button("Auto-Advance") { environment.send(.toggleAutoAdvance) }
    }

    CommandMenu("Photo") {
      Button("Previous Photo") { environment.send(.photoPrevious) }
        .keyboardShortcut(.leftArrow, modifiers: [])
      Button("Next Photo") { environment.send(.photoNext) }
        .keyboardShortcut(.rightArrow, modifiers: [])

      Divider()

      Button("Previous Batch") { environment.send(.batchPrevious) }
        .keyboardShortcut(.leftArrow, modifiers: .command)
      Button("Next Batch") { environment.send(.batchNext) }
        .keyboardShortcut(.rightArrow, modifiers: .command)

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

      Menu("Flag") {
        Button("Reject Flag") { environment.send(.setFlag(.reject)) }
          .keyboardShortcut("x", modifiers: [])
        Button("Unflag") { environment.send(.setFlag(.none)) }
          .keyboardShortcut("u", modifiers: [])
      }

      Button(pickOrKeepTitle) { environment.send(pickOrKeepAction) }
        .keyboardShortcut("p", modifiers: [])

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
    }

    CommandGroup(after: .toolbar) {
      Button("As Loupe") { environment.send(.setViewMode(.loupe)) }
        .keyboardShortcut("e", modifiers: [])
      Button("as Grid") { environment.send(.setViewMode(.grid)) }
        .keyboardShortcut("g", modifiers: [])
      Button("as Compare") { environment.send(.setViewMode(.compare(count: 2))) }
        .keyboardShortcut("c", modifiers: [])

      Divider()

      Button("Info Panel") { environment.send(.toggleInfoPanel) }
        .keyboardShortcut("i", modifiers: [])
      Button("Progress HUD") { environment.send(.toggleHUD) }
        .keyboardShortcut("h", modifiers: [])

      Divider()

      Button("Clipping Overlay") { environment.send(.toggleClippingOverlay) }
        .keyboardShortcut("j", modifiers: [])
      Button("AF Point Overlay") { environment.send(.toggleAFOverlay) }
        .keyboardShortcut("a", modifiers: [])

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

  private var pickOrKeepTitle: String {
    environment.state.ratingMode == .keep ? "Toggle Keep" : "Pick Flag"
  }

  private var pickOrKeepAction: CullAction {
    environment.state.ratingMode == .keep ? .toggleKeep : .setFlag(.pick)
  }
}
