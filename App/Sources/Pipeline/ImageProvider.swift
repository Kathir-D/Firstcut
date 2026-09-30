// Owner: pipeline.
//
// The real `ImageProvider` from [pipeline-api.md], for real files on disk.
//
// ## Why ImageIO and not a RAW decoder
//
// `CGImageSourceCreateThumbnailAtIndex` on a CR3 reads the **embedded JPEG preview** the camera
// wrote into the file, on macOS 15+. There is no TIFF/RAW demosaic in the path, so a 12 MB CR3
// becomes a bitmap without a `libraw`/`dcraw` dependency. Measured on this machine,
// `~/Documents/testing/Game1JENKS/IMG_3181.CR3` (see `ImageProviderTests`):
//
// | call | result | cost |
// | --- | --- | --- |
// | `CreateThumbnailAtIndex` maxPixelSize 256 | 171×256 | ~300 ms |
// | `CreateThumbnailAtIndex` maxPixelSize 1600 | 1067×1600 | ~390 ms |
// | `CreateImageAtIndex` + `kCGImageSourceShouldCacheImmediately` | 6000×4000 | ~130 ms |
//
// **The cost is per file, not per pixel**: asking for 1600 px instead of 256 px is not 6× the
// work, because what costs is reading the file and decoding its full-size preview once — only the
// final downsample scales. Three consequences the design leans on:
//
// * 4 worker threads is the knee of the curve on this 8-core machine (measured over 24 real CR3s:
//   3.0 photos/s on one thread, 7.0 on four, 7.0 on eight), so `maxConcurrent` defaults to 4.
// * Prefetching a *full-resolution* image for a whole batch is not affordable (6000×4000×4 B ≈
//   96 MB each, 12 per batch). Thumbnails are prefetched for the whole focus window; full images
//   only for the current photo and two frames behind and three ahead of it in the current batch.
// * 7 photos/s means a 2 880-photo shoot is ~7 minutes to thumbnail end to end. The UI therefore
//   treats thumbnails as arriving over time (`thumbnailProgress`, the filmstrip's 200 ms retry) and
//   never blocks on one.
//
// ## Threading
//
// `ImageProvider` is main-actor, because the views call it from `body`. Decoding never happens
// there: every call either answers from the cache or *schedules* a job and returns nil, and
// `FilmstripContentView` re-arms `loadThumbnails` every 200 ms while a frame is missing. That is
// the same contract the preview image source had, so no view changed.
//
// `DecodeEngine` is a lock-guarded final class rather than an `actor` on purpose. A `CGImageSource`
// is a C object with a single-owner lifetime and must be created and destroyed on one thread; an
// actor hop per job would cost more in scheduling than the ~300 ms decode it is hiding.
//
// ## focusMisses
//
// [pipeline-api.md] §Guarantees: nothing in the focus window is ever decoded on demand. The
// counter is exact, not estimated: `stats.focusMisses` increments only when a request names a
// photo in the **current focus window** and the cache cannot answer it. The
// `theFocusWindowIsNeverDecodedOnDemand` test pins the property the app relies on — once the
// prefetch has settled, asking for every photo in the window costs zero extra decodes and adds
// zero misses. It deliberately does **not** claim 0 for a whole session: the first decode of a
// freshly opened folder necessarily misses, because nothing is decoded yet, and the counter is what
// makes that visible instead of letting the code pretend otherwise.

import CoreGraphics
import Foundation
import ImageIO
import Observation

// MARK: - Stats

/// `stats` from [pipeline-api.md], plus the counters qa reads (task.md §7.3).
public struct PipelineStats: Equatable, Sendable {
    /// Requests for a photo in the focus window that the cache could not answer. The one number
    /// the contract says "must stay 0".
    public var focusMisses: Int = 0
    public var thumbnailCacheHits: Int = 0
    public var thumbnailCacheMisses: Int = 0
    public var thumbnailDecodes: Int = 0
    public var displayCacheHits: Int = 0
    public var displayDecodes: Int = 0
    public var histogramCacheHits: Int = 0
    public var histogramComputes: Int = 0
    public var decodeFailures: Int = 0
    public var decodesInProgress: Int = 0
    public var queuedDecodes: Int = 0
    public var thumbnailBytes: Int = 0
    public var displayBytes: Int = 0
    public var focusSize: Int = 0
    /// Times a system memory-pressure warning made the cache drop everything outside the focus.
    public var pressureSheds: Int = 0

    public init() {}
}

// MARK: - Provider

/// Decodes real photos, on demand, off the main thread, with a memory budget.
///
/// Conforms to both consumers the merged tree declares: `ImageProviding` (app-logic's `setFocus`
/// seam, `App/Sources/Session/PipelineMirror.swift`) and `CullImageSource` (ui's `images`,
/// `App/Sources/Views/Model/CullViewState.swift`). One type satisfies both, so the focus the model
/// reports and the pixels the views draw cannot come from two different caches.
@MainActor
@Observable
public final class ImageProvider: ImageProviding, CullImageSource {
    /// `PhotoID` → file URL, filled in once when a folder is opened. The pipeline never walks the
    /// file system itself: core-meta owns identity, this owns pixels.
    private(set) var files: [PhotoID: URL] = [:]

    /// EXIF orientation per photo, for the loupe's full read. 26 of Game1JENKS's 708 frames are
    /// orientation 8, and `CGImageSourceCreateImageAtIndex` does not apply the tag — only the
    /// thumbnail path does. Without this the same photograph appeared upright in the filmstrip and
    /// upside down in the viewer.
    private(set) var orientations: [PhotoID: UInt8] = [:]

    /// The folder the cache belongs to. Decides whether `open` is a new shoot (drop everything) or
    /// the same shoot with files added or removed (keep what is still there).
    private var folder: URL?

    /// Where each photo's **full-resolution** JPEG lives, as the file it is in plus the byte range
    /// inside it. Read from the core's `PhotoMeta.fullPreview` (todo.md §7.5, fact 1).
    ///
    /// Display decodes are done from these bytes rather than from the CR3 URL. Handing ImageIO the
    /// container makes it parse the ISO-BMFF, find a track and pick an image — work repeated on
    /// every single decode, for a file where the answer was already computed once during the scan.
    /// `CGImageSourceCreateWithData` on the JPEG's own bytes skips all of that.
    ///
    /// Absent for a file with no `fullPreview` (a JPEG-only shoot, or a format whose parser does not
    /// report one), and the decode then falls back to the URL, so nothing depends on this being
    /// populated.
    private var fullPreviewRanges: [PhotoID: (url: URL, range: ByteRange)] = [:]

    /// How many display decodes have been served from a byte range rather than the container. The
    /// number that shows the optimisation is actually being used rather than merely available.
    public private(set) var byteRangeDecodes = 0

    /// Bumped every time a decode lands. `thumbnail(for:size:)` and `histogram(for:)` read it, so
    /// a SwiftUI body that got a `nil` is asked again when the pixels arrive.
    public private(set) var generation: Int = 0

    /// Fraction of the focus window that has a decoded thumbnail, 0...1. 1.0 when no session is
    /// open, so a progress bar on the welcome screen never reads "0% of nothing".
    public var thumbnailProgress: Double { engine.thumbnailProgress }

    public var stats: PipelineStats { engine.stats }

    private let engine: DecodeEngine

    /// Longest edge of the thumbnails the prefetch decodes: `settings.performance.thumbnailPixels`.
    public let prefetchPixels: Int

    /// A cached thumbnail satisfies a request up to this multiple of the prefetch size. The
    /// filmstrip cell is ~74 pt tall (148 px at 2×) and the layer scales to fit, so re-decoding a
    /// CR3 preview to gain 90 px of sharpness would cost ~300 ms of CPU for no visible gain.
    public let thumbnailSlack: Double = 1.5

    /// `thumbnail(for:size:)` may hand back the **prefetch** bitmap rather than one at exactly
    /// `size`: the view scales it, and re-decoding a CR3 per filmstrip frame to hit an exact pixel
    /// count would make every frame a focus miss. What it never does is return something smaller
    /// than what was asked for.
    public var prefetchPixelsForTests: Int { prefetchPixels }

    public init(
        memoryBudgetBytes: Int = Int(Double(ProcessInfo.processInfo.physicalMemory) * 0.40),
        prefetchPixels: Int = 256,
        maxConcurrentDecodes: Int = 4
    ) {
        self.prefetchPixels = max(16, prefetchPixels)
        engine = DecodeEngine(
            memoryBudgetBytes: memoryBudgetBytes,
            maxConcurrent: max(1, maxConcurrentDecodes),
            prefetchPixels: self.prefetchPixels)
        // Assigned rather than passed, because the closure needs `self` and `self` needs the
        // engine. The hop to the main actor happens here, once, so no other file knows about it.
        engine.onLand = { [weak self] id in
            Task { @MainActor in self?.engineDidLand(id) }
        }
        engine.onByteRange = { [weak self] in
            Task { @MainActor in self?.byteRangeDecodes += 1 }
        }
    }

    /// Points the provider at a shoot. Called once per folder open, and again whenever the watched
    /// folder changes underneath it.
    ///
    /// Two cases, and the difference is the whole point of this method:
    ///
    /// * **A different folder.** Every `PhotoID` in the cache was a hash of the previous folder's
    ///   file names, so the new folder's ids collide with the old ones and a stale entry would
    ///   show the wrong photograph. Everything is dropped and the epoch moves, so a decode still in
    ///   flight for the old shoot is discarded when it lands.
    /// * **The same folder with a different file set.** This is the shoot the user is looking at: a
    ///   second card was copied in, a file was deleted in Finder, a Finish run moved things into
    ///   `_Not kept/`. Only the photographs that are no longer there are dropped, and the rest keep
    ///   their pixels.
    ///
    /// The second case used to reset unconditionally, so dropping a single file into the folder
    /// being culled blanked the filmstrip and re-decoded the whole shoot — 708 CR3s at ~300 ms each
    /// (todo.md §7.1) — while the user was in the middle of rating it. That is the flicker the
    /// folder watcher used to cause.
    public func open(folder: URL, photos: [PhotoMeta]) {
        let newFiles = Dictionary(
            photos.map { ($0.id, folder.appendingPathComponent($0.relPath)) },
            uniquingKeysWith: { first, _ in first })
        let newOrientations = Dictionary(
            photos.map { ($0.id, $0.orientation) },
            uniquingKeysWith: { first, _ in first })
        engine.beginShoot(
            urls: newFiles,
            orientations: newOrientations,
            sizes: Dictionary(
                photos.map { ($0.id, $0.fileSize) }, uniquingKeysWith: { first, _ in first }),
            sameFolder: self.folder == folder)
        files = newFiles
        orientations = newOrientations
        fullPreviewRanges = Dictionary(
            photos.compactMap { meta -> (PhotoID, (url: URL, range: ByteRange))? in
                guard let preview = meta.fullPreview, let url = newFiles[meta.id] else { return nil }
                return (meta.id, (url, preview.range))
            },
            uniquingKeysWith: { first, _ in first })
        self.folder = folder
        generation &+= 1
    }

    public func close() {
        engine.reset()
        files = [:]
        orientations = [:]
        fullPreviewRanges = [:]
        folder = nil
        generation &+= 1
    }

    // MARK: - ImageProviding (app-logic)

    public func setFocus(_ focus: FocusRequest) {
        // Current photo first, then the rest of its batch, then the neighbouring batches. The
        // queue is priority-ordered, so a small window following a large one still lands in the
        // order the user can see.
        var ordered: [PhotoID] = []
        var seen = Set<PhotoID>()
        for id in [focus.currentPhoto] + Self.nearestFirst(focus).flatMap(\.photoIDs)
        where seen.insert(id).inserted {
            ordered.append(id)
        }
        engine.setFocus(
            ids: ordered,
            displayIDs: displayPrefetchIDs(current: focus.currentPhoto, windows: focus.windows),
            urls: files, orientations: orientations,
            fullPreviews: Dictionary(
                fullPreviewRanges.map { ($0.key, $0.value.range) },
                uniquingKeysWith: { first, _ in first }))
    }

    /// The windows ordered the way the user reaches them (task.md §7.1): the current batch, then
    /// the next, then the previous, then further out, next before previous at each distance.
    static func nearestFirst(_ focus: FocusRequest) -> [FocusWindow] {
        guard let home = focus.windows.firstIndex(where: { $0.photoIDs.contains(focus.currentPhoto) })
        else { return focus.windows }
        return focus.windows.indices
            .sorted { left, right in
                let (l, r) = (abs(left - home), abs(right - home))
                return l != r ? l < r : left > right
            }
            .map { focus.windows[$0] }
    }

    /// The current photo, two behind and three ahead of it, **inside its own batch**. Anything wider
    /// and the full-resolution cache is 96 MB per photo, which no sane memory budget holds.
    private func displayPrefetchIDs(current: PhotoID?, windows: [FocusWindow]) -> [PhotoID] {
        guard let current,
            let ids = windows.first(where: { $0.photoIDs.contains(current) })?.photoIDs,
            let position = ids.firstIndex(of: current)
        else { return [] }
        // Three ahead, not two: 4-up Compare shows the current frame and the three after it, and
        // `setFocus` cancels display decodes outside this range on every move.
        let lower = max(0, position - 2)
        let upper = min(ids.count - 1, position + 3)
        guard lower <= upper else { return [] }
        return Array(ids[lower...upper])
    }

    // MARK: - CullImageSource (ui)

    func thumbnail(for id: PhotoID, size: CGSize) -> CGImage? {
        _ = generation  // read so a SwiftUI body re-runs when a decode lands
        return engine.thumbnail(
            id, minimumLongestEdge: max(size.width, size.height), slack: thumbnailSlack,
            url: files[id])
    }

    func displayImage(for id: PhotoID) -> CGImage? {
        _ = generation
        return engine.display(
            id, url: files[id], orientation: orientations[id] ?? 1,
            fullPreview: fullPreviewRanges[id]?.range)
    }

    func histogram(for id: PhotoID) -> CullHistogram? {
        _ = generation  // the info panel redraws when the display image it bins arrives
        return engine.histogram(id, url: files[id], fullPreview: fullPreviewRanges[id]?.range)
    }

    /// What a system memory-pressure warning does; public so a test can trigger it.
    func shedForMemoryPressure() {
        engine.shedToFocus()
        generation &+= 1
    }

    // MARK: - Test hooks

    /// Resolves when nothing is in flight and nothing is queued, or the timeout expires. Returns
    /// whether the engine actually went idle, so a test fails on a timeout instead of hanging.
    @discardableResult
    func waitUntilIdle(timeout: TimeInterval = 60) async -> Bool {
        await engine.waitUntilIdle(timeout: timeout)
    }

    private func engineDidLand(_ id: PhotoID) {
        generation &+= 1
    }
}

// MARK: - Engine

/// The cache, the priority queue and the worker pool. Everything is thread-safe under one lock and
/// nothing in it is main-actor, because a decode that hopped to the main actor would freeze the
/// window for ~300 ms.
final class DecodeEngine: @unchecked Sendable {
    enum Kind: Hashable, Sendable {
        case thumbnail
        case display
    }

    struct Job: Hashable, Sendable {
        var id: PhotoID
        var url: URL
        var kind: Kind
        var maxPixel: Int
        var priority: Int
        /// EXIF orientation, applied on the full read only. Part of the key: the same photo at the
        /// same size is a different job if the orientation changed, which happens when a file is
        /// replaced under a resumed session.
        var orientation: UInt8 = 1
        /// The shoot the job was queued for. A decode that lands after another folder was opened
        /// is dropped: ids are hashes of file names, so `IMG_0001` of the old card would otherwise
        /// be shown as `IMG_0001` of the new one.
        var epoch: UInt64 = 0
        /// The size of the file when the job was queued. Checked on landing, so a file that was
        /// replaced under the same name while the decode ran cannot be served as the old one.
        var fileSize: UInt64 = 0
        /// The full-resolution JPEG's byte range inside `url`, when the core reported one. Nil means
        /// "decode from the container", which is the fallback and the only path for a file with no
        /// reported full preview.
        var fullPreview: ByteRange?
    }

    private struct Key: Hashable {
        let id: PhotoID
        let kind: Kind
    }

    private struct Entry {
        let image: CGImage
        let bytes: Int
        var stamp: UInt64
    }

    private let lock = NSLock()
    /// Concurrent, so `maxConcurrent` decodes really run at once. It was serial, which made the
    /// engine the one-thread case of the measurement above (3.0 photos/s instead of 7.0).
    private let queue = DispatchQueue(
        label: "com.kathird.firstcut.decode", qos: .userInitiated, attributes: .concurrent)
    private var pressureSource: DispatchSourceMemoryPressure?
    private let maxConcurrent: Int
    private let budgetBytes: Int
    let prefetchPixels: Int
    /// Set by the provider after construction: called on the decoding thread when a job lands.
    var onLand: (@Sendable (PhotoID) -> Void)?
    /// Called on the decoding thread when a display decode was served from a byte range rather than
    /// from the container, so the provider can count the ones the optimisation actually served.
    var onByteRange: (@Sendable () -> Void)?

    private var thumbnails: [PhotoID: Entry] = [:]
    private var displays: [PhotoID: Entry] = [:]
    private var histograms: [PhotoID: CullHistogram] = [:]
    private var pending: [Job] = []
    private var inFlight: Set<Key> = []
    /// Decodes that already failed. The filmstrip re-asks every 200 ms; without this a single
    /// unreadable file would be re-opened forever.
    private var failed: Set<Key> = []
    /// Never evicted: the current focus window ("never the current batch", [pipeline-api.md]).
    private var focus: Set<PhotoID> = []
    /// Every id the open folder currently holds, and each one's file size. Together they say
    /// whether a cached entry or an in-flight decode still refers to the file it was made from.
    private var live: Set<PhotoID> = []
    private var knownSizes: [PhotoID: UInt64] = [:]
    private var clock: UInt64 = 0
    private var epoch: UInt64 = 0
    private var bytes = 0
    private var counters = PipelineStats()

    init(memoryBudgetBytes: Int, maxConcurrent: Int, prefetchPixels: Int) {
        self.maxConcurrent = max(1, maxConcurrent)
        self.budgetBytes = max(1, memoryBudgetBytes)
        self.prefetchPixels = max(16, prefetchPixels)
        watchMemoryPressure()
    }

    deinit {
        pressureSource?.cancel()
    }

    /// Under memory pressure everything outside the focus window goes, at once (task.md §7.1:
    /// shed far batches first, never the current one). The focus window is only the batches the
    /// user can reach, so that is the order the spec asks for.
    private func watchMemoryPressure() {
        let source = DispatchSource.makeMemoryPressureSource(eventMask: [.warning, .critical], queue: .global(qos: .utility))
        source.setEventHandler { [weak self] in self?.shedToFocus() }
        source.resume()
        pressureSource = source
    }

    func shedToFocus() {
        lock.lock()
        defer { lock.unlock() }
        for id in Array(thumbnails.keys) where !focus.contains(id) {
            bytes -= thumbnails.removeValue(forKey: id)?.bytes ?? 0
        }
        for id in Array(displays.keys) where !focus.contains(id) {
            bytes -= displays.removeValue(forKey: id)?.bytes ?? 0
        }
        histograms = histograms.filter { focus.contains($0.key) }
        counters.pressureSheds += 1
    }

    // MARK: - Reading

    var thumbnailProgress: Double {
        lock.lock()
        defer { lock.unlock() }
        guard counters.focusSize > 0 else { return 1 }
        let ready = thumbnails.keys.lazy.filter(focus.contains).count
        return min(1, Double(ready) / Double(counters.focusSize))
    }

    var stats: PipelineStats {
        lock.lock()
        defer { lock.unlock() }
        var snapshot = counters
        snapshot.decodesInProgress = inFlight.count
        snapshot.queuedDecodes = pending.count
        snapshot.thumbnailBytes = thumbnails.values.reduce(0) { $0 + $1.bytes }
        snapshot.displayBytes = displays.values.reduce(0) { $0 + $1.bytes }
        return snapshot
    }

    // MARK: - Focus

    /// The ids that must never be decoded on demand, and the prefetch that makes that true.
    @MainActor
    func setFocus(
        ids: [PhotoID], displayIDs: [PhotoID], urls: [PhotoID: URL],
        orientations: [PhotoID: UInt8] = [:],
        fullPreviews: [PhotoID: ByteRange] = [:]
    ) {
        let rank = Dictionary(ids.enumerated().map { ($1, $0) }, uniquingKeysWith: min)
        let wanted = Set(displayIDs)
        lock.lock()
        focus = Set(ids)
        counters.focusSize = ids.count
        // Work for photos the user has moved away from is cancelled, and what is left is ranked
        // again from where they are now; otherwise holding the arrow key queues a backlog that
        // the photo they stop on has to wait behind.
        pending.removeAll { job in
            job.kind == .display ? !wanted.contains(job.id) : rank[job.id] == nil
        }
        for index in pending.indices {
            pending[index].priority = rank[pending[index].id] ?? pending[index].priority
        }
        lock.unlock()

        // `countsAsFocusMiss: false` on both: the prefetch *is* the thing that prevents a focus
        // miss, so counting it would make the guarantee impossible to satisfy by construction.
        for id in displayIDs {
            _ = display(id, url: urls[id], orientation: orientations[id] ?? 1,
                        priority: rank[id] ?? 0, fullPreview: fullPreviews[id],
                        countsAsFocusMiss: false)
        }
        for (position, id) in ids.enumerated() {
            _ = thumbnail(id, minimumLongestEdge: Double(prefetchPixels), slack: 1, url: urls[id],
                           priority: position, countsAsFocusMiss: false)
        }
    }

    /// A new shoot: nothing in the cache belongs to it.
    func reset() {
        lock.lock()
        defer { lock.unlock() }
        thumbnails.removeAll()
        displays.removeAll()
        histograms.removeAll()
        focus.removeAll()
        pending.removeAll()
        inFlight.removeAll()
        failed.removeAll()
        knownSizes.removeAll()
        live.removeAll()
        epoch &+= 1
        bytes = 0
        clock = 0
        counters = PipelineStats()
    }

    /// The provider's `open`, with the knowledge of whether the folder is the same one.
    ///
    /// A different folder resets: the ids collide (they are hashes of file names) and a stale entry
    /// would show the wrong photograph. The *same* folder reconciles — that is a file added or
    /// removed in the shoot the user is looking at, and re-decoding 700 CR3s because one file
    /// landed is the flicker this avoids.
    ///
    /// Reconciliation keeps an entry only when the id is still in the folder **and** the file is
    /// still the same size. The size is what distinguishes "the photograph I already decoded" from
    /// "a different photograph that happens to have the same name" — a re-imported or replaced file.
    func beginShoot(
        urls: [PhotoID: URL], orientations: [PhotoID: UInt8], sizes: [PhotoID: UInt64],
        sameFolder: Bool
    ) {
        lock.lock()
        defer { lock.unlock() }
        // The sizes as they were *before* this call. Comparing against the new ones after assigning
        // them would compare a dictionary with itself and pass everything.
        let previousSizes = knownSizes
        if sameFolder {
            // Drop what is gone, or what is no longer the same file. A new id has no previous size,
            // so it reads as stale — correct, there is no cache entry for it to keep.
            let live = Set(urls.keys)
            func stale(_ id: PhotoID) -> Bool {
                !live.contains(id) || previousSizes[id] != sizes[id]
            }
            for id in Array(thumbnails.keys) where stale(id) {
                bytes -= thumbnails.removeValue(forKey: id)?.bytes ?? 0
            }
            for id in Array(displays.keys) where stale(id) {
                bytes -= displays.removeValue(forKey: id)?.bytes ?? 0
            }
            histograms = histograms.filter { !stale($0.key) }
            failed = failed.filter { !stale($0.id) }
            // Queued work for a photo that is gone is pointless, and a decode in flight for one is
            // dropped when it lands by the same test.
            pending.removeAll { stale($0.id) }
            focus.formIntersection(live)
            counters.focusSize = min(counters.focusSize, live.count)
        } else {
            // Ids are hashes of file names, so the new folder's ids collide with the old ones and a
            // stale entry would show the wrong photograph. Everything goes.
            unlockAndReset()
        }
        // Both paths: what the folder holds now, whatever was kept. Setting this only on the
        // reconcile path left the first `open` with an empty live set, and `store` then discarded
        // every decode as belonging to a photo that was not there.
        live = Set(urls.keys)
        knownSizes = sizes
    }

    /// Must hold the lock. Split out so `beginShoot` and `reset` cannot drift apart.
    private func unlockAndReset() {
        thumbnails.removeAll()
        displays.removeAll()
        histograms.removeAll()
        focus.removeAll()
        pending.removeAll()
        inFlight.removeAll()
        failed.removeAll()
        knownSizes.removeAll()
        live.removeAll()
        epoch &+= 1
        bytes = 0
        clock = 0
        counters = PipelineStats()
    }

    // MARK: - Requests (main actor)

    /// Answers from the cache when it can, otherwise schedules the decode and returns nil. Never
    /// blocks: the caller is a view's `body` and the decode is ~300 ms.
    @MainActor
    func thumbnail(
        _ id: PhotoID, minimumLongestEdge needed: Double, slack: Double, url: URL?, priority: Int = 0,
        countsAsFocusMiss: Bool = true
    ) -> CGImage? {
        lock.lock()
        clock &+= 1
        if var entry = thumbnails[id], Self.satisfies(entry.image, needed: needed, slack: slack,
                                                       prefetch: Double(prefetchPixels))
        {
            entry.stamp = clock
            thumbnails[id] = entry
            counters.thumbnailCacheHits += 1
            lock.unlock()
            return entry.image
        }
        counters.thumbnailCacheMisses += 1
        if countsAsFocusMiss, focus.contains(id) { counters.focusMisses += 1 }
        let queued = url.map { enqueueLocked(Job(id: id, url: $0, kind: .thumbnail,
                                                  maxPixel: max(prefetchPixels, Int(needed.rounded(.up))),
                                                  priority: priority)) } ?? false
        lock.unlock()
        if queued { pump() }
        return nil
    }

    @MainActor
    func display(
        _ id: PhotoID, url: URL?, orientation: UInt8 = 1, priority: Int = 0,
        fullPreview: ByteRange? = nil, countsAsFocusMiss: Bool = true
    ) -> CGImage? {
        lock.lock()
        clock &+= 1
        if var entry = displays[id] {
            entry.stamp = clock
            displays[id] = entry
            counters.displayCacheHits += 1
            lock.unlock()
            return entry.image
        }
        if countsAsFocusMiss, focus.contains(id) { counters.focusMisses += 1 }
        let queued = url.map {
            enqueueLocked(
                Job(id: id, url: $0, kind: .display, maxPixel: 0, priority: priority,
                    orientation: orientation, fullPreview: fullPreview))
        } ?? false
        lock.unlock()
        if queued { pump() }
        return nil
    }

    @MainActor
    func histogram(
        _ id: PhotoID, url: URL?, fullPreview: ByteRange? = nil
    ) -> CullHistogram? {
        lock.lock()
        if let existing = histograms[id] {
            counters.histogramCacheHits += 1
            lock.unlock()
            return existing
        }
        let cached = displays[id]?.image
        lock.unlock()

        guard let cached else {
            // A histogram is computed from pixels, so it needs the display image. Ask for that;
            // the view asks again once `generation` moves.
            if let url { _ = display(id, url: url, fullPreview: fullPreview) }
            return nil
        }
        // 256×256 is 65 k pixels, so this is sub-millisecond and does not need the decode queue.
        let computed = Self.histogram(of: cached)
        lock.lock()
        histograms[id] = computed
        counters.histogramComputes += 1
        lock.unlock()
        return computed
    }

    // MARK: - Queue

    /// Whether a cached bitmap is good enough for a request.
    ///
    /// Two ways to say yes, and the second one is why `focusMisses` can reach zero at all:
    ///  * it is at least as big as asked for; or
    ///  * it *is* the prefetch image and the ask is within `slack` of it. The filmstrip cell is
    ///    ~74 pt (148 px at 2×) and the prefetch is 256 px, so a filmstrip ask of 148 px must not
    ///    re-decode a CR3 preview to gain sharpness nobody can see — and if it did, every frame
    ///    would be a focus miss and the guarantee would be false by construction.
    /// Anything beyond that is a genuine miss and is re-decoded.
    private static func satisfies(
        _ image: CGImage, needed: Double, slack: Double, prefetch: Double
    ) -> Bool {
        let longest = image.longestEdge
        if longest >= needed { return true }
        return longest + 0.5 >= prefetch && needed <= prefetch * slack
    }

    /// Must hold the lock. False when the job is already queued or in flight, so a view that
    /// re-asks every 200 ms cannot flood the queue with duplicates.
    private func enqueueLocked(_ job: Job) -> Bool {
        let key = Key(id: job.id, kind: job.kind)
        guard !failed.contains(key), !inFlight.contains(key),
            !pending.contains(where: { Key(id: $0.id, kind: $0.kind) == key })
        else { return false }
        var job = job
        job.epoch = epoch
        job.fileSize = knownSizes[job.id] ?? 0
        pending.append(job)
        return true
    }

    private func pump() {
        queue.async { [weak self] in self?.drain() }
    }

    private func drain() {
        while let job = takeNext() {
            let decoded: CGImage? =
                switch job.kind {
                case .thumbnail: Self.decodeThumbnail(url: job.url, maxPixel: job.maxPixel)
                case .display: decodeDisplayJob(job)
                }
            store(job, decoded)
        }
    }

    /// A display decode, preferring the reported byte range over the container.
    ///
    /// Split out of `drain` so the fallbacks read as fallbacks: three ways to end up here and only
    /// the middle one is the optimisation. Kept returning rather than branching inline because an
    /// inline `if/else` expression is where a `return` would silently return from `drain` and
    /// abandon the rest of the queue.
    private func decodeDisplayJob(_ job: Job) -> CGImage? {
        guard let range = job.fullPreview else {
            // Nothing reported, so the container is the only authority.
            return Self.decodeFull(url: job.url, orientation: job.orientation)
        }
        guard
            let image = Self.decodeFull(
                byteRange: range, in: job.url, orientation: job.orientation)
        else {
            // The range was there but the bytes were not a JPEG we could read. Fall back rather than
            // show nothing: a stale range is a display bug, an empty viewer is a crash.
            return Self.decodeFull(url: job.url, orientation: job.orientation)
        }
        onByteRange?()
        return image
    }

    private func takeNext() -> Job? {
        lock.lock()
        defer { lock.unlock() }
        guard inFlight.count < maxConcurrent, !pending.isEmpty else { return nil }
        let best = pending.enumerated().min { left, right in
            left.element.priority == right.element.priority
                ? left.offset < right.offset
                : left.element.priority < right.element.priority
        }
        guard let best else { return nil }
        let job = pending.remove(at: best.offset)
        inFlight.insert(Key(id: job.id, kind: job.kind))
        return job
    }

    private func store(_ job: Job, _ image: CGImage?) {
        lock.lock()
        // `epoch` moves when a *different* folder is opened; `live` is what the folder holds now.
        // Both must still hold for the pixels to belong anywhere: a decode of a photo that was
        // deleted or replaced while it was running must not be filed under the id it had then.
        guard job.epoch == epoch, live.contains(job.id), job.fileSize == knownSizes[job.id] else {
            // Queued for a folder that is no longer open, or for a file that is no longer there;
            // `reset`/`beginShoot` already forgot it was in flight.
            lock.unlock()
            return
        }
        inFlight.remove(Key(id: job.id, kind: job.kind))
        if let image {
            let entry = Entry(image: image, bytes: image.pixelBytes, stamp: clock)
            switch job.kind {
            case .thumbnail:
                counters.thumbnailDecodes += 1
                insert(&thumbnails, job.id, entry)
            case .display:
                counters.displayDecodes += 1
                insert(&displays, job.id, entry)
            }
            evictLocked()
        } else {
            counters.decodeFailures += 1
            failed.insert(Key(id: job.id, kind: job.kind))
        }
        let moreWork = !pending.isEmpty
        lock.unlock()

        onLand?(job.id)
        // A worker that finishes frees a slot, and the jobs it was blocking are still queued.
        if moreWork { pump() }
    }

    private func insert(_ store: inout [PhotoID: Entry], _ id: PhotoID, _ entry: Entry) {
        bytes += entry.bytes - (store.removeValue(forKey: id)?.bytes ?? 0)
        store[id] = entry
    }

    /// Least-recently-used eviction down to the budget, never touching the focus window.
    private func evictLocked() {
        guard bytes > budgetBytes else { return }
        var candidates: [(id: PhotoID, stamp: UInt64, bytes: Int)] = []
        for (id, entry) in thumbnails where !focus.contains(id) {
            candidates.append((id, entry.stamp, entry.bytes))
        }
        for (id, entry) in displays where !focus.contains(id) {
            candidates.append((id, entry.stamp, entry.bytes))
        }
        for candidate in candidates.sorted(by: { $0.stamp < $1.stamp }) where bytes > budgetBytes {
            if let entry = thumbnails.removeValue(forKey: candidate.id) { bytes -= entry.bytes }
            if let entry = displays.removeValue(forKey: candidate.id) { bytes -= entry.bytes }
            histograms.removeValue(forKey: candidate.id)
        }
    }

    // MARK: - Waiting (tests)

    func waitUntilIdle(timeout: TimeInterval) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if !isBusy() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return !isBusy()
    }

    /// `NSLock` is unavailable from an async context, so the busy check is a plain synchronous
    /// function the `await` loop calls.
    private func isBusy() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return !pending.isEmpty || !inFlight.isEmpty
    }

    // MARK: - ImageIO

    /// The embedded-preview read. `kCGImageSourceCreateThumbnailFromImageAlways` plus
    /// `kCGImageSourceThumbnailMaxPixelSize` is the pair that makes ImageIO subsample *during*
    /// decode instead of decoding 24 MP and discarding most of it.
    static func decodeThumbnail(url: URL, maxPixel: Int) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, sourceOptions) else { return nil }
        return CGImageSourceCreateThumbnailAtIndex(
            source, 0, [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceThumbnailMaxPixelSize: max(16, maxPixel),
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceShouldCacheImmediately: true,
            ] as CFDictionary)
    }

    /// The full-resolution read for the loupe. This is the *camera's* preview, not a RAW decode: a
    /// Canon R8 CR3's embedded JPEG is 6000×4000, i.e. the sensor's full size, so "100%" is
    /// honest. A viewport-sized `decodeThumbnail` would be far cheaper to cache for fit-to-viewport
    /// and is a one-line change if the 96 MB per photo ever matters more than the decode time.
    ///
    /// `kCGImageSourceShouldCacheImmediately` alone does **not** apply the EXIF orientation, and
    /// 26 of Game1JENKS's 708 frames are orientation 8 (rotate 270°) — measured, not assumed. The
    /// loupe rendered those upside down while the filmstrip, which goes through
    /// `decodeThumbnail` with `kCGImageSourceCreateThumbnailWithTransform`, showed them right way
    /// up, so the same photo appeared twice in two orientations. `CGImageSourceCreateImageAtIndex`
    /// has no transform option, so the rotation is applied here from the parsed orientation.
    static func decodeFull(url: URL, orientation: UInt8 = 1) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, sourceOptions) else { return nil }
        guard
            let raw = CGImageSourceCreateImageAtIndex(
                source, 0, [kCGImageSourceShouldCacheImmediately: true] as CFDictionary)
        else { return nil }
        return applying(orientation: orientation, to: raw)
    }

    /// The same read, from the JPEG's own bytes instead of from the CR3.
    ///
    /// `CGImageSourceCreateWithData` skips the container entirely: no ISO-BMFF walk, no track
    /// selection, no "which image is the preview" decision — all of which the core already made
    /// during the scan and reported as a byte range. todo.md §7.5 expects this to be the cheap path
    /// for display decodes; `realPhotoDecodeCosts` in the integration suite is what proves it, and
    /// `decodeFull(url:)` stays as the fallback for a file with no reported range.
    ///
    /// The bytes are read with a single `pread`-style `Data(contentsOf:options:)` on an unmapped
    /// file rather than `Data(contentsOf:)`, so a 3 MB range does not go through `mmap` and then get
    /// copied out of it.
    static func decodeFull(byteRange: ByteRange, in url: URL, orientation: UInt8 = 1) -> CGImage? {
        guard let data = readBytes(byteRange, in: url),
            let source = CGImageSourceCreateWithData(data as CFData, sourceOptions)
        else { return nil }
        guard
            let raw = CGImageSourceCreateImageAtIndex(
                source, 0, [kCGImageSourceShouldCacheImmediately: true] as CFDictionary)
        else { return nil }
        return applying(orientation: orientation, to: raw)
    }

    /// Just the bytes of a range, or nil. A `FileHandle`-free, allocation-honest read: bounds-checked
    /// against the file's real length so a stale range from a truncated file cannot read past the
    /// end, and a short read is a failure rather than a short image.
    static func readBytes(_ range: ByteRange, in url: URL) -> Data? {
        guard let handle = try? FileHandle(forReadingFrom: url) else { return nil }
        defer { try? handle.close() }
        let size = (try? handle.seekToEnd()) ?? 0
        guard range.offset < size else { return nil }
        let length = Int(min(UInt64(range.len), size - range.offset))
        guard length > 0 else { return nil }
        do {
            try handle.seek(toOffset: range.offset)
            return try handle.read(upToCount: length) ?? nil
        } catch {
            return nil
        }
    }

    /// EXIF orientation 1–8 → the pixels, upright.
    ///
    /// The tag is defined so 1 is upright and 8 is "rotate 270° CW to display", which for a
    /// landscape frame is a half turn. Only the transforms that change the pixels are applied; 1 is
    /// the identity. The mirror cases (2, 4) are handled too rather than left wrong, even though a
    /// camera does not produce them for a back-of-camera shot.
    ///
    /// There is no image-level affine in Core Graphics, so this composites through a context sized
    /// to the *output* — which is what makes a quarter turn come out the right way up instead of
    /// cropped to a corner.
    static func applying(orientation: UInt8, to image: CGImage) -> CGImage {
        guard orientation > 1, let space = CGColorSpace(name: CGColorSpace.sRGB) else { return image }
        let width = image.width
        let height = image.height

        // (rotation, mirror) per EXIF orientation, expressed as what to draw where.
        let quarterTurn: Int
        let mirrored: Bool
        switch orientation {
        case 2: (quarterTurn, mirrored) = (0, true)
        case 3: (quarterTurn, mirrored) = (2, false)
        case 4: (quarterTurn, mirrored) = (0, false)
        case 5: (quarterTurn, mirrored) = (3, true)
        case 6: (quarterTurn, mirrored) = (1, false)
        case 7: (quarterTurn, mirrored) = (3, false)
        // 8 is a quarter turn, **not** a mirror. Treating it as "rotate 90 then mirror" produced a
        // portrait frame with the jersey reading "LLORRAC" — verified by rendering IMG_3181.CR3 and
        // looking at it, which is the only way to catch a transform that is the right shape and the
        // wrong transform. 26 of this game's 708 frames are orientation 8, so it is not a corner.
        case 8: (quarterTurn, mirrored) = (1, false)
        default: return image
        }

        let turns = quarterTurn % 2 == 1
        let outWidth = turns ? height : width
        let outHeight = turns ? width : height
        guard
            let context = CGContext(
                data: nil, width: outWidth, height: outHeight, bitsPerComponent: 8, bytesPerRow: 0,
                space: space,
                bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue
                    | CGBitmapInfo.byteOrder32Little.rawValue)
        else { return image }

        context.translateBy(x: CGFloat(outWidth) / 2, y: CGFloat(outHeight) / 2)
        // Rotate first, then mirror in the *output* frame. Mirroring before rotating mirrors about
        // the source axes, which is a different image.
        context.rotate(by: CGFloat(quarterTurn) * .pi / 2)
        if mirrored { context.scaleBy(x: -1, y: 1) }
        context.translateBy(x: -CGFloat(width) / 2, y: -CGFloat(height) / 2)
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        return context.makeImage() ?? image
    }

    /// No source-level cache: the engine owns one cache, and a second invisible one inside ImageIO
    /// would make the memory budget a lie.
    private static var sourceOptions: CFDictionary {
        [kCGImageSourceShouldCache: false] as CFDictionary
    }

    /// 64 bins per channel off a 256-pixel reduction. Small enough to compute inline and to cache
    /// for the life of the session.
    static func histogram(of image: CGImage) -> CullHistogram {
        let bins = 64
        let side = 256
        let empty = CullHistogram(
            red: [Double](repeating: 0, count: bins), green: [Double](repeating: 0, count: bins),
            blue: [Double](repeating: 0, count: bins), luminance: [Double](repeating: 0, count: bins))

        let scale = Double(side) / Double(max(image.width, image.height))
        let width = max(1, Int((Double(image.width) * scale).rounded()))
        let height = max(1, Int((Double(image.height) * scale).rounded()))
        guard
            let context = CGContext(
                data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return empty }
        context.interpolationQuality = .low
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        guard let data = context.data else { return empty }

        let bytes = data.bindMemory(to: UInt8.self, capacity: width * height * 4)
        let total = Double(width * height)
        var red = [Double](repeating: 0, count: bins)
        var green = [Double](repeating: 0, count: bins)
        var blue = [Double](repeating: 0, count: bins)
        var luminance = [Double](repeating: 0, count: bins)
        for index in 0..<(width * height) {
            let offset = index * 4
            let r = Int(bytes[offset]), g = Int(bytes[offset + 1]), b = Int(bytes[offset + 2])
            red[r * bins / 256] += 1
            green[g * bins / 256] += 1
            blue[b * bins / 256] += 1
            // Rec. 601 luma, which is what the J clipping overlay thresholds against (§9.2).
            let luma = (0.299 * Double(r) + 0.587 * Double(g) + 0.114 * Double(b)).rounded()
            luminance[min(bins - 1, Int(luma) * bins / 256)] += 1
        }
        return CullHistogram(
            red: red.map { $0 / total }, green: green.map { $0 / total },
            blue: blue.map { $0 / total }, luminance: luminance.map { $0 / total })
    }
}

// MARK: - Small extensions

extension CGImage {
    fileprivate var longestEdge: Double { Double(max(width, height)) }

    /// What a decoded bitmap actually costs. 4 bytes per pixel is the worst case and the honest
    /// one; counting compressed bytes would make the budget a lie.
    fileprivate var pixelBytes: Int { width * height * 4 }
}
