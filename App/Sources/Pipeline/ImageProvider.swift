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
//   3.0 photos/s on one thread, 7.0 on four, 7.0 on eight), so that is what "auto" resolves to
//   (`resolvedDecodeThreads`) and what Settings → Performance can change.
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
import CoreImage
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
    /// T4 RAW develops. Should stay tiny: one per photograph the user actually looks at with the
    /// setting on, never one per prefetch, because each is 92 MB.
    public var exactRawDecodes: Int = 0
    public var histogramCacheHits: Int = 0
    public var histogramComputes: Int = 0
    public var decodeFailures: Int = 0
    public var decodesInProgress: Int = 0
    public var queuedDecodes: Int = 0
    public var thumbnailBytes: Int = 0
    public var displayBytes: Int = 0
    /// Memory held by T4 RAW develops, which are ~92 MB each and only exist while the setting is
    /// on. The number the debug HUD and the memory tests read to confirm the tier stays small.
    public var exactRawBytes: Int = 0
    /// RAW develops currently running. The one state with no other way to observe it: a develop that
    /// is queued is visible in `queuedDecodes` and one that has landed in `exactRawDecodes`, but a
    /// develop in between cannot be cancelled by a caller who only sees the totals — which is
    /// exactly when the user turns the setting off.
    public var exactRawInFlight: Int = 0
    public var focusSize: Int = 0
    /// Times a system memory-pressure warning made the cache drop everything outside the focus.
    public var pressureSheds: Int = 0
    /// Display bitmaps that were cached but **smaller than the viewer needs**, so a window resize
    /// or a zoom to 100% asked for more and got the old picture back while the bigger decode ran.
    /// Not a focus miss — nothing was decoded on demand by navigating — but the counter that says
    /// "T2 is being decoded at the wrong size" (todo.md §7.1).
    public var displayResizes: Int = 0

    public init() {}
}

// MARK: - Provider

/// Decodes real photos, on demand, off the main thread, with a memory budget.
///
/// Conforms to both consumers the merged tree declares: `ImageProviding` (app-logic's `setFocus`
/// seam, `App/Sources/Session/PipelineMirror.swift`) and `CullImageSource` (ui's `images`,
/// `App/Sources/Views/Model/CullViewState.swift`). One type satisfies both, so the focus the model
/// reports and the pixels the views draw cannot come from two different caches.
/// `@unchecked Sendable` because the shared-thumbnail seam hands it to the detached signature pass:
/// every call that can reach it from another thread (`cachedThumbnail`, `offerThumbnail`) is a single
/// `NSLock`-guarded operation on `engine`, which is itself `@unchecked Sendable` for the same
/// reason. No mutable state on this type is touched off the main actor — `files`, `orientations`,
/// `fullPreviewRanges` and `generation` are main-actor only, and nothing the worker calls reads them.
@MainActor
@Observable
public final class ImageProvider: ImageProviding, CullImageSource, @unchecked Sendable {
    /// `PhotoID` → file URL, filled in once when a folder is opened. The pipeline never walks the
    /// file system itself: core-meta owns identity, this owns pixels.
    private(set) var files: [PhotoID: URL] = [:]

    /// EXIF orientation per photo, for the loupe's full read. 26 of Game1JENKS's 708 frames are
    /// orientation 8, and `CGImageSourceCreateImageAtIndex` does not apply the tag — only the
    /// thumbnail path does. Without this the same photograph appeared upright in the filmstrip and
    /// upside down in the viewer.
    private(set) var orientations: [PhotoID: UInt8] = [:]

    /// Each photo's longest stored edge, from the scan. The decode size is capped at it, because
    /// asking ImageIO for 3456 px of a 1200 px JPEG only wastes the comparison.
    private(set) var longestEdges: [PhotoID: Int] = [:]

    /// The folder the cache belongs to. Decides whether `open` is a new shoot (drop everything) or
    /// the same shoot with files added or removed (keep what is still there).
    private var folder: URL?

    /// Which photographs are RAW, by the core's own reading of the file. This is what gates T4.
    ///
    /// The gate cannot be "`CIRAWFilter` returned nil": measured, `CIRAWFilter(imageURL:)` hands
    /// back a usable filter for a **JPEG** and even for a file of pure junk with a `.cr3`
    /// extension, so a nil check would let T4 "succeed" on formats it has no business touching and
    /// would silently change what a JPEG folder shows. The core already reports `.raw(.cr3)`
    /// versus `.jpeg` from the container, so the honest test is a fact about the file rather than
    /// an accident of how a decoder responded.
    private var rawPhotos: Set<PhotoID> = []

    /// Whether "Exact RAW" is on. Set from every `FocusRequest`, because the setting can change
    /// without the folder or the photo changing.
    private var exactRawEnabled = false

    /// The viewer's backing size, in pixels, as of the last focus report. Zero until the window
    /// reports one, and a view's on-demand ask is answered at the fallback until then.
    private var viewportPixelSize: CGSize = .zero

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

    /// How many display decodes were served from a byte range rather than the container, and how many
    /// had to fall back. The pair is the honest form: 0 fallbacks *and* 0 range decodes would mean the
    /// optimisation is simply not wired up, which looks identical to "nothing decoded yet".
    public private(set) var byteRangeDecodes = 0
    public private(set) var containerDecodes = 0

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

    /// How many decodes may run at once, from Settings → Performance → decode threads.
    ///
    /// `0` (and anything below it) means auto, and auto is **the measured knee, not the core
    /// count**: 4 concurrent decodes are where this machine stopped getting faster (7.0 photos/s on
    /// four threads and on eight, measured over 24 real CR3s), so "as many as there are cores"
    /// would spend more threads than the disk can feed for no gain. It is still bounded by the
    /// performance cores, because a decode that competes with the UI for an efficiency core is a
    /// decode the user feels.
    ///
    /// An explicit count is honoured as given, with a floor of 1: a queue that cannot drain is not a
    /// setting, it is a hang.
    public static func resolvedDecodeThreads(_ configured: Int) -> Int {
        guard configured < 1 else { return configured }
        return max(1, min(4, performanceCoreCount()))
    }

    /// The performance cores, which on Apple Silicon are `hw.perflevel0` — *not*
    /// `activeProcessorCount`, which is every core including the efficiency ones. Falls back to the
    /// total when the sysctl is unavailable (it is not on every machine, and a wrong-but-sane answer
    /// beats none).
    public static func performanceCoreCount() -> Int {
        var count = 0
        var size = MemoryLayout<Int>.size
        if sysctlbyname("hw.perflevel0.logicalcpu", &count, &size, nil, 0) == 0, count > 0 {
            return count
        }
        return ProcessInfo.processInfo.activeProcessorCount
    }

    public init(
        memoryBudgetBytes: Int = Int(Double(ProcessInfo.processInfo.physicalMemory) * 0.40),
        prefetchPixels: Int = 256,
        maxConcurrentDecodes: Int = 0
    ) {
        self.prefetchPixels = max(16, prefetchPixels)
        engine = DecodeEngine(
            memoryBudgetBytes: memoryBudgetBytes,
            maxConcurrent: Self.resolvedDecodeThreads(maxConcurrentDecodes),
            prefetchPixels: self.prefetchPixels,
            defaultDisplayEdge: Self.fallbackDisplayEdge)
        // Assigned rather than passed, because the closure needs `self` and `self` needs the
        // engine. The hop to the main actor happens here, once, so no other file knows about it.
        engine.onLand = { [weak self] id in
            Task { @MainActor in self?.engineDidLand(id) }
        }
        engine.onByteRange = { [weak self] fromRange in
            Task { @MainActor in
                guard let self else { return }
                if fromRange { self.byteRangeDecodes += 1 } else { self.containerDecodes += 1 }
            }
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
        let newLongestEdges = Dictionary(
            photos.map { ($0.id, Int(max($0.width, $0.height))) },
            uniquingKeysWith: { max($0, $1) })
        // `kind` is an enum with an associated value, so it is matched rather than compared.
        let newRawPhotos = Set(
            photos.lazy.filter { if case .raw = $0.kind { return true } else { return false } }.map(\.id))
        engine.beginShoot(
            urls: newFiles,
            orientations: newOrientations,
            sizes: Dictionary(
                photos.map { ($0.id, $0.fileSize) }, uniquingKeysWith: { first, _ in first }),
            sameFolder: self.folder == folder)
        files = newFiles
        orientations = newOrientations
        longestEdges = newLongestEdges
        rawPhotos = newRawPhotos
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
        longestEdges = [:]
        fullPreviewRanges = [:]
        rawPhotos = []
        exactRawEnabled = false
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
        // The last size the window reported, so an on-demand ask from a view (which knows nothing
        // about the focus request) still decodes at the right size.
        if focus.viewportPixelSize != .zero { viewportPixelSize = focus.viewportPixelSize }
        engine.setDefaultDisplayEdge(t2Edge)
        let displayIDs = displayPrefetchIDs(current: focus.currentPhoto, windows: focus.windows)
        engine.setFocus(
            ids: ordered,
            displayIDs: displayIDs,
            displaySizes: Self.displayEdges(
                current: focus.currentPhoto, ids: displayIDs, photoEdges: longestEdges,
                viewport: focus.viewportPixelSize, zoomed: focus.zoomed, zoomLock: focus.zoomLock),
            urls: files, orientations: orientations,
            fullPreviews: Dictionary(
                fullPreviewRanges.map { ($0.key, $0.value.range) },
                uniquingKeysWith: { first, _ in first }))
        updateExactRaw(current: focus.currentPhoto, enabled: focus.exactRaw, rank: ordered)
    }

    /// Keeps the T4 tier in step with the setting and the current photo.
    ///
    /// Deliberately **one photograph**, not the focus window: a RAW develop is 92 MB (§0.5), so a
    /// window of them is not affordable — and "Exact RAW" is a judgement about the frame in front of
    /// you, not about the batch. The *time* is not the constraint; it is about the same as a preview
    /// decode. Turning the setting off drops the tier
    /// immediately rather than letting 92 MB per photograph sit in the cache until eviction.
    private func updateExactRaw(current: PhotoID, enabled: Bool, rank: [PhotoID]) {
        exactRawEnabled = enabled
        guard enabled, rawPhotos.contains(current), let url = files[current] else {
            engine.setExactRaw(id: nil, url: nil)
            return
        }
        engine.setExactRaw(id: current, url: url, priority: rank.firstIndex(of: current) ?? 0)
    }

    /// The longest edge each display bitmap in the focus window should have, in pixels.
    ///
    /// todo.md §7.1's tiers, as a rule rather than as prose: **T2 is the viewer's backing size**,
    /// because that is the number of pixels the layer will actually show and decoding more is
    /// memory spent on detail nobody can see. **T3 is the photograph's own size**, asked only for
    /// the frame being looked at while zoomed — and, with zoom lock, for the frame the user is
    /// about to arrow to, since zoom lock exists so a burst can be compared at 100% and a soft
    /// frame in the middle of it defeats the point. That is two photos at 92 MB, which is the whole
    /// T3 tier; the rest of the window stays at T2.
    ///
    /// `viewport` is in **backing pixels** (the caller multiplies by the display's scale factor), so
    /// there is no scale here and no second place to get it wrong. A zero viewport — the window has
    /// not laid out yet — falls back to `fallbackDisplayEdge`, which is a plausible full-screen
    /// viewer on any of the machines this ships to: too big costs memory for a moment, too small
    /// costs a visible soft first frame and a second decode.
    static func displayEdges(
        current: PhotoID, ids: [PhotoID], photoEdges: [PhotoID: Int], viewport: CGSize,
        zoomed: Bool, zoomLock: Bool
    ) -> [PhotoID: Int] {
        let viewportEdge = max(viewport.width, viewport.height)
        let t2 =
            viewportEdge > 0
            ? max(Self.minimumDisplayEdge, Int(viewportEdge.rounded(.up)))
            : Self.fallbackDisplayEdge
        var full: Set<PhotoID> = []
        if zoomed {
            full.insert(current)
            if zoomLock, let index = ids.firstIndex(of: current), ids.indices.contains(index + 1) {
                full.insert(ids[index + 1])
            }
        }
        var edges: [PhotoID: Int] = [:]
        for id in ids {
            let photoEdge = max(photoEdges[id] ?? t2, Self.minimumDisplayEdge)
            edges[id] = min(max(full.contains(id) ? photoEdge : t2, Self.minimumDisplayEdge), photoEdge)
        }
        return edges
    }

    /// ImageIO's own floor, and the same floor `decodeThumbnail` uses.
    static let minimumDisplayEdge = 64
    /// What T2 is decoded at before the window has reported a size: a 2048 px viewer is a
    /// full-screen fit on every display this ships to, at 10 MB a photo.
    static let fallbackDisplayEdge = 2048

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

    /// The display bitmap, at least `minimumLongestEdge` pixels on its longest side.
    ///
    /// The size is the viewer's, not a guess: a view that knows it is showing the picture at 100%
    /// asks for the photograph's own pixels, and one that is fitting it to the window asks for the
    /// window's. Zero asks for T2, which is the right answer for a view that has not been laid out
    /// yet and the only one available to a caller that does not track zoom.
    ///
    /// With "Exact RAW" on, the current photograph answers from the T4 tier when it is there, and
    /// from T2/T3 until it arrives. Returning the smaller preview meanwhile is the same rule the
    /// resize path already follows (§7.1: a display decode never blanks the viewer), and it matters
    /// more here than for a resize: the RAW is the slowest decode on screen, so blanking would
    /// leave the loupe empty for the length of a develop.
    func displayImage(for id: PhotoID, minimumLongestEdge: Int = 0) -> CGImage? {
        _ = generation
        if exactRawEnabled, rawPhotos.contains(id), let exact = engine.exactRaw(id) {
            return exact
        }
        return engine.display(
            id, minimumLongestEdge: minimumLongestEdge > 0 ? minimumLongestEdge : t2Edge,
            url: files[id], orientation: orientations[id] ?? 1,
            fullPreview: fullPreviewRanges[id]?.range)
    }

    func histogram(for id: PhotoID) -> CullHistogram? {
        _ = generation  // the info panel redraws when the display image it bins arrives
        return engine.histogram(id, url: files[id], fullPreview: fullPreviewRanges[id]?.range)
    }

    /// The edge an on-demand ask is answered at when the caller does not say: the viewer's backing
    /// size, or the fallback before the window has reported one. Split out because the focus path
    /// and the view path must agree — a prefetch at 3000 px and an on-demand ask at 6000 px would
    /// decode the same photograph twice and call one of them a cache miss.
    var t2Edge: Int {
        let edge = max(viewportPixelSize.width, viewportPixelSize.height)
        return edge > 0
            ? max(Self.minimumDisplayEdge, Int(edge.rounded(.up)))
            : Self.fallbackDisplayEdge
    }

    /// What a system memory-pressure warning does; public so a test can trigger it.
    func shedForMemoryPressure() {
        engine.shedToFocus()
        generation &+= 1
    }

    // MARK: - Sharing a decode with the visual-signature pass

    /// A cached thumbnail, or nil. **Never schedules a decode and never blocks**, so the detached
    /// signature pass can call it for every photo in the shoot without touching the main actor.
    nonisolated public func cachedThumbnail(for id: PhotoID) -> CGImage? {
        engine.cachedThumbnail(id)
    }

    /// Take a bitmap the signature pass already decoded, so the filmstrip does not decode it again.
    nonisolated public func offerThumbnail(_ id: PhotoID, image: CGImage, fileSize: UInt64) {
        guard engine.offerThumbnail(id, image: image, fileSize: fileSize) else { return }
        // Same hop as a landed decode: a SwiftUI body that drew a placeholder has to be asked again.
        Task { @MainActor in self.engineDidLand(id) }
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
    /// The cache tiers, as a *kind* rather than a size.
    ///
    /// `exactRaw` is here and not folded into `display` because the two hold **different
    /// pictures**. A `display` entry is the camera's embedded JPEG; an `exactRaw` entry is the
    /// sensor data developed by `CIRAWFilter`. They differ by ~3–52/255 per pixel and the RAW
    /// costs about the same as the preview decode but is not the same picture, so they are not
    /// interchangeable. Since
    /// `Key` is `id + kind`, a separate case is all the cache separation there is: without it a T2
    /// lookup could return RAW pixels for a preview entry, which shows the user a picture the
    /// camera never took rather than merely a slow one.
    enum Kind: Hashable, Sendable {
        case thumbnail
        case display
        /// T4: a true RAW develop of the sensor data, on demand and only for the current photo.
        case exactRaw
    }

    struct Job: Hashable, Sendable {
        var id: PhotoID
        var url: URL
        var kind: Kind
        /// For a thumbnail, the longest edge to decode. For a **display** job, the longest edge the
        /// viewer needs: T2 at the viewer's backing size, T3 at the photograph's own size when
        /// zoomed. Zero means "whatever the view asked for", which only happens for a photo with no
        /// reported size.
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
        /// The longest edge the decode was asked for. A display entry is only good for a viewer
        /// smaller than this; anything larger is a resize, not a hit (todo.md §7.1).
        let pixels: Int
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
    /// The size a display decode is queued at when the caller does not say. Set by the provider from
    /// the viewer's backing size, so the engine and the provider cannot disagree about what T2 is.
    private var defaultDisplayEdge: Int
    /// Set by the provider after construction: called on the decoding thread when a job lands.
    var onLand: (@Sendable (PhotoID) -> Void)?
    /// Called on the decoding thread after a display decode, with whether it was served from the
    /// reported byte range or from the container, so the provider can count both.
    var onByteRange: (@Sendable (Bool) -> Void)?

    private var thumbnails: [PhotoID: Entry] = [:]
    private var displays: [PhotoID: Entry] = [:]
    /// T4. Separate from `displays` on purpose: a RAW develop is a different picture from the
    /// embedded preview, so it must never answer a display request. At most one entry exists at a
    /// time (§7.1: on demand, current photo only), which is what keeps 92 MB a picture affordable —
    /// the time is about the same as a preview decode, so **memory is the whole constraint** here,
    /// not latency — but it is still budgeted and evicted like everything else.
    private var exactRaws: [PhotoID: Entry] = [:]
    private var histograms: [PhotoID: CullHistogram] = [:]
    private var pending: [Job] = []
    private var inFlight: Set<Key> = []
    /// Decodes that already failed. The filmstrip re-asks every 200 ms; without this a single
    /// unreadable file would be re-opened forever.
    private var failed: Set<Key> = []
    /// Work whose result must be thrown away even though it ran to completion: a T4 develop that
    /// was still in flight when the setting was turned off. Distinct from `failed`, which means
    /// "do not retry this", because a cancelled RAW develop *should* be redone if it is asked for
    /// again.
    private var cancelled: Set<Key> = []
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

    init(memoryBudgetBytes: Int, maxConcurrent: Int, prefetchPixels: Int, defaultDisplayEdge: Int) {
        self.maxConcurrent = max(1, maxConcurrent)
        self.budgetBytes = max(1, memoryBudgetBytes)
        self.prefetchPixels = max(16, prefetchPixels)
        self.defaultDisplayEdge = max(16, defaultDisplayEdge)
        watchMemoryPressure()
    }

    /// T2 moved. Called on every focus report, which is every window resize.
    func setDefaultDisplayEdge(_ edge: Int) {
        lock.lock()
        defaultDisplayEdge = max(16, edge)
        lock.unlock()
    }

    deinit {
        pressureSource?.cancel()
    }

    /// Under memory pressure everything outside the focus window goes, at once (task.md §7.1:
    /// shed far batches first, never the current one). The focus window is only the batches the
    /// user can reach, so that is the order the spec asks for.
    private func watchMemoryPressure() {
        let source = DispatchSource.makeMemoryPressureSource(
            eventMask: [.warning, .critical], queue: .global(qos: .utility))
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
        // A RAW develop is 92 MB, the largest single thing in the cache, so a memory warning drops
        // every one that is not the photo on screen. It is cheap to redo on demand, which is the
        // whole reason it is a tier rather than a permanent cache.
        for id in Array(exactRaws.keys) where !focus.contains(id) {
            bytes -= exactRaws.removeValue(forKey: id)?.bytes ?? 0
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
        snapshot.exactRawBytes = exactRaws.values.reduce(0) { $0 + $1.bytes }
        // Per-kind, because `decodesInProgress` cannot tell a 26 ms RAW develop from a thumbnail:
        // a test (or the HUD) that wants "is a develop running right now" needs this, not the total.
        snapshot.exactRawInFlight = inFlight.filter { $0.kind == .exactRaw }.count
        return snapshot
    }

    // MARK: - Focus

    /// The ids that must never be decoded on demand, and the prefetch that makes that true.
    @MainActor
    func setFocus(
        ids: [PhotoID], displayIDs: [PhotoID], displaySizes: [PhotoID: Int] = [:],
        urls: [PhotoID: URL], orientations: [PhotoID: UInt8] = [:],
        fullPreviews: [PhotoID: ByteRange] = [:]
    ) {
        let interval = SignpostInterval.begin(Signposts.setFocus)
        defer { interval.end() }
        let rank = Dictionary(ids.enumerated().map { ($1, $0) }, uniquingKeysWith: min)
        let wanted = Set(displayIDs)
        lock.lock()
        focus = Set(ids)
        counters.focusSize = ids.count
        // Work for photos the user has moved away from is cancelled, and what is left is ranked
        // again from where they are now; otherwise holding the arrow key queues a backlog that
        // the photo they stop on has to wait behind.
        pending.removeAll { job in
            switch job.kind {
            case .display: return !wanted.contains(job.id)
            // A queued RAW develop is dropped as soon as focus moves off its photograph. It is
            // real work for a frame the user is no longer looking at, and the queue is ranked
            // by what they can see. Already-running ones are handled by `clearExactRaws`.
            case .exactRaw: return false
            case .thumbnail: return rank[job.id] == nil
            }
        }
        for index in pending.indices {
            pending[index].priority = rank[pending[index].id] ?? pending[index].priority
        }
        lock.unlock()

        // `countsAsFocusMiss: false` on both: the prefetch *is* the thing that prevents a focus
        // miss, so counting it would make the guarantee impossible to satisfy by construction.
        for id in displayIDs {
            _ = display(
                id, minimumLongestEdge: displaySizes[id] ?? 0, url: urls[id],
                orientation: orientations[id] ?? 1,
                priority: rank[id] ?? 0, fullPreview: fullPreviews[id],
                countsAsFocusMiss: false)
        }
        for (position, id) in ids.enumerated() {
            _ = thumbnail(
                id, minimumLongestEdge: Double(prefetchPixels), slack: 1, url: urls[id],
                priority: position, countsAsFocusMiss: false)
        }
    }

    /// A new shoot: nothing in the cache belongs to it.
    func reset() {
        lock.lock()
        defer { lock.unlock() }
        thumbnails.removeAll()
        displays.removeAll()
        exactRaws.removeAll()
        histograms.removeAll()
        focus.removeAll()
        pending.removeAll()
        inFlight.removeAll()
        failed.removeAll()
        cancelled.removeAll()
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
            for id in Array(exactRaws.keys) where stale(id) {
                bytes -= exactRaws.removeValue(forKey: id)?.bytes ?? 0
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
        exactRaws.removeAll()
        histograms.removeAll()
        focus.removeAll()
        pending.removeAll()
        inFlight.removeAll()
        failed.removeAll()
        cancelled.removeAll()
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
        if var entry = thumbnails[id],
            Self.satisfies(
                entry.image, needed: needed, slack: slack,
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
        let queued =
            url.map {
                enqueueLocked(
                    Job(
                        id: id, url: $0, kind: .thumbnail,
                        maxPixel: max(prefetchPixels, Int(needed.rounded(.up))),
                        priority: priority))
            } ?? false
        lock.unlock()
        if queued { pump() }
        return nil
    }

    /// A display bitmap, at least `needed` pixels on its longest side.
    ///
    /// Three answers, and which one is correct depends on what is in the cache:
    ///
    /// * **big enough** — a hit, returned as is;
    /// * **present but too small** — returned *anyway*, so a window resize or a zoom to 100% never
    ///   blanks the viewer while the bigger decode runs, and the bigger decode is scheduled. This is
    ///   todo.md §7.1's "re-decode on resize, keeping old bitmaps visible until new ones are ready",
    ///   and `displayResizes` is the counter for it. It is deliberately **not** a focus miss: nothing
    ///   was decoded on demand by navigating, the photographer is looking at the same photograph;
    /// * **absent** — a miss, `focusMisses` if it is in the focus window, and nil so the view shows
    ///   the 256 px stand-in rather than nothing.
    @MainActor
    func display(
        _ id: PhotoID, minimumLongestEdge needed: Int = 0, url: URL?, orientation: UInt8 = 1,
        priority: Int = 0, fullPreview: ByteRange? = nil, countsAsFocusMiss: Bool = true
    ) -> CGImage? {
        lock.lock()
        // Zero means "whatever T2 is", which is what the histogram's ask and a view that has not
        // been laid out both mean. Resolved here so the engine and the provider cannot disagree.
        let wanted = max(needed > 0 ? needed : defaultDisplayEdge, 1)
        clock &+= 1
        if var entry = displays[id] {
            let bigEnough = entry.pixels >= wanted || entry.image.longestEdge >= Double(wanted)
            entry.stamp = clock
            displays[id] = entry
            let queued =
                (!bigEnough && url != nil)
                ? enqueueLocked(
                    Job(
                        id: id, url: url!, kind: .display, maxPixel: wanted, priority: priority,
                        orientation: orientation, fullPreview: fullPreview))
                : false
            if bigEnough {
                counters.displayCacheHits += 1
                lock.unlock()
                return entry.image
            }
            counters.displayResizes += 1
            lock.unlock()
            if queued { pump() }
            return entry.image
        }
        if countsAsFocusMiss, focus.contains(id) { counters.focusMisses += 1 }
        let queued =
            url.map {
                enqueueLocked(
                    Job(
                        id: id, url: $0, kind: .display, maxPixel: wanted, priority: priority,
                        orientation: orientation, fullPreview: fullPreview))
            } ?? false
        lock.unlock()
        if queued { pump() }
        return nil
    }

    /// Points the T4 tier at a photograph, or at nothing.
    ///
    /// One call replaces whatever the tier was doing, which is what makes moving to the next frame
    /// cheap: the develop for the photograph just left is dropped rather than left to finish and
    /// land. `nil` means "no T4", so turning the setting off and arrowing onto a JPEG both land
    /// here and need no separate path.
    ///
    /// Idempotent for the same photograph: a second call while one is in flight or already cached
    /// does not queue anything, so `setFocus` firing on every window resize cannot start a develop
    /// per resize.
    @MainActor
    func setExactRaw(id: PhotoID?, url: URL?, priority: Int = 0) {
        lock.lock()
        // Whatever is queued is for a photograph that is no longer the one on screen.
        pending.removeAll { $0.kind == .exactRaw }
        for key in inFlight where key.kind == .exactRaw {
            cancelled.insert(key)
        }
        // Cached entries for other photographs go too: 92 MB each, and the tier is meant to hold the
        // frame in front of the user and nothing else (§7.1).
        let keep = id.map { exactRaws[$0] != nil } ?? false
        if !keep {
            for stale in exactRaws.keys where stale != id {
                bytes -= exactRaws.removeValue(forKey: stale)?.bytes ?? 0
            }
        }
        guard let id, let url else {
            lock.unlock()
            return
        }
        if exactRaws[id] != nil {
            // Touch the stamp so it is the most recently used entry, which keeps the photograph on
            // screen out of the eviction pass.
            exactRaws[id]?.stamp = clock
            lock.unlock()
            return
        }
        let queued = enqueueLocked(
            Job(
                id: id, url: url, kind: .exactRaw,
                // Always the photograph's own size: `scaleFactor` on the filter costs more than it
                // saves (0.622 s at 0.50 against ~0.30 s at 1.0 in the same cold-cache run), so
                // there is no smaller
                // RAW develop to ask for.
                maxPixel: 0, priority: priority))
        lock.unlock()
        if queued { pump() }
    }

    /// The cached RAW develop for `id`, or nil. Never schedules: `setExactRaw` owns that, so this
    /// is a single lock-guarded read a view's `body` can make on every redraw.
    func exactRaw(_ id: PhotoID) -> CGImage? {
        lock.lock()
        defer { lock.unlock() }
        guard var entry = exactRaws[id] else { return nil }
        entry.stamp = clock
        exactRaws[id] = entry
        return entry.image
    }

    /// Drops the whole T4 tier, and any develop queued or in flight for it.
    ///
    /// Cancelling matters as much as clearing: a develop already running cannot be interrupted, so
    /// it is marked cancelled and `store` throws the result away. Clearing `inFlight` alone would
    /// not do, because `store` puts the entry back — the 92 MB would land in the cache after the
    /// reason for it had gone. It is recorded in `cancelled` rather than `failed` so that asking
    /// for the photograph again is allowed to retry.
    @MainActor
    func clearExactRaws() {
        lock.lock()
        for stale in exactRaws.keys {
            bytes -= exactRaws.removeValue(forKey: stale)?.bytes ?? 0
        }
        pending.removeAll { $0.kind == .exactRaw }
        for key in inFlight where key.kind == .exactRaw {
            cancelled.insert(key)
        }
        lock.unlock()
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

    /// The quality of service a decode of `kind` runs at (todo.md §7.1: "decode work runs at
    /// `.userInitiated`, thumbnail generation at `.utility` so it never competes with the current
    /// batch").
    ///
    /// The mechanism is the queue a job is dispatched to, and it is worth being precise about what
    /// that does and does not do: it is a *scheduling* hint to the kernel, not a reservation. What it
    /// buys is that when the machine is busy, a thread decoding the photograph the user is waiting
    /// for outranks a thread filling in filmstrip thumbnails. What it does **not** buy is a decode
    /// slot: `maxConcurrent` counts every kind, so a thumbnail in flight still occupies one of the
    /// four. Giving thumbnails their own budget would mean the focus window could ask for five
    /// displays and find the cap full of filmstrip work — the failure this ranking exists to prevent.
    static func qualityOfService(for kind: Kind) -> DispatchQoS.QoSClass {
        switch kind {
        case .display, .exactRaw: .userInitiated
        case .thumbnail: .utility
        }
    }

    /// Thumbnails are decoded here so they never share a runloop with what the user is waiting for.
    private let thumbnailQueue = DispatchQueue(
        label: "com.kathird.firstcut.decode.thumbnails", qos: .utility, attributes: .concurrent)

    private func drain() {
        while let job = takeNext() {
            // Ranking stays here (one priority order, one cap); only the queue a job runs on differs.
            // A thumbnail is dispatched rather than run inline so the drain loop can keep pulling
            // higher-priority work instead of blocking on a 300 ms filmstrip decode.
            switch job.kind {
            case .thumbnail:
                thumbnailQueue.async { [weak self] in self?.run(job) }
            case .display, .exactRaw:
                run(job)
            }
        }
    }

    /// One decode, from start to the cache. The interval is the decode and nothing else:
    /// computing a pixel count or a signature here would cost real time on every decode whether or
    /// not anything is tracing.
    private func run(_ job: Job) {
        let decoded: CGImage?
        switch job.kind {
        case .thumbnail:
            let interval = SignpostInterval.begin(Signposts.decodeThumbnail)
            decoded = Self.decodeThumbnail(url: job.url, maxPixel: job.maxPixel)
            interval.end()
        case .display:
            decoded = decodeDisplayJob(job)
        case .exactRaw:
            // At `.userInitiated`, like a display decode: the user is looking at this photograph, and
            // it is the one the user is looking at, which is the same reason display decodes get it.
            let interval = SignpostInterval.begin(Signposts.decodeExactRaw)
            decoded = Self.decodeExactRaw(url: job.url)
            interval.end()
        }
        store(job, decoded)
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
            return decodeFromContainer(job)
        }
        let interval = SignpostInterval.begin(Signposts.decodeDisplay)
        guard
            let image = decodeFromBytes(
                range, in: job.url, orientation: job.orientation, maxPixel: job.maxPixel)
        else {
            // The range was there but the bytes were not a JPEG we could read. Fall back rather than
            // show nothing: a stale range is a display bug, an empty viewer is a crash.
            interval.end()
            return decodeFromContainer(job)
        }
        // A nested interval, so a trace can separate the two: the claim in §7.5 is that the range is
        // the cheaper path, and that is only a measurement if the two are distinguishable.
        onByteRange?(true)
        interval.end()
        return image
    }

    /// The display decode from the reported byte range, nested inside `decodeDisplay` so a trace can
    /// tell it apart from the container read and compare the two — which is the whole claim in §7.5.
    private func decodeFromBytes(
        _ range: ByteRange, in url: URL, orientation: UInt8, maxPixel: Int
    ) -> CGImage? {
        let interval = SignpostInterval.begin(Signposts.decodeFromBytes)
        defer { interval.end() }
        return Self.decodeDisplay(
            byteRange: range, in: url, orientation: orientation, maxPixel: maxPixel)
    }

    private func decodeFromContainer(_ job: Job) -> CGImage? {
        let interval = SignpostInterval.begin(Signposts.decodeDisplay)
        defer { interval.end() }
        onByteRange?(false)
        return Self.decodeDisplay(
            url: job.url, orientation: job.orientation, maxPixel: job.maxPixel)
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
        let key = Key(id: job.id, kind: job.kind)
        guard job.epoch == epoch, live.contains(job.id), job.fileSize == knownSizes[job.id] else {
            // Queued for a folder that is no longer open, or for a file that is no longer there;
            // `reset`/`beginShoot` already forgot it was in flight.
            lock.unlock()
            return
        }
        // Cancelled after it started: the work was real, so it is not a failure and not a cache
        // entry either. Dropping it here is what stops a RAW develop from re-inserting 92 MB after
        // the user turned the setting off.
        if cancelled.remove(key) != nil {
            inFlight.remove(key)
            lock.unlock()
            onLand?(job.id)
            return
        }
        inFlight.remove(key)
        if let image {
            // The size the *pixels* have, not the size that was asked for: a file smaller than the
            // request comes back at its own size, and recording the request would make a 1200 px JPEG
            // look like a 3456 px T2 to every later reader.
            let entry = Entry(
                image: image, bytes: image.pixelBytes,
                pixels: max(job.maxPixel, Int(image.longestEdge)), stamp: clock)
            switch job.kind {
            case .thumbnail:
                counters.thumbnailDecodes += 1
                insert(&thumbnails, job.id, entry)
            case .exactRaw:
                // No "must not shrink the cache" dance here: a RAW develop is always the
                // photograph's own size, so there is no smaller version of it to regress to.
                counters.exactRawDecodes += 1
                insert(&exactRaws, job.id, entry)
            case .display:
                // A decode that finished after a bigger one must not shrink the cache: the window
                // grew while this was in flight, and putting the small one back would make every
                // later ask a resize.
                if let existing = displays[job.id], existing.pixels > entry.pixels {
                    break
                }
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
        let interval = SignpostInterval.begin(Signposts.evictions)
        defer { interval.end() }
        var candidates: [(id: PhotoID, stamp: UInt64, bytes: Int)] = []
        for (id, entry) in thumbnails where !focus.contains(id) {
            candidates.append((id, entry.stamp, entry.bytes))
        }
        for (id, entry) in displays where !focus.contains(id) {
            candidates.append((id, entry.stamp, entry.bytes))
        }
        // T4 entries join the same LRU pool. A RAW develop is the most expensive thing here, so it
        // is the first thing worth dropping when the budget is tight — and the focus window is
        // still protected, so the photo on screen keeps its pixels.
        for (id, entry) in exactRaws where !focus.contains(id) {
            candidates.append((id, entry.stamp, entry.bytes))
        }
        for candidate in candidates.sorted(by: { $0.stamp < $1.stamp }) where bytes > budgetBytes {
            if let entry = thumbnails.removeValue(forKey: candidate.id) { bytes -= entry.bytes }
            if let entry = displays.removeValue(forKey: candidate.id) { bytes -= entry.bytes }
            if let entry = exactRaws.removeValue(forKey: candidate.id) { bytes -= entry.bytes }
            histograms.removeValue(forKey: candidate.id)
        }
    }

    // MARK: - Sharing a decode with the visual-signature pass
    //
    // `VisualSigWorker` needs the same 256 px bitmap the filmstrip already has, and used to decode it
    // a second time from the file URL. That is not a small duplicate: a CR3 costs ~300 ms to decode
    // (measured, todo.md §3), so a 2,880-photo shoot was being decoded twice over — once for the
    // focus window and once for the signatures, on the same 4 decode threads, fighting each other for
    // the same disk.
    //
    // These two are deliberately *not* main-actor isolated. The worker runs detached at `.utility`
    // and must not have to hop to the main actor to ask "do you already have this?", because the
    // answer it would get on the main thread is "not yet" for every photo in the shoot.

    /// The cached thumbnail for `id`, or nil. Never schedules anything and never blocks, so it is
    /// safe from any thread: a lock-guarded dictionary read.
    func cachedThumbnail(_ id: PhotoID) -> CGImage? {
        lock.lock()
        defer { lock.unlock() }
        return thumbnails[id]?.image
    }

    /// Publish a bitmap that was decoded elsewhere, so it is not decoded again.
    ///
    /// Refused unless the id is still live **and** the caller is talking about the same file the
    /// cache recorded, so a stale worker cannot overwrite a fresh decode with pixels from a file that
    /// has since been replaced. Returns whether it was stored, which is what the caller's
    /// `generation` bump is keyed on.
    @discardableResult
    func offerThumbnail(_ id: PhotoID, image: CGImage, fileSize: UInt64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard live.contains(id), knownSizes[id] == fileSize else { return false }
        // A decode already in flight would land later and overwrite this with an equally good
        // image, so the only case worth taking is one with nothing pending.
        let key = Key(id: id, kind: .thumbnail)
        guard !inFlight.contains(key) else { return false }
        counters.thumbnailDecodes += 1
        insert(
            &thumbnails, id,
            Entry(
                image: image, bytes: image.pixelBytes, pixels: max(image.width, image.height),
                stamp: clock))
        evictLocked()
        return true
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
        guard
            let raw = CGImageSourceCreateThumbnailAtIndex(
                source, 0,
                [
                    kCGImageSourceCreateThumbnailFromImageAlways: true,
                    kCGImageSourceThumbnailMaxPixelSize: max(16, maxPixel),
                    kCGImageSourceCreateThumbnailWithTransform: true,
                    kCGImageSourceShouldCacheImmediately: true,
                ] as CFDictionary)
        else { return nil }
        return inDisplayLayout(raw)
    }

    /// The display read, at the size the viewer needs.
    ///
    /// ## Why the thumbnail call and not `CreateImageAtIndex`
    ///
    /// Measured on a real CR3 (`RealRawDecodeTests.testDisplayDecodeCosts`, todo.md §7.5), a Canon
    /// R8's embedded preview is 6000 × 4000, and the two ImageIO calls for it are not two spellings
    /// of the same read:
    ///
    /// | call | pixels | decode | + layout fix | memory |
    /// | --- | --- | --- | --- | --- |
    /// | `CreateImageAtIndex` | 6000×4000, **16-bit**, Display P3 | ~850 ms | +850 ms | **183 MB** |
    /// | `CreateThumbnailAtIndex`, 6000 | 6000×4000, 8-bit, sRGB | ~85 ms | +28 ms | 92 MB |
    /// | `CreateThumbnailAtIndex`, 3456 | 3456×2304, 8-bit | ~156 ms | +10 ms | 30 MB |
    /// | `CreateThumbnailAtIndex`, 3000 | 3000×2000, 8-bit | ~140 ms | +7 ms | 23 MB |
    ///
    /// So the old path was not merely "a full decode scaled by the layer": it decoded **16 bits per
    /// component into a P3 space**, which is twice the memory of anything a display needs and costs
    /// most of a second to narrow to 8-bit device RGB. The thumbnail call subsamples in the DCT and
    /// hands back 8 bits, which is 6× faster at the same size and 5× faster *including* the layout
    /// fix at 3456 px.
    ///
    /// `kCGImageSourceCreateThumbnailWithTransform` is deliberately **false**: the orientation is
    /// applied by `applying(orientation:to:)` instead, so the byte-range and container paths stay
    /// byte-for-byte comparable and there is one rotation in the codebase rather than two. The
    /// thumbnail *pass* does set it, because that image is never rotated twice and the filmstrip
    /// needs it right way up.
    static func decodeDisplay(url: URL, orientation: UInt8 = 1, maxPixel: Int) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, sourceOptions) else { return nil }
        guard
            let raw = displayImage(at: source, maxPixel: maxPixel)
        else { return nil }
        return inDisplayLayout(applying(orientation: orientation, to: raw))
    }

    /// The same read, from the JPEG's own bytes instead of from the CR3.
    ///
    /// `CGImageSourceCreateWithData` skips the container entirely: no ISO-BMFF walk, no track
    /// selection, no "which image is the preview" decision — all of which the core already made
    /// during the scan and reported as a byte range. todo.md §7.5 expects this to be the cheap path
    /// for display decodes; the integration suite's byte-range test is what proves it, and
    /// `decodeDisplay(url:…)` stays as the fallback for a file with no reported range.
    ///
    /// The bytes are read with a single `pread`-style `Data(contentsOf:options:)` on an unmapped
    /// file rather than `Data(contentsOf:)`, so a 3 MB range does not go through `mmap` and then get
    /// copied out of it.
    static func decodeDisplay(
        byteRange: ByteRange, in url: URL, orientation: UInt8 = 1, maxPixel: Int
    ) -> CGImage? {
        guard let data = readBytes(byteRange, in: url),
            let source = CGImageSourceCreateWithData(data as CFData, sourceOptions)
        else { return nil }
        guard
            let raw = displayImage(at: source, maxPixel: maxPixel)
        else { return nil }
        return inDisplayLayout(applying(orientation: orientation, to: raw))
    }

    /// The one ImageIO call every display decode makes, whichever source it came from. Split out so
    /// the container path and the byte-range path cannot drift apart in their options — they must
    /// produce identical pixels, and a test asserts it.
    private static func displayImage(at source: CGImageSource, maxPixel: Int) -> CGImage? {
        CGImageSourceCreateThumbnailAtIndex(
            source, 0,
            [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceThumbnailMaxPixelSize: max(16, maxPixel),
                kCGImageSourceCreateThumbnailWithTransform: false,
                kCGImageSourceShouldCacheImmediately: true,
            ] as CFDictionary)
    }

    // MARK: - The layout a layer can be handed without a copy
    //
    // todo.md §7.5's claim for the arrow-key row is "only a pointer swap on the main thread". That
    // is only true if the bitmap is already in the layout Core Animation wants. ImageIO hands back
    // whatever the file's own colour description implies — three components, 16 bits, a YCbCr-backed
    // decode, an embedded ICC — and *any* of those makes Core Animation convert the image to the
    // display's format during the very commit that the key-to-frame interval closes on. So the
    // conversion is hoisted out of the commit and into the decode queue, where it happens on a
    // background thread and is paid once instead of every time the frame is shown.
    //
    // It is not free: one extra buffer and one draw per decode, ~96 MB for a full-resolution CR3
    // today. That is the trade §7.5 asks for, and it is only worth making because the alternative
    // spends the same conversion on the main thread inside the navigation path.

    /// 8 bits per component, 32-bit little-endian words, so the bytes come out B, G, R, X.
    ///
    /// `noneSkipFirst` rather than an alpha channel: a photograph is opaque, and an unused alpha is
    /// another thing for the compositor to look at. An image that *does* carry transparency keeps
    /// it — see `inDisplayLayout(_:)`.
    static let displayBitmapInfo = CGBitmapInfo(
        rawValue: CGImageAlphaInfo.noneSkipFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)

    /// The colour space a cached bitmap has to be in to skip the conversion. `deviceRGB` compares
    /// equal to `CGColorSpace(name: sRGB)`, so both spellings pass.
    private static let deviceRGB = CGColorSpaceCreateDeviceRGB()

    /// Whether `image` can be assigned to `layer.contents` with no conversion at all.
    ///
    /// The colour space has to be the device one, not merely "some RGB space": a JPEG carrying an
    /// embedded ICC profile decodes into a colour space that is RGB but is not the display's, and
    /// Core Animation has to convert *that* anyway. A nil colour space is "unknown", not "fine", so
    /// it fails: an image the decoder cannot describe is exactly the one whose layout you cannot
    /// promise.
    static func isDisplayLayout(_ image: CGImage) -> Bool {
        guard image.bitsPerComponent == 8, image.bitsPerPixel == 32 else { return false }
        guard let space = image.colorSpace, space == deviceRGB else { return false }
        // The byte order is compared through `rawValue & byteOrderMask` rather than the newer
        // `bitmapInfo.byteOrder` accessor: the overlay member does not exist on the macOS 15 SDK,
        // and CI builds against it (Xcode 16.x) while the app is developed on Xcode 27. The mask
        // (`0x7000`) is as old as CGBitmapInfo itself, and `CGImageByteOrderInfo` has no
        // `byteOrder32Little` case of its own, so the comparison is on raw values either way.
        guard
            image.bitmapInfo.rawValue & CGBitmapInfo.byteOrderMask.rawValue
                == CGBitmapInfo.byteOrder32Little.rawValue
        else { return false }
        return image.alphaInfo == .noneSkipFirst || image.alphaInfo == .premultipliedFirst
    }

    /// `image` in the display layout, redrawn only when it is not already there.
    ///
    /// Returns the same object when nothing needs doing, so a JPEG that ImageIO happened to decode
    /// the way we want costs nothing. The redraw is the only place a cached bitmap is ever
    /// resampled, and it is at 1:1 with `interpolationQuality = .none`-equivalent settings, so it
    /// cannot soften a photograph.
    static func inDisplayLayout(_ image: CGImage) -> CGImage {
        guard !isDisplayLayout(image) else { return image }
        // Transparency is preserved rather than dropped: the formats Firstcut opens are opaque, but
        // a PNG in a mixed folder is not, and a black rectangle where a cut-out PNG used to be
        // would be a worse bug than a few milliseconds of extra decode.
        let opaque =
            image.alphaInfo == .none || image.alphaInfo == .noneSkipFirst
            || image.alphaInfo == .noneSkipLast
        let alpha = opaque ? CGImageAlphaInfo.noneSkipFirst : CGImageAlphaInfo.premultipliedFirst
        let info = CGBitmapInfo(
            rawValue: alpha.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)
        guard
            let context = CGContext(
                data: nil, width: image.width, height: image.height, bitsPerComponent: 8,
                bytesPerRow: 0, space: deviceRGB, bitmapInfo: info.rawValue)
        else { return image }
        // `.none`, not `.high`: this is a colour conversion at identical geometry, and an
        // interpolating filter at 1:1 would read every pixel's neighbours for nothing.
        context.interpolationQuality = .none
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return context.makeImage() ?? image
    }

    // MARK: - T4: the one true RAW develop
    //
    // Everything else in this file reads the camera's **embedded JPEG preview**. This is the only
    // place the sensor data is developed, and it exists because Settings → Viewer → "Exact RAW"
    // asks for it by name. Measured on a Canon R8 CR3, alternating the two calls over five rounds
    // so neither is systematically first (`RealRawDecodeTests.testExactRawCostsMoreThanThePreview`):
    //
    // | call | result | cost |
    // | --- | --- | --- |
    // | `CIRAWFilter(imageURL:)` + render | 6000×4000, the sensor data | **~0.079 s**, 92 MB |
    // | the preview decode the app ships | 6000×4000, the camera's JPEG | ~0.09 s |
    //
    // **So a RAW develop is not slower than the preview it replaces** — it is about the same, and
    // what puts T4 out of the prefetch is the **92 MB**, not the time. That is the opposite of what
    // an earlier note here claimed: a first measurement of 0.299 s against the preview's 0.089 s
    // was a **cold file cache**, paid once by whichever call went first. It is recorded here because
    // the wrong number is the more plausible one to act on, and it would have put T4 in the prefetch
    // for the wrong reason.
    //
    // ## Three things that are easy to get wrong, and were
    //
    // **1. `CIRAWFilter(imageURL:)`, not `CIFilter(name: "CIRAWFilter")`.** The name-based route
    // returns an object whose `inputKeys` is **empty**, and setting the obvious
    // `inputImage` key throws `NSUnknownKeyException` — an Objective-C exception Swift cannot
    // catch, so it terminates the process rather than failing a decode. `CIRAWFilter` is public
    // API (`CoreImage/CIRAWFilter.h`, `NS_CLASS_AVAILABLE(12_0, 15_0)`); the class method taking a
    // `URL` is the supported way in.
    //
    // **2. The filter applies the EXIF orientation itself, so this must not.** `CIRAWFilter
    // .orientation` defaults to the file's EXIF tag and the output *geometry* follows it: on one
    // CR3, `.up` yields a 6000×4000 extent and `.left` yields 4000×6000. Every other path in this
    // file therefore rotates via `applying(orientation:to:)`, and doing that here as well would
    // rotate an orientation-8 frame **twice** — the exact bug that function's comment documents
    // ("a portrait frame with the jersey reading LLORRAC"). 26 of Game1JENKS's 708 frames are
    // orientation 8. `RealRawDecodeTests.testExactRawKeepsTheExifOrientation` pins this.
    //
    // **3. Do not ask the filter to scale.** `scaleFactor` is documented as a drawing optimisation,
    // and measured here it costs *more*: 0.50 took 0.622 s and 0.34 took 0.579 s, against 0.299 s at
    // 1.0. (Those three were measured back to back in one process, so the 1.0 figure in that set
    // carries the cold-cache cost described above; the ordering is still the finding — scaling is
    // not a saving.) `isDraftModeEnabled` is no help either at full size (0.291 s vs 0.299 s). So the
    // develop is always full resolution and the layer resamples, which is also the only way the
    /// picture keeps the detail the user turned the setting on for.
    static func decodeExactRaw(url: URL) -> CGImage? {
        guard let filter = CIRAWFilter(imageURL: url), let output = filter.outputImage else {
            return nil
        }
        let context = sharedRawContext
        guard let rendered = context.createCGImage(output, from: output.extent.integral) else {
            return nil
        }
        // Measured layout of `rendered`: 8 bpc, 32 bpp, `deviceRGB`, but `byteOrder32Little` is
        // **not** set (raw value 1, `premultipliedFirst` in the default byte order). So
        // `isDisplayLayout` returns false and this redraw is real work, not a no-op — it is the
        // step that keeps the conversion off the main thread and out of the layer commit (§7.5).
        return inDisplayLayout(rendered)
    }

    /// One context for every T4 develop.
    ///
    /// `CIContext` is expensive to build and documented as safe to use from multiple threads for
    /// rendering, so sharing one is both cheaper and bounded — a per-decode context would hand every
    /// concurrent job its own Metal resources, and the engine runs up to four at once. Created on
    /// first use rather than at init so a session that never turns T4 on never pays for it.
    ///
    /// `nonisolated(unsafe)` because `CIContext` is not marked `Sendable` in the SDK, and under
    /// Swift 6 a `static let` of a non-`Sendable` type is a concurrency error. The property is
    /// `let`, so it is initialised exactly once, and nothing here mutates it — which is the
    /// invariant `nonisolated(unsafe)` asserts. This is the same idiom as
    /// `PhotoViewerHost.storedFactory`. Note this is also one of the §0.4 traps: a `static let` of a
    /// non-`Sendable` type **compiles under Xcode 27 and fails CI**, which builds with Xcode 16.4.
    nonisolated(unsafe) private static let sharedRawContext: CIContext = CIContext(
        options: [.cacheIntermediates: false])

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
            let r = Int(bytes[offset])
            let g = Int(bytes[offset + 1])
            let b = Int(bytes[offset + 2])
            red[r * bins / 256] += 1
            green[g * bins / 256] += 1
            blue[b * bins / 256] += 1
            // Rec. 601 luma, which is what Lightroom's histogram shows. (The J overlay thresholds
            // *per channel*, not on luma — see `ClippingMask`.)
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
