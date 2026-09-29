// Owner: ui.
//
// Finder-style filmstrip: the current batch only, one horizontally scrolling strip of thumbnails at
// their own aspect ratio, the selected frame on a rounded gray plate, rating stars/flags/labels in
// stars mode and green/red keep rings in keep mode (task.md §6.1, §6.2, §9.3).
//
// Layer-backed and drawn by hand rather than one view per frame, so 60+ frames stay smooth.

import AppKit
import QuartzCore

struct FilmstripFrame: Identifiable, Equatable {
  var id: PhotoID
  var rating: Rating
  var aspectRatio: Double
}

@MainActor
final class FilmstripScrollView: NSScrollView {
  private let strip = FilmstripContentView()

  init() {
    super.init(frame: .zero)
    drawsBackground = false
    hasVerticalScroller = false
    hasHorizontalScroller = true
    autohidesScrollers = true
    scrollerStyle = .overlay
    documentView = strip
    strip.frame = bounds
    strip.autoresizingMask = []
  }

  override func layout() {
    super.layout()
    let height = bounds.height
    let width = max(strip.frame.width, bounds.width)
    if strip.frame.height != height || strip.frame.width != width {
      strip.setFrameSize(NSSize(width: width, height: height))
    }
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is not used")
  }

  var thumbnailProvider: (@MainActor (PhotoID, CGSize) -> CGImage?)? {
    get { strip.thumbnailProvider }
    set { strip.thumbnailProvider = newValue }
  }

  var onSelect: (@MainActor (PhotoID) -> Void)? {
    get { strip.onSelect }
    set { strip.onSelect = newValue }
  }

  func apply(frames: [FilmstripFrame], selectedID: PhotoID?) {
    strip.apply(frames: frames, selectedID: selectedID)
    DispatchQueue.main.async { [weak self] in self?.revealSelection(animated: false) }
  }

  private func revealSelection(animated: Bool) {
    guard let id = strip.selectedID, let frame = strip.frame(for: id) else { return }
    let visible = strip.convert(frame, to: nil)
    let clip = contentView.bounds
    var offset = contentView.bounds.origin.x
    if visible.minX < clip.minX {
      offset = max(0, frame.minX - 16)
    } else if visible.maxX > clip.maxX {
      offset = visible.maxX - clip.width + 16
    } else {
      return
    }
    var point = clip.origin
    point.x = min(offset, max(0, strip.frame.width - clip.width))
    contentView.scroll(to: point)
    reflectScrolledClipView(contentView)
  }
}

@MainActor
final class FilmstripContentView: NSView {
  var thumbnailProvider: (@MainActor (PhotoID, CGSize) -> CGImage?)?
  var onSelect: (@MainActor (PhotoID) -> Void)?

  private struct FrameLayers {
    let plate: CALayer
    let image: CALayer
    let badge: FilmstripBadgeLayer
    let id: PhotoID
    var frame: CGRect
    var desiredSize: CGSize
    var loadedSize: CGSize
  }

  private var frames: [FilmstripFrame] = []
  private var items: [FrameLayers] = []
  private(set) var selectedID: PhotoID?
  private var ratingMode: RatingMode = .stars
  private var needsThumbnailPass = true
  private var pressedIndex: Int?
  private var thumbnailRetryScheduled = false

  override var isFlipped: Bool { true }
  override var isOpaque: Bool { false }
  override var acceptsFirstResponder: Bool { true }

  override init(frame frameRect: NSRect) {
    super.init(frame: frameRect)
    wantsLayer = true
    setAccessibilityRole(.list)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is not used")
  }

  // MARK: - Content

  func apply(frames newFrames: [FilmstripFrame], selectedID newSelection: PhotoID?) {
    let contentChanged = newFrames != frames
    let selectionChanged = newSelection != selectedID
    guard contentChanged || selectionChanged else { return }
    frames = newFrames
    selectedID = newSelection
    if contentChanged { rebuildLayers() }
    updateSelection()
    needsLayout = true
  }

  func frame(for id: PhotoID) -> CGRect? {
    items.first { $0.id == id }?.frame
  }

  private var hostLayer: CALayer { layer ?? backingLayer }

  private let backingLayer = CALayer()

  // MARK: - Layers

  private func rebuildLayers() {
    if let existing = hostLayer.sublayers {
      for sublayer in existing { sublayer.removeFromSuperlayer() }
    }
    let scale = window?.backingScaleFactor ?? 2
    items = frames.map { frame in
      let plate = CALayer()
      plate.backgroundColor = NSColor.quaternaryLabelColor.withAlphaComponent(0.5).cgColor
      plate.cornerRadius = Appearance.filmstripPlateCornerRadius
      plate.isHidden = true

      let image = CALayer()
      image.cornerRadius = Appearance.filmstripCornerRadius
      image.masksToBounds = true
      image.contentsGravity = .resizeAspect
      image.contentsScale = scale
      image.backgroundColor = NSColor.underPageBackgroundColor.cgColor

      let badge = FilmstripBadgeLayer()
      badge.contentsScale = scale

      hostLayer.addSublayer(plate)
      hostLayer.addSublayer(image)
      hostLayer.addSublayer(badge)

      return FrameLayers(
        plate: plate, image: image, badge: badge, id: frame.id, frame: .zero, desiredSize: .zero,
        loadedSize: .zero)
    }
    needsThumbnailPass = true
  }

  private func updateSelection() {
    for item in items {
      item.plate.isHidden = item.id != selectedID
    }
  }

  // MARK: - Layout

  override func layout() {
    super.layout()
    let inset = Appearance.filmstripInset
    let plateInset = Appearance.filmstripPlateInset
    let gap = Appearance.filmstripGap
    let height = max(1, bounds.height - inset * 2)
    let scale = window?.backingScaleFactor ?? 2

    var x = inset
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    for index in items.indices {
      let aspect = max(0.2, min(5, frames[index].aspectRatio))
      let width = max(28, min(360, (height - plateInset * 2) * aspect))
      let imageFrame = CGRect(
        x: x + plateInset, y: inset + plateInset, width: width, height: height - plateInset * 2)
      let plateFrame = imageFrame.insetBy(dx: -plateInset, dy: -plateInset)

      items[index].frame = imageFrame
      items[index].image.frame = imageFrame
      items[index].plate.frame = plateFrame
      items[index].badge.frame = imageFrame
      items[index].badge.rating = frames[index].rating
      items[index].badge.mode = ratingMode
      items[index].badge.setNeedsDisplay()
      items[index].loadedSize = CGSize(
        width: imageFrame.width * scale, height: imageFrame.height * scale)
      items[index].desiredSize = items[index].loadedSize
      x = plateFrame.maxX + gap
    }
    CATransaction.commit()

    let contentWidth =
      items.last.map { $0.frame.maxX + Appearance.filmstripInset + Appearance.filmstripPlateInset }
      ?? bounds.width
    if frame.width != contentWidth || frame.height != bounds.height {
      setFrameSize(NSSize(width: contentWidth, height: bounds.height))
    }
    loadThumbnails()
  }

  private func loadThumbnails() {
    guard window != nil else { return }
    needsThumbnailPass = false
    for index in items.indices {
      let target = items[index].desiredSize
      guard target != .zero else { continue }
      if items[index].loadedSize == target, items[index].image.contents != nil { continue }
      guard let image = thumbnailProvider?(items[index].id, target) else {
        needsThumbnailPass = true
        continue
      }
      CATransaction.begin()
      CATransaction.setDisableActions(true)
      items[index].image.contents = image
      items[index].image.backgroundColor = nil
      items[index].loadedSize = target
      CATransaction.commit()
    }
    if needsThumbnailPass { scheduleThumbnailRetry() }
  }

  private func scheduleThumbnailRetry() {
    guard !thumbnailRetryScheduled else { return }
    thumbnailRetryScheduled = true
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak self] in
      guard let self else { return }
      self.thumbnailRetryScheduled = false
      guard self.window != nil else { return }
      self.loadThumbnails()
    }
  }

  // MARK: - Interaction

  override func mouseDown(with event: NSEvent) {
    let point = convert(event.locationInWindow, from: nil)
    guard let hit = index(at: point) else { return }
    pressedIndex = hit
  }

  override func mouseUp(with event: NSEvent) {
    defer { pressedIndex = nil }
    guard let pressed = pressedIndex else { return }
    let point = convert(event.locationInWindow, from: nil)
    guard index(at: point) == pressed else { return }
    onSelect?(items[pressed].id)
  }

  private func index(at point: CGPoint) -> Int? {
    items.firstIndex {
      $0.frame.insetBy(dx: -Appearance.filmstripPlateInset, dy: -Appearance.filmstripPlateInset)
        .contains(point)
    }
  }
}
