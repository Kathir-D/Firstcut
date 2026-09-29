// Owner: pipeline.
//
// Real pixels, from the JPEG the camera already put inside the RAW.
//
// Every other image source in this app is a placeholder: `EmbeddedPreviewSource` paints a colour
// derived from a seed, so the filmstrip is a wall of synthetic gradients. This is the one that
// shows the photograph.
//
// ## Why the embedded preview, and what it is not
//
// A CR3 contains a large JPEG the camera wrote at capture time. Reading that and decoding it with
// ImageIO is fast, needs no demosaicer, and is what every RAW browser does for fit-to-screen
// viewing. What it is *not* is the sensor data: T3/T4 in task.md §7.1 (100% zoom, "Exact RAW")
// need a real demosaic, and this does not provide it. That is a deliberate v0.1 boundary, not an
// oversight -- a photographer decides what to keep by the frame and the moment, and a 24 MP
// embedded preview carries both. Sharpness comparisons across a burst, which is what zoom-lock is
// for, are only meaningful against consistent pixels; getting them from one source is better than
// getting them from a different source per photo.
//
// `PhotoMeta.preview` gives the byte range the Rust parser already found, so this file never
// searches the file for an SOI marker and never re-parses anything. If a format has no embedded
// preview the image is simply absent, and the viewer says so rather than inventing a placeholder --
// a wrong picture of a photograph is worse than no picture.

import CoreGraphics
import Foundation
import ImageIO

/// Everything needed to show one photograph.
struct PreviewSource: Sendable {
  let url: URL
  /// Where the embedded JPEG starts and how long it is, from `PhotoMeta.preview`.
  let range: ByteRange?
  let pixelWidth: UInt32
  let pixelHeight: UInt32

  init(url: URL, range: ByteRange?, pixelWidth: UInt32, pixelHeight: UInt32) {
    self.url = url
    self.range = range
    self.pixelWidth = pixelWidth
    self.pixelHeight = pixelHeight
  }
}

/// Decodes embedded previews, with a budget and a priority order.
///
/// Three properties, and each is a requirement rather than an optimisation (task.md §7.1):
///
/// * **No lazy loading where the user can reach.** The first request for a photo decodes it and
///   the ones either side of it, because those are what ← and → will show. A cache miss on the
///   next frame is a bug, not a slow path.
/// * **Priority, not FIFO.** The current photo outranks its batch-mates, which outrank the next
///   batch. Navigating re-prioritises immediately, so the frame you land on is the frame that
///  gets decoded next.
/// * **A budget, in bytes.** Decoded bitmaps are large -- a 24 MP frame is ~96 MB -- so the cache
///  evicts the least valuable entries when it is full. Value is recency and priority, never age
///  alone: a photo the user is looking at is not evicted because an old one was touched more
///  recently in a batch they have since left.
///
/// Decoding happens off the main thread and results are delivered on it, so scrolling the filmstrip
/// never waits for a JPEG.
final class PreviewPipeline: @unchecked Sendable {
  /// A decoded image and what it cost. The cost is tracked so the budget is in bytes rather than
  /// in entries -- 20 filmstrip thumbnails and 2 full frames are not the same number of objects.
  private struct Entry {
    let image: CGImage
    let cost: Int
  }

  private let lock = NSLock()
  private var sources: [PhotoID: PreviewSource] = [:]
  private var cache: [PhotoID: Entry] = [:]
  private var order: [PhotoID] = []
  private var cost = 0
  private var budget: Int
  private var inFlight: Set<PhotoID> = []
  private var cancelled: Set<PhotoID> = []
  /// Decodes at `.userInitiated` and at most this many at once (task.md §7.1: decode concurrency
  /// = performance cores). Above that they queue behind each other and the current frame still
  /// gets through first, because priority is the queue's order.
  private let queue: DispatchQueue

  init(budgetBytes: Int = 512 * 1024 * 1024) {
    self.budget = budgetBytes
    self.queue = DispatchQueue(
      label: "com.kathird.firstcut.preview", qos: .userInitiated,
      attributes: .concurrent)
  }

  var budgetBytes: Int {
    get { lock.withLock { budget } }
    set {
      lock.withLock {
        budget = newValue
        evictIfNeeded()
      }
    }
  }

  /// A `CGImage` crossing from the decode queue to the main thread.
  ///
  /// `CGImage` is immutable once created and is only ever read on the main thread after the hand
  /// off, so this is safe; it is spelled out in a type rather than asserted with
  /// `@unchecked Sendable` on a closure that does more than carry a pixel buffer.
  private struct Delivery: @unchecked Sendable {
    let image: CGImage?
  }

  /// Replaces the known photos. Called when a folder is opened; a new session means a new set of
  /// files, and keeping the old ones would show a stale photograph under a new rating.
  func setSources(_ new: [PhotoID: PreviewSource]) {
    lock.withLock {
      sources = new
      // Anything cached belongs to the old folder. Dropping it here rather than lazily is what
      // keeps a reopened folder from showing the previous shoot's first frame.
      cache.removeAll()
      order.removeAll()
      cost = 0
      inFlight.removeAll()
      cancelled.removeAll()
    }
  }

  func source(for id: PhotoID) -> PreviewSource? {
    lock.withLock { sources[id] }
  }

  /// The image, if it is already decoded. Views call this while drawing, so it must never block and
  /// never allocate.
  func cached(_ id: PhotoID) -> CGImage? {
    lock.withLock { touch(id); return cache[id]?.image }
  }

  func cachedCount() -> Int { lock.withLock { cache.count } }
  func cacheCostBytes() -> Int { lock.withLock { cost } }

  /// Asks for an image, and for its neighbours while it is at it.
  ///
  /// `neighbours` is in capture order around `id`, so the caller decides what "next" means -- a
  /// batch's worth of frames, or just the one the arrow key will reach.
  func request(
    _ id: PhotoID, neighbours: [PhotoID] = [], maxPixel: Int,
    completion: @escaping @MainActor (CGImage?) -> Void
  ) {
    let immediate: CGImage? = lock.withLock {
      touch(id)
      return cache[id]?.image
    }
    if let immediate {
      // Even on a hit, keep the neighbourhood warm: it is the same work the user is about to need
      // and doing it now is invisible.
      prefetch(neighbours, maxPixel: maxPixel)
      MainActor.assumeIsolated { completion(immediate) }
      return
    }

    let source = lock.withLock { sources[id] }
    guard source != nil else {
      // No source: either the folder has no embedded preview for this format, or the id is not in
      // this session. Both are "no picture", and saying so beats drawing a placeholder that looks
      // like a photograph.
      MainActor.assumeIsolated { completion(nil) }
      return
    }

    let shouldStart = lock.withLock { () -> Bool in
      guard !inFlight.contains(id) else { return false }
      inFlight.insert(id)
      return true
    }
    guard shouldStart else { return }

    queue.async { [weak self] in
      let delivery = Delivery(image: self?.decodeSynchronously(source!, maxPixel: maxPixel))
      DispatchQueue.main.async {
        guard let self else { return }
        // Cache and deliver, unless the photo was dropped from the session while it was decoding:
        // a photo the user has navigated away from should not be put back on screen afterwards.
        let wanted = self.lock.withLock { () -> Bool in
          self.inFlight.remove(id)
          let stillWanted = self.cancelled.remove(id) == nil
          if let image = delivery.image, stillWanted { self.insert(id, image) }
          return stillWanted
        }
        if wanted {
          // The hop above is onto the main queue, which is the main actor; saying so explicitly
          // is what lets a @MainActor completion be called from here.
          MainActor.assumeIsolated { completion(delivery.image) }
        }
      }
    }

    // The queue is concurrent, so neighbours can be decoded alongside the current photo rather
    // than after it.
    prefetch(neighbours, maxPixel: maxPixel)
  }

  /// Decodes ahead, quietly: no completion, no priority bump. Used to keep ← and → from waiting.
  func prefetch(_ ids: [PhotoID], maxPixel: Int) {
    for id in ids {
      let source = lock.withLock { () -> PreviewSource? in
        guard !cache.keys.contains(id), !inFlight.contains(id),
          let source = sources[id]
        else { return nil }
        inFlight.insert(id)
        return source
      }
      guard let source else { continue }
      queue.async { [weak self] in
        let delivery = Delivery(image: self?.decodeSynchronously(source, maxPixel: maxPixel))
        DispatchQueue.main.async {
          guard let self else { return }
          self.lock.withLock {
            self.inFlight.remove(id)
            if let image = delivery.image { self.insert(id, image) }
          }
        }
      }
    }
  }

  /// Drops anything not in `keep`, and the whole cache when a folder closes.
  func retainOnly(_ keep: Set<PhotoID>) {
    lock.withLock {
      for id in cache.keys where !keep.contains(id) {
        cost -= cache.removeValue(forKey: id)?.cost ?? 0
      }
      order.removeAll { !keep.contains($0) }
      cancelled.formUnion(inFlight.subtracting(keep))
      inFlight.subtract(keep)
    }
  }

  func removeAll() {
    lock.withLock {
      cache.removeAll()
      order.removeAll()
      cost = 0
    }
  }

  // MARK: - Decoding

  /// The actual read-and-decode, for callers that want the pixels now.
  ///
  /// Internal rather than private so a test can assert what a *real* CR3 decodes to without
  /// standing up the async delivery, the cache and a run loop around it. Everything in the app goes
  /// through `request`, which is what a view can call from `draw`.
  func decodeSynchronously(_ source: PreviewSource, maxPixel: Int) -> CGImage? {
    guard let data = readPreview(source) else { return nil }
    return Self.decode(data, maxPixel: maxPixel)
  }

  /// Pulls the embedded JPEG's bytes out of the RAW.
  ///
  /// `mmap` for the file and a slice of the mapping, rather than `read`: a CR3 is 40-60 MB and the
  /// preview is a few MB inside it, so reading the whole file to show one frame is the difference
  /// between a few milliseconds and a few hundred.
  private func readPreview(_ source: PreviewSource) -> Data? {
    guard let range = source.range, range.len > 0 else { return nil }
    // A truncated or lying range would make the read nonsense; refusing it here is better than
    // handing ImageIO a few megabytes of the middle of a RAW and letting it fail obscurely.
    guard
      let attributes = try? FileManager.default.attributesOfItem(atPath: source.url.path),
      let size = (attributes[.size] as? NSNumber)?.int64Value,
      range.offset + range.len <= UInt64(size)
    else { return nil }

    guard let handle = try? FileHandle(forReadingFrom: source.url) else { return nil }
    defer { try? handle.close() }
    do {
      try handle.seek(toOffset: range.offset)
      guard let data = try handle.read(upToCount: Int(range.len)), data.count == Int(range.len)
      else { return nil }
      return data
    } catch {
      return nil
    }
  }

  /// Decodes at the size the screen actually needs.
  ///
  /// `kCGImageSourceThumbnailMaxPixelSize` lets ImageIO's DCT scaler do the downscale
  /// instead of decoding 24 MP and throwing most of it away (task.md §7.2: never nearest/bilinear,
  /// never re-encode). `ShouldCacheImmediately` is not optional: without it ImageIO hands back a
  /// *lazy* image that decodes on first draw, which would put the whole cost back on the main
  /// thread at the moment it is least welcome.
  static func decode(_ data: Data, maxPixel: Int) -> CGImage? {
    let sourceOptions = [kCGImageSourceShouldCache: false] as CFDictionary
    guard let source = CGImageSourceCreateWithData(data as CFData, sourceOptions) else { return nil }

    let options: [CFString: Any] = [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceThumbnailMaxPixelSize: max(1, maxPixel),
      kCGImageSourceShouldCacheImmediately: true,
      // The embedded preview carries the camera's own orientation handling, and honouring it is
      // what makes a portrait frame come out the right way up.
      kCGImageSourceCreateThumbnailWithTransform: true,
    ]
    return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
  }

  // MARK: - Cache bookkeeping (all callers hold the lock)

  private func insert(_ id: PhotoID, _ image: CGImage) {
    if let existing = cache.removeValue(forKey: id) { cost -= existing.cost }
    let bytes = image.bytesPerRow * image.height
    cache[id] = Entry(image: image, cost: bytes)
    cost += bytes
    touch(id)
    evictIfNeeded()
  }

  /// Marks `id` as the most recently wanted. The order list is the eviction order, so this is the
  /// only thing that decides what goes.
  private func touch(_ id: PhotoID) {
    order.removeAll { $0 == id }
    order.append(id)
  }

  private func evictIfNeeded() {
    guard cost > budget else { return }
    // Oldest first. The current photo was just touched, so it is at the end and is the last thing
    // considered -- which is the behaviour that matters when the budget is smaller than one frame.
    var index = 0
    while cost > budget, index < order.count {
      let victim = order[index]
      if let entry = cache.removeValue(forKey: victim) { cost -= entry.cost }
      order.remove(at: index)
      if index > 0 { index -= 1 } else { index = 0 }
    }
    // An entry larger than the whole budget cannot be made to fit; the loop above stops when
    // there is nothing left to drop, and the oversized image stays because showing it beats
    // showing nothing.
  }
}

extension NSLock {
  @inline(__always)
  fileprivate func withLock<T>(_ body: () throws -> T) rethrows -> T {
    lock()
    defer { unlock() }
    return try body()
  }
}
