// Owner: ui.
//
// Single owner of the state the whole app renders from. `bootstrapped` builds the stand-in model
// while app-logic's `AppModel` is being written; the swap point is the one line in `init` (REQ-ui-1).

import AppKit
import Foundation
import OSLog
import Observation
import UniformTypeIdentifiers

/// Opened folders are logged, failures especially. "Firstcut did nothing when I picked a folder" is
/// otherwise unanswerable after the fact: the window looks the same whether the core refused the
/// folder, the session opened and no photos parsed, or the images never decoded.
let logger = Logger(subsystem: "com.kathird.firstcut", category: "session")

@MainActor
@Observable
final class AppEnvironment {
  static let shared = AppEnvironment()

  private(set) var state: any CullViewState
  private(set) var pendingFolderURL: URL?

  /// The real pipeline, and the real model over it.
  ///
  /// This is the swap point the whole mock arrangement was built around. `PreviewCullViewState`
  /// generated 148 batches from a seed, so the window could be built and screenshotted before the
  /// core existed -- and it was still wired in here afterwards, which meant the shipped app showed
  /// photographs that were never on disk.
  private let previewPipeline: PreviewPipeline
  private let previewImages: EmbeddedPreviewSource
  private let model: AppModel
  private var live: LiveCullViewState

  private init() {
    let pipeline = PreviewPipeline()
    let images = EmbeddedPreviewSource(pipeline: pipeline)
    let model = AppModel(.live())
    previewPipeline = pipeline
    previewImages = images
    self.model = model
    // `live` first, then `state` points at it: every stored property is initialised before the
    // registration below touches anything.
    let live = LiveCullViewState(model: model, images: images)
    self.live = live
    state = live

    // A folder named on the command line is opened once the app is up. See LaunchOptions for why
    // this exists and why it is not a preference.
    if let folder = LaunchOptions.folderToOpen {
      logger.info("opening \(folder.path, privacy: .public) from the launch arguments")
      openRealFolder(folder)
    }

    // The viewer's one registration (PhotoViewerHost's swap table). Registering it here is what
    // removes the "Viewer layer pending from the pipeline agent" placeholder: the window embeds this
    // layer, the layer asks the image source for pixels, and the pixels are the camera's own
    // embedded preview.
    PhotoViewerHostView.register { _, _ in
      PreviewViewerView(images: images)
    }

    // ⌘O and the welcome screen's button both land here, and both go through NSOpenPanel: the only
    // way this app is allowed to learn about a folder.
    model.onRequestOpenFolder = { [weak self] in
      guard let self else { return }
      // NSOpenPanel, not a path typed into a text field and not a remembered default: the app
      // never learns about a folder the user did not hand it (task.md §11, and the sandbox).
      let panel = NSOpenPanel()
      panel.canChooseDirectories = true
      panel.canChooseFiles = false
      panel.allowsMultipleSelection = false
      panel.prompt = "Open"
      guard panel.runModal() == .OK, let url = panel.url else { return }
      self.openRealFolder(url)
    }
  }

  /// Opens a folder for real: the Rust session, the real batches, the real Finish.
  func openRealFolder(_ url: URL) {
    do {
      let backend = try CoreSessionBackend(folder: url)
      model.open(backend, folderName: url.lastPathComponent)
      let data = backend.data
      live.attach(folder: url, photos: data.photos, order: data.batches.flatMap(\.photoIds))
      logger.info(
        "opened \(url.path, privacy: .public): \(data.photos.count) photos in \(data.batches.count) batches"
      )
    } catch {
      logger.error("could not open \(url.path, privacy: .public): \(error.localizedDescription)")
      // A refusal is a sentence, not a crash: "you picked a file" and "there are no photographs
      // in it" are the two a user can act on.
      presentRefusal(error)
    }
    pendingFolderURL = url
  }

  /// The window's pixel size, pushed to the model so the decoder is asked for what will be drawn.
  func setViewportSize(_ size: CGSize) {
    live.setViewport(size)
  }

  private func presentRefusal(_ error: any Error) {
    let alert = NSAlert()
    alert.messageText = "Firstcut can't open that"
    alert.informativeText = error.localizedDescription
    alert.addButton(withTitle: "OK")
    alert.alertStyle = .warning
    alert.runModal()
  }

  /// Test and preview seam. Production always uses the real state; a test may substitute its own.
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
