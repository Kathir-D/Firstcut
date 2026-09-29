// Owner: ui.
//
// Single owner of the state the whole app renders from. `bootstrapped` builds the stand-in model
// while app-logic's `AppModel` is being written; the swap point is the one line in `init` (REQ-ui-1).

import AppKit
import Foundation
import Observation
import UniformTypeIdentifiers

@MainActor
@Observable
final class AppEnvironment {
  static let shared = AppEnvironment()

  private(set) var state: any CullViewState
  private(set) var pendingFolderURL: URL?

  private init() {
    state = PreviewCullViewState()
  }

  func use(_ newState: any CullViewState) {
    state = newState
  }

  func send(_ action: CullAction) {
    state.send(action)
  }

  func handleDrop(_ providers: [NSItemProvider]) -> Bool {
    guard let provider = providers.first else { return false }
    _ = provider.loadObject(ofClass: URL.self) { [weak self] url, _ in
      guard let url else { return }
      Task { @MainActor in self?.open(folder: url) }
    }
    return true
  }

  func open(folder url: URL) {
    pendingFolderURL = url
    state.send(.openFolder)
  }
}
