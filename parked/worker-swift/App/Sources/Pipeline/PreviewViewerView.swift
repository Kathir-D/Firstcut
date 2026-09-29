// Owner: pipeline.
//
// The real viewer layer: draws a decoded photograph, aspect-fit, and zooms to 100% on a click
// (task.md §9.2).
//
// It conforms to `Views/Viewer/PhotoViewerHost.swift`'s protocol rather than being imported by any
// view, so the swap table in that file stays honest: `AppEnvironment` registers this, the window
// does not know it exists, and changing how the pixels are drawn never touches a file in `Views/`.
//
// ## What is deliberately not here
//
// * **A demosaicer.** 100% zoom shows the camera's embedded preview at 1:1, not sensor data. See
//   `PreviewPipeline`'s header for why that boundary is a decision and not a gap.
// * **Clipping and AF overlays.** They need a decoded full-resolution buffer and are task.md §9.2
//   items of their own; the histogram above is the one overlay-adjacent thing that is cheap
//   enough to be correct already.
//
// The old image stays on screen while a new one decodes, which is what task.md §7.1 asks for on a
// resize: a black flash on every window drag is the single most obvious way to make an app feel
// slow even when it is fast.

import AppKit
import QuartzCore

@MainActor
final class PreviewViewerView: NSView, PhotoViewerHost {
  private let images: EmbeddedPreviewSource
  private let onViewportChange: (CGSize) -> Void

  private var image: CGImage?
  private var zoomScale: Double = 1
  private var isZoomed = false
  private var zoomLock = false
  /// Where a drag started, so a click that became a drag does not toggle zoom (§9.2).
  private var mouseDownPoint: NSPoint?
  private var didDrag = false

  init(images: EmbeddedPreviewSource, onViewportChange: @escaping (CGSize) -> Void = { _ in }) {
    self.images = images
    self.onViewportChange = onViewportChange
    super.init(frame: .zero)
    wantsLayer = true
    let pinch = NSMagnificationGestureRecognizer(target: self, action: #selector(handlePinch(_:)))
    addGestureRecognizer(pinch)
    layer?.backgroundColor = NSColor.clear.cgColor
    layer?.contentsGravity = .resizeAspect
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is not used")
  }

  // MARK: - PhotoViewerHost

  func setPhoto(_ id: PhotoID, aspectRatio: Double) {
    present(id)
  }

  func setViewerState(_ state: ViewerPresentation) {
    zoomLock = state.isZoomLocked
    if !state.isZoomed, isZoomed {
      // Leaving zoom keeps the current frame rather than re-decoding: task.md §9.2's zoom lock
      // exists so a burst can be compared at one magnification, and that means the *pixels* must
      // not change under the user while they are arrowing.
      isZoomed = false
      zoomScale = 1
      needsDisplay = true
    }
  }

  func setViewportSize(_ size: CGSize) {
    guard size.width > 0, size.height > 0 else { return }
    onViewportChange(size)
    // Re-request at the new size. The image that is already up stays up until the new one is
    // ready, which is the whole point of doing this asynchronously.
    if let id = currentID { present(id, force: true) }
  }

  var presentedZoom: Double { zoomScale }

  private var currentID: PhotoID?

  private func present(_ id: PhotoID, force: Bool = false) {
    currentID = id
    // Fit-to-screen means the decoder is asked for the viewer's pixel size, not a constant: a
    // Retina 16" and an external 5K want very different images out of the same photograph.
    let maxPixel = fittedMaxPixel()
    images.prefetchAround(id, maxPixel: maxPixel)
    if let cached = images.displayImage(for: id) {
      image = cached
      needsDisplay = true
      return
    }
    if !force, image != nil { return }
    needsDisplay = true
  }

  /// The longest edge of the view, at backing scale. `bounds` is already in points and
  /// `backingScaleFactor` converts to pixels, so this is the real number of pixels on screen.
  private func fittedMaxPixel() -> Int {
    let scale = window?.backingScaleFactor ?? 1
    let pixels = max(bounds.width, bounds.height) * scale
    return Int(pixels.rounded(.up)).clamped(to: 256...8192)
  }

  // MARK: - Drawing

  override func draw(_ dirtyRect: NSRect) {
    guard let context = NSGraphicsContext.current?.cgContext else { return }
    if let image {
      draw(image, in: context)
    } else {
      // No image and none on the way: say so, rather than leaving a rectangle the user has to guess
      // about. "No embedded preview" is a real answer -- TIFF and PNG shoots have none.
      drawPlaceholder()
    }
  }

  private func draw(_ image: CGImage, in context: CGContext) {
    context.saveGState()
    defer { context.restoreGState() }
    context.interpolationQuality = .high

    let fitted = aspectFitRect(for: image)
    if !isZoomed {
      context.translateBy(x: fitted.origin.x, y: fitted.origin.y)
      context.draw(image, in: CGRect(origin: .zero, size: fitted.size))
      return
    }

    // Zoomed: centre the image and scale about the centre, which is what "100% centred on this
    // spot" means when the spot is not tracked across a resize.
    context.translateBy(x: bounds.midX, y: bounds.midY)
    context.scaleBy(x: zoomScale, y: zoomScale)
    context.translateBy(x: -fitted.size.width / 2, y: -fitted.size.height / 2)
    context.draw(image, in: CGRect(origin: .zero, size: fitted.size))
  }

  private func drawPlaceholder() {
    let text = "No embedded preview"
    let attributes: [NSAttributedString.Key: Any] = [
      .font: NSFont.systemFont(ofSize: 12),
      .foregroundColor: NSColor.secondaryLabelColor,
    ]
    let size = text.size(withAttributes: attributes)
    text.draw(
      at: NSPoint(x: bounds.midX - size.width / 2, y: bounds.midY - size.height / 2),
      withAttributes: attributes)
  }

  /// Aspect-fit into `bounds`, centred, with the rounded corners of Finder's gallery (§9.2).
  private func aspectFitRect(for image: CGImage) -> NSRect {
    let inset = bounds.insetBy(dx: 12, dy: 12)
    guard inset.width > 0, inset.height > 0 else { return bounds }
    let scale = min(inset.width / CGFloat(image.width), inset.height / CGFloat(image.height))
    let size = NSSize(width: CGFloat(image.width) * scale, height: CGFloat(image.height) * scale)
    return NSRect(
      x: bounds.midX - size.width / 2, y: bounds.midY - size.height / 2,
      width: size.width, height: size.height)
  }

  // MARK: - Zoom (task.md §9.2: mouse only, never a keyboard shortcut)

  override func mouseDown(with event: NSEvent) {
    mouseDownPoint = convert(event.locationInWindow, from: nil)
    didDrag = false
  }

  override func mouseDragged(with event: NSEvent) {
    guard mouseDownPoint != nil else { return }
    didDrag = true
    // Panning only means anything while zoomed; dragging on a fit image is not a gesture this app
    // has, and pretending otherwise makes a fit image feel like it is stuck.
    guard isZoomed else { return }
    let now = convert(event.locationInWindow, from: nil)
    layer?.setAffineTransform(
      CGAffineTransform(translationX: now.x - (mouseDownPoint?.x ?? 0), y: 0))
  }

  override func mouseUp(with event: NSEvent) {
    let wasDrag = didDrag
    let start = mouseDownPoint
    mouseDownPoint = nil
    didDrag = false
    layer?.setAffineTransform(.identity)
    // "A click that turned into a drag must not toggle zoom" -- so the drag flag decides, not the
    // distance, which is what makes a slightly shaky click still count as a click.
    guard !wasDrag, let start else { return }

    if isZoomed {
      isZoomed = false
      zoomScale = 1
      needsDisplay = true
      return
    }
    zoomTo(convert(event.locationInWindow, from: nil))
  }

  override func scrollWheel(with event: NSEvent) {
    // Two-finger scroll pans while zoomed (task.md §9.2). Magnification is handled by the host's
    // pinch recogniser; this is only the scroll.
    guard isZoomed, event.scrollingDeltaY != 0 else { return }
    let dy = event.scrollingDeltaY
    layer?.setAffineTransform(
      CGAffineTransform(translationX: layer?.affineTransform().tx ?? 0, y: dy))
  }

  /// 100% centred on the clicked spot, in one step (§9.2: "a single step, no multi-level zoom").
  private func zoomTo(_ point: NSPoint) {
    guard let image else { return }
    // "100%" means one image pixel per screen pixel, and the image is drawn fitted, so the ratio
    // is the screen's scale over the fit scale.
    zoomScale = currentBaseScale(for: image)
    isZoomed = zoomScale > 1.001
    needsDisplay = true
  }

  /// Pinch to zoom (task.md §9.2, "pinch to zoom in and out smoothly, anchored at the pinch
  /// point").
  ///
  /// A gesture recogniser rather than the legacy `magnify(_:)`: the legacy form mutates the event,
  /// which no longer compiles, and the recogniser gives a continuous value so a pinch accumulates
  /// smoothly instead of stepping.
  @objc private func handlePinch(_ recognizer: NSMagnificationGestureRecognizer) {
    guard let image else { return }
    switch recognizer.state {
    case .began:
      // Anchor: remember the scale the gesture started from, so the pinch is relative to where the
      // user grabbed it rather than to wherever the last pinch left off.
      pinchBaseScale = currentBaseScale(for: image)
    case .changed:
      let base = pinchBaseScale ?? 1
      zoomScale = min(max(base * (1 + recognizer.magnification), 1), 12)
      isZoomed = zoomScale > 1.001
      if !isZoomed { zoomScale = 1 }
      needsDisplay = true
    case .ended, .cancelled, .failed:
      pinchBaseScale = nil
    default:
      break
    }
  }

  /// The zoom at which the embedded preview's pixels map 1:1 to the screen's.
  private func currentBaseScale(for image: CGImage) -> Double {
    let fitted = aspectFitRect(for: image)
    let displayScale = fitted.width / CGFloat(image.width)
    return Double((window?.backingScaleFactor ?? 1) / displayScale)
  }

  private var pinchBaseScale: Double?

  public override var acceptsFirstResponder: Bool { true }
  override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}

extension Comparable {
  func clamped(to limits: ClosedRange<Self>) -> Self {
    min(max(self, limits.lowerBound), limits.upperBound)
  }
}
