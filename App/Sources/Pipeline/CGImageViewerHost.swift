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

import AppKit
import QuartzCore

@MainActor
final class CGImageViewerHost: NSView, PhotoViewerHost {
  private let images: any CullImageSource
  private let imageLayer = CALayer()

  private var photoID: PhotoID?
  private var zoom: Double = 1
  private var isZoomLocked = false
  private var anchor: CGPoint = .zero

  /// The zoom the view is currently presenting, derived from the layer's own transform. Read for the
  /// HUD, so it must not be a second stored number that can drift from what is on screen.
  var presentedZoom: Double { Double(imageLayer.transform.m11) }

  init(images: any CullImageSource) {
    self.images = images
    super.init(frame: .zero)
    wantsLayer = true
    layer?.backgroundColor = NSColor.clear.cgColor

    imageLayer.contentsGravity = .resizeAspect
    imageLayer.magnificationFilter = .linear
    imageLayer.minificationFilter = .trilinear
    // The loupe is the whole point of a culling app, so this is the linear-filter quality choice
    // §7.2 asks for: cheap, and it is already a camera preview rather than a RAW decode.
    layer?.addSublayer(imageLayer)

    let click = NSClickGestureRecognizer(target: self, action: #selector(toggleZoom))
    addGestureRecognizer(click)
    let pinch = NSClickGestureRecognizer(target: self, action: #selector(toggleZoom))
    addGestureRecognizer(pinch)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is not used")
  }

  // MARK: - PhotoViewerHost

  func setPhoto(_ id: PhotoID, aspectRatio: Double) {
    photoID = id
    present()
  }

  func setViewerState(_ state: ViewerPresentation) {
    zoom = state.isZoomed ? 2 : 1
    isZoomLocked = state.isZoomLocked
    if !state.isZoomed { anchor = .zero }
    applyTransform()
  }

  func setViewportSize(_ size: CGSize) {
    // Re-frame on layout. The image itself is not re-decoded: a size change is a transform, and
    // §7.1's budget is about decodes.
    imageLayer.frame = bounds
  }

  override func layout() {
    super.layout()
    imageLayer.frame = bounds
  }

  // MARK: - Presentation

  private func present() {
    guard let photoID else {
      imageLayer.contents = nil
      return
    }
    // Synchronous on purpose: `ImageProvider` returns a cached `CGImage` when the photo is inside
    // the focus window, which is the state navigation moves through. A miss returns nil and the
    // focus counter records it, so a regression here shows up in a test rather than as a blank
    // viewer nobody measures.
    if let image = images.displayImage(for: photoID) {
      imageLayer.contents = image
    }
    applyTransform()
  }

  private func applyTransform() {
    guard bounds.width > 0, bounds.height > 0 else { return }
    let scale = zoom.isFinite && zoom > 0 ? zoom : 1
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    // Centre, then scale about the zoom anchor.
    //
    // Setting `position` after `anchorPoint` matters: `anchorPoint` is in unit coordinates of the
    // layer's *own* bounds, and the image is drawn with `resizeAspect` inside `bounds`. Pinning the
    // position to `bounds.midX/midY` with a non-centred anchor put the photograph in the top-right
    // corner at 1:1 — which looked like a decoding failure rather than a layout mistake, and cost a
    // while to tell apart from "the viewer is black".
    imageLayer.contentsGravity = .resizeAspect
    imageLayer.frame = CGRect(origin: .zero, size: bounds.size)
    imageLayer.anchorPoint = CGPoint(x: anchor.x, y: anchor.y)
    imageLayer.position = CGPoint(x: bounds.midX, y: bounds.midY)
    imageLayer.transform = CATransform3DMakeScale(scale, scale, 1)
    CATransaction.commit()
  }

  @objc private func toggleZoom() {
    guard !isZoomLocked else { return }
    zoom = zoom > 1.01 ? 1 : 2
    applyTransform()
  }
}
