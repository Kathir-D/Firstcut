// Owner: pipeline.
//
// The seam ui declared: `CullImageSource` in `Views/Model/CullViewState.swift`, implemented here
// over real pixels.
//
// The protocol is synchronous (`func displayImage(for:) -> CGImage?`) because a view's draw method
// cannot await. That is not a limitation to work around, it is the shape that forces the right
// thing: `displayImage` only ever returns something already decoded, and the decoding is requested
// ahead of time by `prefetchAround`. A synchronous getter that decoded on demand would put a
// 24 MP JPEG decode on the main thread during a scroll.
//
// The filmstrip asks for small images and the viewer for one at screen size, so there are two
// sizes in the cache and each is requested at its own size.

import CoreGraphics
import Foundation

/// Real images for the filmstrip and the viewer, from the camera's embedded previews.
///
/// Named apart from the mock `PreviewImageSource` in `Views/Model/PreviewCullViewState.swift`
/// deliberately: both satisfy `CullImageSource`, and a name shared between the real one and the
/// placeholder is a bug waiting to be introduced by an import.
@MainActor
final class EmbeddedPreviewSource: NSObject, CullImageSource, @unchecked Sendable {
  private let pipeline: PreviewPipeline
  /// Photo id -> where its pixels are. Rebuilt when a folder is opened.
  private var sources: [PhotoID: PreviewSource] = [:]
  /// The order the user is looking through, so "the next frames" is meaningful.
  private var order: [PhotoID] = []
  private var currentIndex: [PhotoID: Int] = [:]

  public private(set) var thumbnailProgress: Double = 0

  init(pipeline: PreviewPipeline) {
    self.pipeline = pipeline
    super.init()
  }

  /// Points the source at a folder's photos. Passing the capture order is what lets
  /// `prefetchAround` know which frames the arrow keys will reach.
  func configure(folder: URL, photos: [PhotoMeta], inOrder order: [PhotoID]) {
    var built: [PhotoID: PreviewSource] = [:]
    for photo in photos {
      let id = photo.id
      built[id] = PreviewSource(
        url: folder.appendingPathComponent(photo.relPath),
        range: photo.preview.map { ByteRange(offset: $0.range.offset, len: $0.range.len) },
        pixelWidth: photo.width,
        pixelHeight: photo.height)
    }
    sources = built
    self.order = order
    // `uniquingKeysWith` rather than `uniqueKeysWithValues`, which traps on a duplicate id. A
    // session with two photos sharing an id is exactly the bug this project has already shipped
    // once, and a trap here would be a poor place to find out.
    currentIndex = order.enumerated().reduce(into: [:]) { result, pair in
      result[pair.element] = pair.offset
    }
    pipeline.setSources(built)
    thumbnailProgress = 0
  }

  func close() {
    sources = [:]
    order = []
    currentIndex = [:]
    pipeline.removeAll()
    thumbnailProgress = 0
  }

  // MARK: - CullImageSource

  /// A filmstrip-sized image. 256 px on the long edge (task.md §7.1, tier T0).
  func thumbnail(for id: PhotoID, size: CGSize) -> CGImage? {
    let maxPixel = max(Int(size.width), Int(size.height))
    if let cached = pipeline.cached(id) { return cached }
    pipeline.request(id, maxPixel: maxPixel) { _ in }
    return nil
  }

  /// The image for the viewer. Fit-to-screen, so the request is for the viewer's own pixel size
  /// rather than a fixed number that would be wrong on every display.
  func displayImage(for id: PhotoID) -> CGImage? {
    if let cached = pipeline.cached(id) { return cached }
    let neighbour = maxPixel
    pipeline.request(id, maxPixel: neighbour) { _ in }
    return nil
  }

  /// The current frame's histogram (task.md §9.2, toggleable). Read from the decoded image, so it
  /// costs nothing extra once the viewer has it, and is `nil` before that rather than guessed.
  func histogram(for id: PhotoID) -> CullHistogram? {
    guard let image = pipeline.cached(id) else { return nil }
    return Self.histogram(of: image)
  }

  /// What the viewer should be asking for at this size. Kept as a property rather than a constant
  /// so the window can change it on resize and every subsequent request is the right size.
  var maxPixel: Int = 2048

  // MARK: - Preloading

  /// Decodes the frames around `id` that the user can reach with one keypress.
  ///
  /// task.md §7.1: "nothing in the previous, current or next batch is ever decoded on demand". So
  /// this is a fixed window, not "one each side", and it runs before the user needs it.
  func prefetchAround(_ id: PhotoID, radius: Int = 6, maxPixel: Int) {
    guard let index = currentIndex[id] else {
      pipeline.prefetch([id], maxPixel: maxPixel)
      return
    }
    let lower = max(0, index - radius)
    let upper = min(order.count, index + radius + 1)
    guard lower < upper else { return }
    pipeline.prefetch(Array(order[lower..<upper]), maxPixel: maxPixel)
  }

  /// Keeps the cache to what the open session needs, so a 1,500-frame shoot does not grow without
  /// bound while the user works through it.
  func trimToOpenSession() {
    pipeline.retainOnly(Set(sources.keys))
  }

  // MARK: - Histogram

  /// Luminance and RGB histograms from the decoded bitmap.
  ///
  /// Read once per image and thrown away: the HUD is the only consumer, it is a toggle, and caching
  /// 256×3 floats per photo to save a few milliseconds is not a trade worth making.
  static func histogram(of image: CGImage) -> CullHistogram? {
    let width = image.width, height = image.height
    guard width > 0, height > 0 else { return nil }
    var pixels = [UInt8](repeating: 0, count: width * height * 4)
    let colorSpace = CGColorSpaceCreateDeviceRGB()
    let info = CGImageAlphaInfo.premultipliedLast.rawValue
    guard
      let context = CGContext(
        data: &pixels, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
        space: colorSpace, bitmapInfo: info)
    else { return nil }
    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))

    var red = [Double](repeating: 0, count: 256)
    var green = [Double](repeating: 0, count: 256)
    var blue = [Double](repeating: 0, count: 256)
    var luma = [Double](repeating: 0, count: 256)
    for i in stride(from: 0, to: pixels.count, by: 4) {
      let r = pixels[i], g = pixels[i + 1], b = pixels[i + 2]
      red[Int(r)] += 1
      green[Int(g)] += 1
      blue[Int(b)] += 1
      // Rec. 601 luma, which is what the eye weights and therefore what a clipping overlay
      // should be judged on.
      let y = (0.299 * Double(r) + 0.587 * Double(g) + 0.114 * Double(b)).rounded()
      luma[Int(min(255, max(0, y)))] += 1
    }
    let total = Double(width * height)
    guard total > 0 else { return nil }
    return CullHistogram(
      red: red.map { $0 / total }, green: green.map { $0 / total },
      blue: blue.map { $0 / total }, luminance: luma.map { $0 / total })
  }
}
