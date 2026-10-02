// Owner: pipeline.
//
// The implementation of ui's `PhotoViewerHost` protocol: a plain `CALayer` whose `contents` is the
// decoded `CGImage`. This is the "display = pointer swap" property todo.md §7.1 asks for —
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
// ## Zoom (todo.md §9.2) — mouse and trackpad only, no keyboard shortcut
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
import Observation
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

    /// See `PhotoViewerHost.onFramePresented`. Set by whoever builds the host; nil in previews and
    /// tests, which is why every call site tolerates it.
    var onFramePresented: ((PresentedFrame) -> Void)?

    /// See `PhotoViewerHost.onViewportPixelSize`: this host's frame in **backing pixels**, which is
    /// what the pipeline has to decode T2 at.
    var onViewportPixelSize: ((CGSize) -> Void)?

    /// A double-click in the loupe. `direction` is +1 for the right button of the pair, -1 for the
    /// left, so the app can move a whole batch in the direction the photographer is pointing.
    ///
    /// Set by whoever builds the host; nil in previews. This exists because the loupe's own
    /// `mouseUp` is already spent: a single click zooms to 100% and a second one zooms back, so a
    /// double-click cannot be handled there without breaking both.
    var onDoubleClick: ((Int) -> Void)?

    /// The last size reported, so a resize is not re-reported on every `layout()` and the debug HUD can
    /// show what the decode is being sized from.
    private(set) var reportedViewportPixels: CGSize = .zero

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
        layoutImage(animated: false)
    }

    func setViewerState(_ state: ViewerPresentation) {
        let lockChanged = isZoomLocked != state.isZoomLocked
        isZoomLocked = state.isZoomLocked
        let overlaysChanged =
            presentation.afRects != state.afRects || presentation.showsClipping != state.showsClipping
            || presentation.clippingHighlight != state.clippingHighlight
            || presentation.clippingShadow != state.clippingShadow
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
        reportViewportSize()
        layoutImage(animated: false)
    }

    override func layout() {
        super.layout()
        reportViewportSize()
        layoutImage(animated: false)
    }

    /// Tells the model how many pixels this view covers, so T2 is decoded at exactly that size
    /// (todo.md §7.2: no double resampling, no GPU minification blur). Reported on every layout but
    /// only *sent* when it changes, because `layout()` runs constantly and a window resize storm
    /// would otherwise re-report the size on every pass.
    private func reportViewportSize() {
        guard bounds.width > 0, bounds.height > 0 else { return }
        let scale = window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2
        let pixels = CGSize(
            width: (bounds.width * scale).rounded(), height: (bounds.height * scale).rounded())
        guard pixels != reportedViewportPixels else { return }
        reportedViewportPixels = pixels
        onViewportPixelSize?(pixels)
    }

    /// The longest edge the display bitmap has to have for what this view is showing right now:
    /// the window's own pixels when the photograph is fitted to it, and the photograph's own pixels at
    /// 100% — which is todo.md §7.1's T3, and the only thing that makes 100% honest now that T2 is
    /// smaller than the file.
    private var neededEdge: Int {
        guard zoom > 1.001 else {
            return Int(max(reportedViewportPixels.width, reportedViewportPixels.height).rounded(.up))
        }
        let photoEdge = max(presentation.pixelSize.width, presentation.pixelSize.height)
        return photoEdge > 0 ? Int(photoEdge.rounded(.up)) : 0
    }

    // MARK: - Presentation

    private func present() {
        guard let id = photoID else {
            image = nil
            imageLayer.contents = nil
            updateOverlays()
            return
        }
        // Synchronous on purpose: `ImageProvider` returns a cached `CGImage` when the photo is inside
        // the focus window, which is the state navigation moves through. A miss returns nil (and the
        // focus counter records it, so a regression shows up in a test), and then:
        //
        // * the thumbnail stands in, so the photograph on screen is always the one being rated and
        //   never the previous frame left behind;
        // * the read is observed, so the full decode replaces the thumbnail the moment it lands.
        var decoded: CGImage?
        withObservationTracking {
            decoded = images.displayImage(for: id, minimumLongestEdge: neededEdge)
        } onChange: { [weak self] in
            Task { @MainActor [weak self] in
                guard let self, self.photoID == id, !self.showsFullImage else { return }
                self.present()
            }
        }
        let next = decoded ?? images.thumbnail(for: id, size: CGSize(width: 1024, height: 1024))
        showsFullImage = decoded != nil
        guard next !== image else { return }
        image = next
        // The transaction's completion block runs after this run loop turn's layer tree is committed,
        // which is the "the user can see the new photo" moment that todo.md §7.3 is written against.
        // Waiting for a display link instead would measure scan-out on a different schedule than the one
        // the user pressed the key on.
        let presented = onFramePresented
        let kind: PresentedFrame = decoded != nil ? .display : .standIn
        if let presented {
            CATransaction.begin()
            CATransaction.setCompletionBlock { presented(kind) }
        }
        imageLayer.contents = next
        if presented != nil { CATransaction.commit() }
        updateOverlays()
        layoutImage(animated: false)
    }

    /// Whether `image` is the full display decode rather than the stand-in thumbnail.
    private var showsFullImage = false

    // MARK: - Geometry

    /// The image's size in points when the whole photograph fits the view, or `.zero` with no image.
    private var fitSize: CGSize {
        guard let image, image.width > 0, image.height > 0, bounds.width > 0, bounds.height > 0 else {
            return .zero
        }
        let scale = min(bounds.width / CGFloat(image.width), bounds.height / CGFloat(image.height))
        return CGSize(width: CGFloat(image.width) * scale, height: CGFloat(image.height) * scale)
    }

    /// `zoom` at which one **photograph** pixel is one screen pixel: 100%.
    ///
    /// From `presentation.pixelSize`, not from the bitmap: T2 is decoded at the viewer's size, so a
    /// view that took the bitmap's own width would report a 3000 px half-size photograph as 100% and
    /// the user would see a soft picture labelled as the sharpest thing the app can show.
    private var oneToOne: CGFloat {
        guard let image, fitSize.width > 0 else { return 1 }
        let pointsPerPixel = 1 / (window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2)
        let photoWidth =
            presentation.pixelSize.width > 0 ? presentation.pixelSize.width : CGFloat(image.width)
        return max(1, photoWidth * pointsPerPixel / fitSize.width)
    }

    private var maxZoom: CGFloat { max(oneToOne * 3, 4) }

    /// Where the photograph is drawn, in this view's coordinates (AppKit: bottom-left origin).
    /// Test hooks, because the double-click's whole job is deciding *which half* of the photograph
    /// was clicked, and there is no way to assert that without a real image and its real rect.
    /// Narrow on purpose: a hook that set the image from the outside would let a test pass without
    /// the geometry being right.
    func setTestImage(_ image: CGImage) { self.image = image }
    var testImageRect: CGRect { imageRect(zoom: zoom, center: center) }

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
        // The click-to-100% glide is decoration, so it is dropped when Reduce Motion is on.
        if animated, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
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
        for sublayer in overlayLayer.sublayers ?? [] {
            sublayer.removeFromSuperlayer()
        }
        clippingLayer = nil
        afLayers = []
        if presentation.showsClipping, let image {
            let mask = CALayer()
            mask.contentsGravity = .resize
            mask.contents = ClippingMask.make(
                from: image, highlight: presentation.clippingHighlight, shadow: presentation.clippingShadow)
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
        defer {
            mouseDownPoint = nil
            // A double-click is reported from here rather than from a gesture recogniser, because
            // AppKit delivers both clicks to the same view and the second one is the only one that
            // carries `clickCount == 2`.
            if event.clickCount == 2, !didDrag, image != nil {
                let point = convert(event.locationInWindow, from: nil)
                if imageRect(zoom: zoom, center: center).contains(point) {
                    let forward = convert(point, to: nil).x >= bounds.midX
                    onDoubleClick?(forward ? 1 : -1)
                }
            }
        }
        // A click that turned into a drag must not toggle zoom (todo.md §9.2).
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

/// Highlight / shadow clipping (todo.md §9.2, key J): red where any channel is at or above the
/// highlight point, blue where **every** channel is at or below the shadow point. Computed from a
/// downsampled copy, because a 24-megapixel frame does not need 24 million tests to show where the
/// sky blew out.
///
/// The thresholds are per-channel byte clip points and they come from Settings → Viewer, so a
/// photographer can see a *near*-clipped sky. The defaults are the numbers this always used
/// (250/255 ≈ 0.98 and 5/255 ≈ 0.02): conservative, so the overlay shows what is actually clipping
/// rather than what is nearly clipping. `shadow >= highlight` would paint every pixel one colour,
/// so it is prevented rather than painted.
enum ClippingMask {
    /// Any channel at or above this clips (highlight).
    static let defaultHighlight: UInt8 = 250
    /// Every channel at or below this clips (shadow).
    static let defaultShadow: UInt8 = 5

    /// Where the two overlay colours come from: premultiplied red/blue at ~70%, so the picture is
    /// still readable underneath.
    private static let overlayAlpha: UInt8 = 178

    /// `Settings → Viewer`'s normalized 0…1 thresholds as byte clip points, clamped and ordered.
    ///
    /// The settings model stores 0…1 because that is what a slider wants; the mask tests bytes. The
    /// conversion lives here so the two cannot drift, and so the model defaults can be written as the
    /// fractions they really are.
    public static func thresholds(
        highlight: Double, shadow: Double
    ) -> (highlight: UInt8, shadow: UInt8) {
        let upper = Int((min(max(highlight, 0), 1) * 255).rounded())
        let lower = Int((min(max(shadow, 0), 1) * 255).rounded())
        // Clipping both ends of the range at once is not a display setting, it is a mistake: keep
        // the highlight above the shadow so every pixel has at most one verdict. In `Int`, because
        // `lower + 1` on a `UInt8` shadow of 255 would trap — which is exactly what a hand-edited
        // settings file can ask for.
        return upper > lower
            ? (UInt8(upper), UInt8(lower)) : (UInt8(max(upper, min(lower + 1, 255))), UInt8(lower))
    }

    static func make(
        from image: CGImage, highlight: UInt8 = defaultHighlight, shadow: UInt8 = defaultShadow,
        maxEdge: Int = 1024
    ) -> CGImage? {
        let longest = max(image.width, image.height)
        guard longest > 0 else { return nil }
        let scale = min(1, Double(maxEdge) / Double(longest))
        let width = max(1, Int(Double(image.width) * scale))
        let height = max(1, Int(Double(image.height) * scale))
        // Reading the pixels correctly took two bugs' worth of care, and the layout below is
        // measured rather than assumed (`ClippingMaskTests` is why it can be):
        //
        // * The byte order must be **pinned**. With `premultipliedLast` on its own it was not: the
        //   buffer came back in an order where byte 0 was the *alpha* channel, so `>= 250` was true
        //   for every pixel of an opaque photograph and the J overlay painted the whole picture red.
        // * The buffer must be **opaque**, not premultiplied: drawing into a premultiplied buffer
        //   round-trips every pixel through an unpremultiply that turns premultiplied black
        //   (0, 0, 0, 255) into transparent black (0, 0, 0, 0) — precisely the pixel the shadow
        //   overlay exists to find.
        //
        // `noneSkipFirst | byteOrder32Little` is the layout §7.1 already forces on every cached
        // bitmap (`DecodeEngine.displayBitmapInfo`), so for the common case this draw is a straight
        // copy with no conversion at all. Measured, in memory, it is **[B, G, R, A]**: the three
        // colour bytes are at offsets 0, 1, 2 and alpha is at offset 3.
        let sourceRowBytes = width * 4
        var source = [UInt8](repeating: 0, count: sourceRowBytes * height)
        let space = CGColorSpaceCreateDeviceRGB()
        let readLayout =
            CGImageAlphaInfo.noneSkipFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue
        let maskLayout =
            CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Little.rawValue
        let drew = source.withUnsafeMutableBytes { buffer -> Bool in
            guard
                let context = CGContext(
                    data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8,
                    bytesPerRow: sourceRowBytes, space: space, bitmapInfo: readLayout)
            else { return false }
            // No interpolation: the mask is a threshold test, and averaging two neighbouring pixels
            // to decide whether either clips is how a threshold stops being a threshold.
            context.interpolationQuality = .none
            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        guard drew else { return nil }

        let maskRowBytes = width * 4
        var out = [UInt8](repeating: 0, count: maskRowBytes * height)
        var index = 0
        while index < source.count {
            // Offsets 0, 1, 2 are the three colour channels; offset 3 is alpha (see `readLayout`).
            // The test is over all three and over any of them, so it does not matter which is which:
            // a blown-out channel is blown out whatever it is called.
            let first = source[index]
            let second = source[index + 1]
            let third = source[index + 2]
            if first >= highlight || second >= highlight || third >= highlight {
                // Premultiplied red at ~70%.
                out[index] = overlayAlpha
                out[index + 3] = overlayAlpha
            } else if first <= shadow && second <= shadow && third <= shadow {
                out[index + 2] = overlayAlpha
                out[index + 3] = overlayAlpha
            }
            index += 4
        }
        return out.withUnsafeMutableBytes { buffer -> CGImage? in
            guard
                let context = CGContext(
                    data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8,
                    bytesPerRow: maskRowBytes, space: space, bitmapInfo: maskLayout)
            else { return nil }
            return context.makeImage()
        }
    }
}
