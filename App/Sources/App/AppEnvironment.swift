// Owner: app-logic + ui.
//
// The composition root, and the one documented swap point between the model and the views (REV-74,
// REV-56). It builds the single `AppModel`, wraps it in the read-only `CullViewState` projection the
// views bind to, and hands out the real `ImageProvider`.
//
// The swap point, in one line: `ModelCullViewState(model:images:)`. Before this the app ran
// `PreviewCullViewState` — 148 synthetic batches from a seeded PRNG — because nothing bridged
// `AppModel` to the views' protocol. Now the shipped path is:
//
//     AppModel (app-logic)  ->  ModelCullViewState  ->  the views
//            |                       |
//            |                       +-- reads tier/isKeep off the model, never recomputes
//            +-- SessionBackend over the real Rust `Session`
//            +-- ImageProvider decoding the CR3's embedded preview
//
// Nothing in `AppEnvironment` holds state. A folder open goes to the model and the model's
// observation drives every view, so there is exactly one place a value can come from.

import AppKit
import Foundation
import Observation
import UniformTypeIdentifiers

@MainActor
@Observable
final class AppEnvironment {
  static let shared = AppEnvironment()

  /// The model. Everything the app knows, and the only thing allowed to mutate it.
  let model: AppModel

  /// The views' read-only projection of `model`. Same object, narrower surface: views cannot
  /// mutate through this, which is what makes "the model is the only mutation path" (REV-50)
  /// checkable rather than aspirational.
  private(set) var state: any CullViewState

  /// The real decoder. Shared with the model so the viewer and the filmstrip hit one cache.
  let images: ImageProvider

  /// A folder chosen but not opened yet — see `open(folder:)`, which is why this is not just a
  /// property the views read.
  private(set) var pendingFolderURL: URL?

  private init() {
    // `shared` is a `static let`, so this runs at `NSApplicationMain` — which in an XCTest process
    // is the **test host**, before the runner has connected. Building a real `AppModel` there means
    // a real Rust `Session`, which opens a database under Application Support, and that is enough
    // to stop the host ever bootstrapping: the runner reports "test crashed with signal term before
    // establishing connection", with no crash log and no failing test. It cost a long time to
    // diagnose because "the app does nothing when I run it" and "the tests die" look nothing alike.
    //
    // So: in a test host, build an empty model and let the test set up its own session. In the real
    // app, build the real thing. `isRunningUnderXCTest` is the whole mechanism, and it is checked
    // once, here, where the side effect happens.
    if Self.isRunningUnderXCTest {
      // An empty `MockSession` rather than a real one: the test host must touch no database and no
      // user folder, or the runner never connects. A suite that needs a session installs its own
      // through `Dependencies.testing(backend:)`.
      let model = AppModel(
        .testing(backend: MockSession(data: SessionData(folder: "", photos: [], batches: []))))
      self.model = model
      self.images = ImageProvider(memoryBudgetBytes: 256 << 20)
      self.state = ModelCullViewState(model: model, images: self.images)
      PhotoViewerHostView.register { [images] _, _ in CGImageViewerHost(images: images) }
      return
    }

    let dependencies = Dependencies.live()
    let model = AppModel(dependencies)
    self.model = model
    // The *same* provider the model holds, not a second one. Two providers means two caches, two
    // decode queues, and two `focusMisses` counters, and only the one `AppModel.open` primes ever
    // gets a folder — so the viewer's copy returns nil for every photo while the model's works. One
    // instance, read from the model, is the only arrangement where the viewer and the filmstrip see
    // the same pixels.
    // `live()` always builds a real `ImageProvider`, so this is non-nil in the app. The fallback
    // exists only so a future `live()` that does not would fail visibly rather than silently.
    guard let provider = model.imageProvider else {
      preconditionFailure("Dependencies.live() must supply a real ImageProvider")
    }
    self.images = provider
    self.state = ModelCullViewState(model: model, images: self.images)
    // The one registration call in the app (REQ-ui-2, REV-52). Until this ran, the viewer stayed
    // empty and drew "Viewer layer pending from the pipeline agent" over every photo. It is
    // unconditional: a test that wants a different host registers its own, and `register` is
    // last-wins.
    PhotoViewerHostView.register { [images] _, _ in CGImageViewerHost(images: images) }

    // The model asks for a panel and a window; it cannot make either. Without these two lines
    // "Open Folder…" (⌘O, the toolbar, the Welcome button) and Full Screen (⌃⌘F) were consumed by
    // the key router and then did nothing, because nothing was listening.
    model.onRequestOpenFolder = { [weak self] in self?.presentOpenPanel() }
    model.onRequestToggleFullScreen = { _ in NSApp.keyWindow?.toggleFullScreen(nil) }

    // Launch flags, applied to *this* instance rather than through `AppEnvironment.shared`.
    //
    // The obvious spelling — a static helper that reaches back for `AppEnvironment.shared` — traps
    // with EXC_BREAKPOINT in `_dispatch_once_wait`: `shared` is a `static let`, so asking for it
    // from inside its own initialisation re-enters the `dispatch_once` that is still running. The
    // app died on launch, every time, with a crash report that took a while to read because the
    // frame that matters is `unsafeMutableAddressor`. `self` is already the singleton here, so the
    // indirection was never needed.
    if LaunchOptions.usesMockShoot {
      let mock = AppModel.preview(game: nil, photoLimit: nil)
      mock.open(mock.backend, folderName: "Preview")
      use(
        ModelCullViewState(
          model: mock, images: PreviewImageSource(seed: 0x5eed_f1c5) as any CullImageSource))
    }
    if let folder = LaunchOptions.folder {
      open(folder: folder)
    }
  }

  /// True when this process is an XCTest host rather than the app the user launched.
  ///
  /// `XCTestConfigurationFilePath` is set by the runner in the test process only; the shipped app
  /// never has it. `XCTestBundlePath` is set too, but it is also present in some launch contexts, so
  /// the configuration file is the reliable one.
  private static var isRunningUnderXCTest: Bool {
    ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil
        || ProcessInfo.processInfo.environment["XCTestBundlePath"] != nil
  }

  /// The documented swap point (REV-74). ui reached this through a one-line `use(_:)`; that is kept
  /// so the swap stays a single greppable line, and a test can install a stand-in.
  func use(_ newState: any CullViewState) { state = newState }

  /// Every command from every surface — the menus, the toolbar, the key router, the filmstrip —
  /// funnels through here, so there is one place the rules are enforced.
  func send(_ action: CullAction) { state.send(action) }

  /// Drag a folder onto the window. A dropped folder is a security-scoped resource, so the URL has
  /// to be claimed before the sandbox will let us read it.
  func handleDrop(_ providers: [NSItemProvider]) -> Bool {
    guard let provider = providers.first else { return false }
    _ = provider.loadObject(ofClass: URL.self) { url, _ in
      guard let url else { return }
      Task { @MainActor in self.open(folder: url) }
    }
    return true
  }

  /// The folder chooser. The URL it returns carries the user's grant for that folder, which is the
  /// only way the app is meant to be given one (see `open(folder:)`).
  func presentOpenPanel() {
    let panel = NSOpenPanel()
    panel.title = "Open a Folder of Photos"
    panel.message = "Choose the folder that holds the whole shoot."
    panel.prompt = "Open"
    panel.canChooseDirectories = true
    panel.canChooseFiles = false
    panel.canCreateDirectories = false
    panel.allowsMultipleSelection = false
    guard panel.runModal() == .OK, let url = panel.url else { return }
    open(folder: url)
  }

  /// Open a real folder. The URL arrives from `NSOpenPanel` (which already carries the grant) or
  /// from a launch flag, and either way this is the *only* place the app starts reading a
  /// user-chosen directory — never a raw path from anywhere else (the TCC rule, and the reason the
  /// test host used to hang: a GUI app reading under ~/Documents raises a consent prompt that
  /// nobody is there to answer).
  func open(folder url: URL) {
    let scoped = url.startAccessingSecurityScopedResource()
    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
    pendingFolderURL = url
    model.open(folder: url)
    pendingFolderURL = nil
  }
}
