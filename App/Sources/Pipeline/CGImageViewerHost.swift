// Owner: pipeline.
//
// The implementation of ui's `PhotoViewerHost` protocol: a plain `CALayer` whose `contents` is the
// decoded `CGImage`. This is the "display = pointer swap" property task.md §7.1 asks for —
// navigation sets a new image, and nothing is re-allocated per frame.
//
// ## Why a CALayer and not an IOSurface (REV-38, REV-39)
//
// pipeline-api.md said `CALayer.contents = IOSurfaceRef`, which cannot work: `contents` takes a
// `CGImage`, and an `IOSurfaceRef` is not accepted. The IOSurface path needs a `CAMetalLayer` plus a
// `CVMetalTextureCache` and a Metal drawable, which buys nothing for v0.1.0: the cost in §7.1 is the
// *decode*, and `ImageProvider` already decodes off the main thread into a cache. So the viewer
// holds a `CGImage` and assigns it to a layer, and the cache is what keeps a decode off the
// navigation path. `focusMisses` is the counter that proves it, and it is 0 in the perf tests.
//
// The seam is unchanged: if the IOSurface path is ever worth it, it replaces *this* file and
// `ViewerArea` does not move. That is what the protocol was for (REV-52).
//
// ## Zoom (task.md §9.2) — mouse and trackpad only, no keyboard shortcut
//
// * **Pinch** zooms smoothly, anchored at the pinch point.
// * **Click** a spot jumps to 100% (one image pixel per screen pixel) *centred on that spot*;
//   click again returns to fit. A click that turned into a drag does neither.
// * While zoomed, **drag** or **two-finger scroll** pans.
// * **Zoom lock**: arrowing to the next frame keeps the same zoom level and the same spot, so
//   sharpness can be compared across a burst. Without it, every photo opens at fit.
//
// The geometry is one function (`imageRect`) from three numbers — the image size, the zoom relative
// to fit, and the normalized point of the image that sits at the centre of the view — so pinch,
// click, pan and lock cannot disagree about where the photograph is.

import AppKit
import QuartzCore

@MainActor
final class CGImageViewerHost: NSView, PhotoViewerHost {
  private let images: any CullImageSource
  private let imageLayer = CALayer()
  private let overlayLayer = CALayer()

  private var photoID: PhotoID?
  private var image: CGImage?

  /// Zoom relative to *fit*: 1 is the whole photograph, `oneToOne` is 100%.
  private var zoom: CGFloat = 1
  /// The point of the image (0…1, top-left origin) that sits at the centre of the view.
  private var center = CGPoint(x: 0.5, y: 0.5)
  private var isZoomLocked = false
  private var presentation = ViewerPresentation.fit

  /// Set by Compare: the other panes follow whatever the user does to this one.
  var syncGroup: ViewerSyncGroup?

  // Gesture bookkeeping.
  private var mouseDownPoint: CGPoint?
  private var mouseDownCenter = CGPoint(x: 0.5, y: 0.5)
  private var didDrag = false
  private let dragSlop: CGFloat = 3

  /// The zoom the view is presenting, for the HUD, as a multiple of fit. Read on demand.
  var presentedZoom: Double { Double(zoom) }

  init(images: any CullImageSource) {
    self.images = images
    super.init(frame: .zero)
    wantsLayer = true
    layer?.backgroundColor = NSColor.clear.cgColor

    imageLayer.contentsGravity = .resize
    imageLayer.magnificationFilter = .linear
    imageLayer.minificationFilter = .trilinear
    imageLayer.masksToBounds = true
    layer?.addSublayer(imageLayer)

    overlayLayer.masksToBounds = true
    imageLayer.addSublayer(overlayLayer)

    let pinch = NSMagnificationGestureRecognizer(target: self, action: #selector(pinched(_:)))
    addGestureRecognizer(pinch)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is not used")
  }

  // MARK: - PhotoViewerHost

  func setPhoto(_ id: PhotoID, aspectRatio: Double) {
    let changed = photoID != id
    photoID = id
    if changed, !isZoomLocked {
      // A new photo opens at fit, unless the user has locked the zoom to compare a burst.
      zoom = 1
      center = CGPoint(x: 0.5, y: 0.5)
    }
    present()
  }

  func setViewerState(_ state: ViewerPresentation) {
    let lockChanged = isZoomLocked != state.isZoomLocked
    isZoomLocked = state.isZoomLocked
    let overlaysChanged =
      presentation.afRects != state.afRects || presentation.showsClipping != state.showsClipping
    presentation = state
    if lockChanged || overlaysChanged { updateOverlays() }
    layoutImage(animated: false)
  }

  func applySynced(_ state: ViewerSyncGroup.State) {
    zoom = state.zoom
    center = state.center
    layoutImage(animated: false)
  }

  private func broadcast() {
    syncGroup?.broadcast(.init(zoom: zoom, center: center), from: self)
  }

  func setViewportSize(_ size: CGSize) {
    // A size change is a transform, not a decode: §7.1's budget is about decodes.
    layoutImage(animated: false)
  }

  override func layout() {
    super.layout()
    layoutImage(animated: false)
  }

  // MARK: - Presentation

  private func present() {
    guard let photoID else {
      image = nil
      imageLayer.contents = nil
      return
    }
    // Synchronous on purpose: `ImageProvider` returns a cached `CGImage` when the photo is inside
    // the focus window, which is the state navigation moves through. A miss returns nil and the
    // focus counter records it, so a regression here shows up in a test rather than as a blank
    // viewer nobody measures.
    if let decoded = images.displayImage(for: photoID) {
      image = decoded
      imageLayer.contents = decoded
    }
    updateOverlays()
    layoutImage(animated: false)
  }

  // MARK: - Geometry

  /// The image's size in points when the whole photograph fits the view, or `.zero` with no image.
  private var fitSize: CGSize {
    guard let image, image.width > 0, image.height > 0, bounds.width > 0, bounds.height > 0 else {
      return .zero
    }
    let scale = min(bounds.width / CGFloat(image.width), bounds.height / CGFloat(image.height))
    return CGSize(width: CGFloat(image.width) * scale, height: CGFloat(image.height) * scale)
  }

  /// `zoom` at which one image pixel is one screen pixel: 100%.
  private var oneToOne: CGFloat {
    guard let image, fitSize.width > 0 else { return 1 }
    let pointsPerPixel = 1 / (window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2)
    return max(1, CGFloat(image.width) * pointsPerPixel / fitSize.width)
  }

  private var maxZoom: CGFloat { max(oneToOne * 3, 4) }

  /// Where the photograph is drawn, in this view's coordinates (AppKit: bottom-left origin).
  private func imageRect(zoom: CGFloat, center: CGPoint) -> CGRect {
    let fit = fitSize
    guard fit != .zero else { return .zero }
    let size = CGSize(width: fit.width * zoom, height: fit.height * zoom)
    // `center` is top-left based; flip y for AppKit's bottom-left layer coordinates.
    var origin = CGPoint(
      x: bounds.midX - center.x * size.width,
      y: bounds.midY - (1 - center.y) * size.height)
    // Never leave a gap on a side the photograph could cover, and centre it where it is smaller
    // than the view (which is exactly fit).
    origin.x = clampedOrigin(origin.x, size: size.width, viewport: bounds.width)
    origin.y = clampedOrigin(origin.y, size: size.height, viewport: bounds.height)
    return CGRect(origin: origin, size: size)
  }

  private func clampedOrigin(_ origin: CGFloat, size: CGFloat, viewport: CGFloat) -> CGFloat {
    if size <= viewport { return (viewport - size) / 2 }
    return min(0, max(viewport - size, origin))
  }

  /// The inverse of `imageRect` for a point in view coordinates: the normalized (top-left) point
  /// of the image under it.
  private func imagePoint(at viewPoint: CGPoint, in rect: CGRect) -> CGPoint {
    guard rect.width > 0, rect.height > 0 else { return CGPoint(x: 0.5, y: 0.5) }
    return CGPoint(
      x: min(1, max(0, (viewPoint.x - rect.minX) / rect.width)),
      y: min(1, max(0, 1 - (viewPoint.y - rect.minY) / rect.height)))
  }

  private func layoutImage(animated: Bool) {
    let rect = imageRect(zoom: zoom, center: center)
    // Keep `center` honest after clamping, so a later pan starts from what is on screen.
    if rect != .zero, zoom > 1 {
      center = imagePoint(at: CGPoint(x: bounds.midX, y: bounds.midY), in: rect)
    }
    CATransaction.begin()
    if animated {
      CATransaction.setAnimationDuration(0.22)
      CATransaction.setAnimationTimingFunction(CAMediaTimingFunction(name: .easeInEaseOut))
    } else {
      CATransaction.setDisableActions(true)
    }
    imageLayer.frame = rect
    overlayLayer.frame = imageLayer.bounds
    relayoutOverlays()
    CATransaction.commit()
  }

  // MARK: - Overlays (AF points, clipping)

  private var clippingLayer: CALayer?
  private var afLayers: [(layer: CAShapeLayer, rect: ViewerPresentation.AFRect)] = []

  private func updateOverlays() {
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    overlayLayer.sublayers?.forEach { $0.removeFromSuperlayer() }
    clippingLayer = nil
    afLayers = []

    if presentation.showsClipping, let image {
      let mask = CALayer()
      mask.contentsGravity = .resize
      mask.contents = ClippingMask.make(from: image)
      overlayLayer.addSublayer(mask)
      clippingLayer = mask
    }
    for rect in presentation.afRects {
      let layer = CAShapeLayer()
      layer.fillColor = nil
      layer.lineWidth = 2
      // Green is "in focus" the way a camera draws it; white is a point that was active but missed.
      layer.strokeColor =
        (rect.inFocus ? NSColor.systemGreen : NSColor.white.withAlphaComponent(0.6)).cgColor
      overlayLayer.addSublayer(layer)
      afLayers.append((layer, rect))
    }
    relayoutOverlays()
    CATransaction.commit()
  }

  /// Overlays are laid out from normalized geometry every time the photograph moves, so they stay
  /// on the pixels they mark through pinch, pan and resize.
  private func relayoutOverlays() {
    let frame = overlayLayer.bounds
    clippingLayer?.frame = frame
    for (layer, rect) in afLayers {
      layer.frame = frame
      // `rect` is normalized with a top-left origin; layer coordinates are bottom-left.
      let box = CGRect(
        x: (rect.x - rect.w / 2) * frame.width,
        y: (1 - rect.y - rect.h / 2) * frame.height,
        width: rect.w * frame.width,
        height: rect.h * frame.height)
      layer.path = CGPath(rect: box, transform: nil)
    }
  }

  // MARK: - Input

  override var acceptsFirstResponder: Bool { false }

  override func mouseDown(with event: NSEvent) {
    mouseDownPoint = convert(event.locationInWindow, from: nil)
    mouseDownCenter = center
    didDrag = false
  }

  override func mouseDragged(with event: NSEvent) {
    guard let start = mouseDownPoint else { return }
    let point = convert(event.locationInWindow, from: nil)
    if !didDrag, hypot(point.x - start.x, point.y - start.y) < dragSlop { return }
    didDrag = true
    guard zoom > 1.001 else { return }
    let size = imageRect(zoom: zoom, center: center).size
    guard size.width > 0, size.height > 0 else { return }
    // Dragging the photograph right moves the view's centre left over the image.
    center = CGPoint(
      x: min(1, max(0, mouseDownCenter.x - (point.x - start.x) / size.width)),
      y: min(1, max(0, mouseDownCenter.y + (point.y - start.y) / size.height)))
    layoutImage(animated: false)
    broadcast()
  }

  override func mouseUp(with event: NSEvent) {
    defer { mouseDownPoint = nil }
    // A click that turned into a drag must not toggle zoom (task.md §9.2).
    guard !didDrag, image != nil else { return }
    let point = convert(event.locationInWindow, from: nil)
    let rect = imageRect(zoom: zoom, center: center)
    guard rect.contains(point) else { return }

    if zoom > 1.001 {
      zoom = 1
      center = CGPoint(x: 0.5, y: 0.5)
    } else {
      // 100%, centred on the spot that was clicked.
      center = imagePoint(at: point, in: rect)
      zoom = oneToOne
    }
    layoutImage(animated: true)
    broadcast()
  }

  override func scrollWheel(with event: NSEvent) {
    guard zoom > 1.001 else {
      super.scrollWheel(with: event)
      return
    }
    let size = imageRect(zoom: zoom, center: center).size
    guard size.width > 0, size.height > 0 else { return }
    center = CGPoint(
      x: min(1, max(0, center.x - event.scrollingDeltaX / size.width)),
      y: min(1, max(0, center.y - event.scrollingDeltaY / size.height)))
    layoutImage(animated: false)
    broadcast()
  }

  @objc private func pinched(_ recognizer: NSMagnificationGestureRecognizer) {
    guard image != nil else { return }
    switch recognizer.state {
    case .began, .changed:
      let anchorView = recognizer.location(in: self)
      let rect = imageRect(zoom: zoom, center: center)
      let anchor = imagePoint(at: anchorView, in: rect)
      let factor = 1 + recognizer.magnification
      recognizer.magnification = 0
      let next = min(maxZoom, max(1, zoom * factor))
      guard next != zoom else { return }
      // Keep the image point under the fingers where it is: solve for the centre that puts
      // `anchor` back under `anchorView` at the new zoom.
      let fit = fitSize
      let newSize = CGSize(width: fit.width * next, height: fit.height * next)
      if next <= 1.001 {
        zoom = 1
        center = CGPoint(x: 0.5, y: 0.5)
      } else if newSize.width > 0, newSize.height > 0 {
        zoom = next
        center = CGPoint(
          x: anchor.x + (bounds.midX - anchorView.x) / newSize.width,
          y: anchor.y - (bounds.midY - anchorView.y) / newSize.height)
        center.x = min(1, max(0, center.x))
        center.y = min(1, max(0, center.y))
      }
      layoutImage(animated: false)
      broadcast()
    default:
      break
    }
  }
}

// MARK: - Clipping mask

/// Highlight / shadow clipping (task.md §9.2, key J): red where any channel is at the top of its
/// range, blue where every channel is at the bottom. Computed from a downsampled copy, because a
/// 24-megapixel frame does not need 24 million tests to show where the sky blew out.
enum ClippingMask {
  static let highlight: UInt8 = 250
  static let shadow: UInt8 = 5

  static func make(from image: CGImage, maxEdge: Int = 1024) -> CGImage? {
    let longest = max(image.width, image.height)
    guard longest > 0 else { return nil }
    let scale = min(1, Double(maxEdge) / Double(longest))
    let width = max(1, Int(Double(image.width) * scale))
    let height = max(1, Int(Double(image.height) * scale))
    let bytesPerRow = width * 4
    var source = [UInt8](repeating: 0, count: bytesPerRow * height)
    let space = CGColorSpaceCreateDeviceRGB()
    let drew = source.withUnsafeMutableBytes { buffer -> Bool in
      guard
        let context = CGContext(
          data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8,
          bytesPerRow: bytesPerRow, space: space,
          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
      else { return false }
      context.interpolationQuality = .medium
      context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
      return true
    }
    guard drew else { return nil }

    var out = [UInt8](repeating: 0, count: bytesPerRow * height)
    var index = 0
    while index < source.count {
      let r = source[index]
      let g = source[index + 1]
      let b = source[index + 2]
      if r >= highlight || g >= highlight || b >= highlight {
        // Premultiplied red at ~70%.
        out[index] = 178
        out[index + 3] = 178
      } else if r <= shadow && g <= shadow && b <= shadow {
        out[index + 2] = 178
        out[index + 3] = 178
      }
      index += 4
    }
    return out.withUnsafeMutableBytes { buffer -> CGImage? in
      guard
        let context = CGContext(
          data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8,
          bytesPerRow: bytesPerRow, space: space,
          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
      else { return nil }
      return context.makeImage()
    }
  }
}
