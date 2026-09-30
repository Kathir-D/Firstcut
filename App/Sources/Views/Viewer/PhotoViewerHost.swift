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
  /// (todo.md §7.1) while keeping the old image visible.
  func setViewportSize(_ size: CGSize)

  /// The zoom level currently presented, for the HUD. Read on demand, not pushed.
  var presentedZoom: Double { get }

  /// Takes a zoom and spot from a peer in the same sync group. Must not broadcast again.
  func applySynced(_ state: ViewerSyncGroup.State)

  /// Hosts that share a group zoom and pan together (Compare view, todo.md §9.6).
  var syncGroup: ViewerSyncGroup? { get set }

  /// Called once the newly presented frame has been **committed**, not merely drawn into a layer.
  ///
  /// This is the closing end of todo.md §7.3's "arrow key → sharp photo" interval, and the distinction
  /// is the whole measurement: the handler returning says nothing about what the user sees, and the
  /// decode landing says nothing either if the commit has not happened. A `CATransaction` completion
  /// is the closest synchronous signal AppKit offers, and it is what the signpost closes on.
  ///
  /// The argument matters as much as the call. §7.3 says *sharp* photo, and a stand-in is not one, so
  /// a span that closed on `PresentedFrame.standIn` would report a fast frame for a soft picture.
  var onFramePresented: ((PresentedFrame) -> Void)? { get set }
}

/// What the viewer just put on screen. The distinction is todo.md §7.1's whole promise: navigation
/// never shows the stand-in, and the intervals that measure it are not allowed to pretend otherwise.
public enum PresentedFrame: Equatable, Sendable {
  /// The 256 px thumbnail standing in while a display decode is in flight. §7.1 says the user can
  /// never reach one by navigating — a `standIn` between a keystroke and the photograph is a bug in
  /// the prefetch, and the counter that shows it is `PipelineStats.focusMisses`.
  case standIn

  /// The display decode: T2 at the viewer's size, or the full-resolution bitmap when zoomed. This is
  /// the "sharp photo" §7.3 measures to.
  case display
}

/// Keeps several viewer hosts at the same zoom and the same spot, so two or more frames of a burst
/// can be compared at 100% on the same detail. Members are held weakly: the group never keeps a
/// closed pane alive.
@MainActor
final class ViewerSyncGroup {
  struct State: Equatable {
    var zoom: CGFloat
    var center: CGPoint
  }

  private struct Member { weak var host: (any PhotoViewerHost)? }
  private var members: [Member] = []

  func add(_ host: any PhotoViewerHost) {
    members.removeAll { $0.host == nil }
    members.append(Member(host: host))
  }

  /// Called by the host the user is touching. Peers take the state without re-broadcasting.
  func broadcast(_ state: State, from origin: any PhotoViewerHost) {
    for member in members {
      guard let host = member.host, host !== origin else { continue }
      host.applySynced(state)
    }
  }
}

/// What the window tells the viewer: whether zoom is locked across photos, and which overlays to
/// draw. Mirrored from app-model.md's `ViewerState` (REV-46); app-logic owns the value.
struct ViewerPresentation: Equatable {
  var isZoomed = false
  var zoomScale: Double = 1
  var isZoomLocked = false
  /// Autofocus points to draw, in the *upright* image's normalized coordinates (top-left origin).
  var afRects: [AFRect] = []
  var showsClipping = false

  static let fit = ViewerPresentation()

  struct AFRect: Equatable {
    /// Centre and size, normalized 0…1.
    var x: CGFloat
    var y: CGFloat
    var w: CGFloat
    var h: CGFloat
    var inFocus: Bool
  }

  /// The AF points of a photo mapped from the sensor's frame into the upright frame the viewer
  /// displays, by EXIF orientation. `decodeFull` rotates the pixels the same way, so the box lands
  /// on the thing that was in focus rather than on the same spot of a sideways sensor.
  static func afRects(from af: AfInfo?, orientation: UInt8) -> [AFRect] {
    guard let af else { return [] }
    return af.points.map { point in
      let (x, y, w, h) = (CGFloat(point.x), CGFloat(point.y), CGFloat(point.w), CGFloat(point.h))
      switch orientation {
      case 2: return AFRect(x: 1 - x, y: y, w: w, h: h, inFocus: point.inFocus)
      case 3: return AFRect(x: 1 - x, y: 1 - y, w: w, h: h, inFocus: point.inFocus)
      case 4: return AFRect(x: x, y: 1 - y, w: w, h: h, inFocus: point.inFocus)
      case 5: return AFRect(x: y, y: x, w: h, h: w, inFocus: point.inFocus)
      case 6: return AFRect(x: 1 - y, y: x, w: h, h: w, inFocus: point.inFocus)
      case 7: return AFRect(x: 1 - y, y: 1 - x, w: h, h: w, inFocus: point.inFocus)
      case 8: return AFRect(x: y, y: 1 - x, w: h, h: w, inFocus: point.inFocus)
      default: return AFRect(x: x, y: y, w: w, h: h, inFocus: point.inFocus)
      }
    }
  }
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

  /// Set before the first layout; every host in the group zooms and pans together.
  var syncGroup: ViewerSyncGroup? {
    didSet {
      host?.syncGroup = syncGroup
      if let host, let syncGroup { syncGroup.add(host) }
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
