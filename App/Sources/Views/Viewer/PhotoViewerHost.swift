// Owner: ui.
//
// The seam between the window and pipeline's viewer layer (REV-52). ui declares the protocol and
// embeds whatever conforms; pipeline provides the implementation. Neither folder imports the
// other, so REV-38's mechanism change (IOSurface → CAMetalLayer, or a custom `CALayer` taking an
// IOSurface) does not touch a file in `Views/`.
//
// The host is a plain `NSView` so it embeds in the window's layer tree with no per-frame
// allocation: the layer is created once per photo and only `setPhoto` is called on navigation.
//
// ## Swap table (REV-56, REV-74)
//
// | what | stands in for | swap point | who deletes it |
// | --- | --- | --- | --- |
// | `PhotoViewerHost` (this file) | pipeline's `PhotoViewerLayerView` in pipeline-api.md | `App/Sources/Views/Viewer/ViewerArea.swift` → `PhotoViewerHostLayer` | nobody: pipeline conforms, ui keeps the protocol |
// | `PhotoViewerHostView.register(_:)` | the conformance itself | the one `register` call in `AppEnvironment` (REQ-ui-2) | ui, once pipeline's type can be named directly |
//
// If pipeline's view needs more than this protocol, add it **here** — that is the point of the
// protocol. Do not reach around it into pipeline's concrete type from a view.

import AppKit
import QuartzCore

@MainActor
protocol PhotoViewerHost: AnyObject {
  /// The photo to show. Called on every navigation; must not reallocate the backing surface when
  /// the id is unchanged.
  func setPhoto(_ id: PhotoID, aspectRatio: Double)

  /// Zoom state pushed down from the model (app-model.md's `ViewerState`). The host owns pinch and
  /// click-to-100% itself; this is only so the window chrome can react.
  func setViewerState(_ state: ViewerPresentation)

  /// The viewer's own frame changed, so the implementation can re-decode for the new backing size
  /// (task.md §7.1) while keeping the old image visible.
  func setViewportSize(_ size: CGSize)

  /// The zoom level currently presented, for the HUD. Read on demand, not pushed.
  var presentedZoom: Double { get }
}

/// What the window needs to know about the viewer's zoom, mirrored from app-model.md's
/// `ViewerState` (REV-46). app-logic owns the value; ui mirrors the fields it draws.
struct ViewerPresentation: Equatable {
  var isZoomed = false
  var zoomScale: Double = 1
  var isZoomLocked = false

  static let fit = ViewerPresentation()
}

/// Hosts pipeline's viewer layer without a compile-time dependency on it. `register` is the one
/// call the app makes at launch; until it is called the host stays empty and `ViewerArea` draws the
/// placeholder.
@MainActor
final class PhotoViewerHostView: NSView {
  typealias HostFactory = @MainActor (PhotoID, Double) -> any PhotoViewerHost

  nonisolated(unsafe) private static var storedFactory: HostFactory?

  /// Register pipeline's viewer layer. Idempotent; the last registration wins. One factory, one
  /// host **per host view** — the factory is called once per `PhotoViewerHostView`, so two windows
  /// never share a drawing surface.
  static func register(_ factory: @escaping HostFactory) {
    storedFactory = factory
  }

  static var isRegistered: Bool { storedFactory != nil }

  var photoID: PhotoID? {
    didSet {
      guard photoID != oldValue else { return }
      guard let id = photoID, let host else { return }
      host.setPhoto(id, aspectRatio: aspectRatio)
    }
  }

  var aspectRatio: Double = 1.5 {
    didSet {
      guard aspectRatio != oldValue else { return }
      guard let id = photoID, let host else { return }
      host.setPhoto(id, aspectRatio: aspectRatio)
    }
  }

  var viewerState = ViewerPresentation.fit {
    didSet {
      guard viewerState != oldValue else { return }
      host?.setViewerState(viewerState)
    }
  }

  private var host: (any PhotoViewerHost)?

  override init(frame frameRect: NSRect) {
    super.init(frame: frameRect)
    wantsLayer = true
    guard let factory = Self.storedFactory else { return }
    host = factory(0, aspectRatio)
    adopt(host)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is not used")
  }

  override func layout() {
    super.layout()
    guard bounds.size != .zero else { return }
    host?.setViewportSize(bounds.size)
  }

  private func adopt(_ host: (any PhotoViewerHost)?) {
    guard let host, let view = host as? NSView else { return }
    view.removeFromSuperview()
    view.frame = bounds
    view.autoresizingMask = [.width, .height]
    addSubview(view)
    setAccessibilityLabel("Photo viewer")
  }
}
