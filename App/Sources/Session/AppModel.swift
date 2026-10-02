// Owner: app-logic.
//
// The app's brain: observable state, every command, the rating rules, navigation, undo bridging,
// focus reporting and the Finish flow. [app-model.md] is the contract for this file.
//
// Everything the user can do arrives here as a `Command` and leaves here as state plus a
// `FocusRequest` for the pipeline. There is no other way in, which is why the entire app — every
// command, both rating modes, a whole cull — is testable with no window and no image decoding.

import CoreGraphics
import Foundation
import Observation

@MainActor @Observable
public final class AppModel: SessionListener, KeyRouterSource {
    // MARK: Session state

    public private(set) var phase: Phase = .welcome
    public private(set) var folderName: String = ""
    public private(set) var folderURL: URL?
    /// Every photo in capture order. Batches are contiguous ranges of this array.
    public private(set) var allPhotos: [PhotoVM] = []
    public private(set) var batches: [BatchVM] = []
    public private(set) var currentBatchIndex: Int = 0
    public private(set) var currentPhotoIndex: Int = 0
    /// Files that couldn't be parsed (task.md §8: shown with a placeholder, never blocking).
    public private(set) var skippedFiles: [SkippedFile] = []
    /// Something the user needs to know went wrong; the window shows it as an alert until
    /// `dismissError()`.
    public private(set) var lastError: String?
    /// A read-only card fails every sidecar write; one alert per folder says so, not one per rating.
    private var reportedXMPFailure = false

    // MARK: Modes and toggles

    public private(set) var viewMode: ViewMode = .loupe
    public private(set) var infoPanelVisible: Bool = false
    public private(set) var hudVisible: Bool = true
    /// Settings → Performance → Debug HUD. Read by the view, which pulls the counters off the
    /// pipeline; the model only carries the flag and hands over the numbers.
    public var debugHUDVisible: Bool { settings.performance.debugHUD }
    public private(set) var afOverlay: Bool = false
    public private(set) var clippingOverlay: Bool = false
    public private(set) var viewer = ViewerState()
    public private(set) var finish: FinishStage = .hidden

    /// Caps Lock flips this for the session only; Settings → General owns the persistent value
    /// (Lightroom behaves the same way, and task.md §6.3 calls it "a setting, toggled also with
    /// Caps Lock").
    public private(set) var autoAdvanceOverride: Bool?

    public var settings: AppSettings
    public private(set) var keymap: Keymap
    public let images: any ImageProviding

    /// The concrete provider, for the composition root. `images` is existential because the tests
    /// and previews pass a mock; this is the real one, and the viewer has to share it rather than
    /// build a second — see `AppEnvironment.init`.
    public var imageProvider: ImageProvider? { images as? ImageProvider }

    /// ui owns these two; the model asks, it doesn't do.
    /// Folders opened before, newest first, for the Welcome window (task.md §9.6).
    public private(set) var recents: [RecentFolder] = []

    public var onRequestOpenFolder: (() -> Void)?
    public var onRequestToggleFullScreen: ((Bool) -> Void)?

    // MARK: Private

    private let backendBox = SessionBox()
    private let sessionFactory: (URL) throws -> any SessionBackend
    private let asyncSessionFactory: (@MainActor (URL) async throws -> any SessionBackend)?
    /// One photograph from a folder, for the first-photo fast path. nil where there are no real
    /// files to read; see `Dependencies.firstPhoto`.
    private let firstPhoto: (@Sendable (URL) -> PhotoMeta?)?
    private var openTask: Task<Void, Never>?
    /// todo.md §7.3's "folder open → first photo on screen < 1 s": the fast path's read of one
    /// header, cancelled when the open it belongs to is superseded.
    private var fastPathTask: Task<Void, Never>?
    /// True while the one photograph on screen came from the fast path rather than from a session.
    ///
    /// The provisional frame is *display-only*, and this flag is what makes that structural rather
    /// than a matter of remembering: a rating, an undo or a Finish here would be written through the
    /// **previous** session's backend, because the session for this folder does not exist yet. So
    /// every command that mutates refuses while it is set, and the toolbar says the folder is still
    /// being read.
    private var isProvisional = false
    private let settingsStore: SettingsStore
    private let recentsStore: RecentFoldersStore
    private var keymapStore: KeymapStore
    private var photoIndex: [PhotoID: Int] = [:]
    private var batchIndexByID: [BatchID: Int] = [:]
    private var countsByTier: [Tier: Int] = [:]
    private var ratedCount: Int = 0
    private var saveTask: Task<Void, Never>?
    private var folderWatcher: FolderWatcher?
    private let visualSigWorker = VisualSigWorker()
    public private(set) var viewportPixelSize: CGSize = .zero
    private var isTextEditingFlag: Bool = false

    /// The live session. A stored `let` can't be swapped when a second folder is opened, and opening
    /// one must not leave the previous session's listener attached.
    ///
    /// Internal rather than private so `AppEnvironment` can hand a mock model's own backend to its
    /// own `open(_:folderName:)`. The write path stays in this file: swapping happens through
    /// `open(_:folderName:)`, which detaches the old listener first.
    private var storage: SessionBox { backendBox }
    var backend: any SessionBackend { storage.session }

    // MARK: Init

    public init(_ dependencies: Dependencies) {
        if let session = dependencies.backend {
            backendBox.session = session
        }
        images = dependencies.images
        sessionFactory = dependencies.sessionFactory
        asyncSessionFactory = dependencies.asyncSessionFactory
        firstPhoto = dependencies.firstPhoto
        settings = dependencies.settings
        settingsStore = dependencies.settingsStore
        recentsStore = RecentFoldersStore(directory: dependencies.settingsStore.directory)
        recents = recentsStore.load()
        keymapStore = dependencies.keymapStore
        keymap = dependencies.keymap
        hudVisible = dependencies.settings.viewer.hudVisible
        afOverlay = dependencies.settings.viewer.afOverlay
        viewer.zoomLock = dependencies.settings.viewer.zoomLock
        viewer.backgroundGray = dependencies.settings.viewer.backgroundGray
        backend.listener = self
    }

    /// A model wired to the exiftool fixtures (or a synthetic shoot), with persistence pointed at a
    /// scratch directory so a preview can't disturb the real settings or keymap. This is what ui
    /// builds every screen against before the core and the pipeline exist.
    public static func preview(game: String? = "Game1JENKS", photoLimit: Int? = nil) -> AppModel {
        AppModel(.preview(game: game, photoLimit: photoLimit))
    }

    // MARK: - Derived state

    public var folderDisplayName: String { folderName }

    public var ratingMode: RatingMode { settings.general.ratingMode }

    public var autoAdvance: Bool { autoAdvanceOverride ?? settings.general.autoAdvance }

    public var isTextEditing: Bool {
        get { isTextEditingFlag }
        set { isTextEditingFlag = newValue }
    }

    public var currentBatch: BatchVM? {
        batches.indices.contains(currentBatchIndex) ? batches[currentBatchIndex] : nil
    }

    /// The selected photo, always inside the current batch (task.md §6.3).
    public var currentPhoto: PhotoVM? {
        absolutePhotoIndex.flatMap { allPhotos.indices.contains($0) ? allPhotos[$0] : nil }
    }

    /// `currentPhotoIndex` is *within* the current batch (app-model.md); this is where that lands
    /// in the flat, capture-ordered photo list.
    public var absolutePhotoIndex: Int? {
        guard let batch = currentBatch else { return nil }
        let index = batch.range.lowerBound + currentPhotoIndex
        return batch.range.contains(index) ? index : nil
    }

    public func photos(inBatch index: Int) -> [PhotoVM] {
        guard batches.indices.contains(index) else { return [] }
        let range = batches[index].range
        guard allPhotos.indices.contains(range) else { return [] }
        return Array(allPhotos[range])
    }

    public func batchIndex(for id: BatchID) -> Int? { batchIndexByID[id] }

    public func photoIndex(of id: PhotoID) -> Int? { photoIndex[id] }

    public func isVisited(_ batch: Int) -> Bool {
        batches.indices.contains(batch) && batches[batch].visited
    }

    public var progress: CullProgress {
        guard let batch = currentBatch else { return CullProgress() }
        return CullProgress(
            batchNumber: currentBatchIndex + 1,
            batchCount: batches.count,
            photoNumber: min(currentPhotoIndex + 1, batch.count),
            photoCount: batch.count,
            photosLeftInBatch: max(0, batch.count - currentPhotoIndex - 1),
            batchesLeft: max(0, batches.count - currentBatchIndex - 1),
            unvisitedBatches: batches.count(where: { !$0.visited }),
            counts: countsByTier,
            totalPhotos: allPhotos.count,
            ratedPhotos: ratedCount)
    }

    public var summary: FinishSummary {
        FinishSummary(
            counts: countsByTier,
            unvisitedBatches: batches.count(where: { !$0.visited }),
            batchCount: batches.count,
            totalPhotos: allPhotos.count)
    }

    /// Convenience for menus and the HUD.
    public func shortcut(for command: Command) -> String? {
        keymap.primaryChord(for: command, mode: ratingMode)?.description
    }

    // MARK: - Opening a session

    public func open(folder url: URL) {
        // The shipped app opens off the main thread, so the window stays alive and the loading
        // screen is drawn while the core reads the folder. Without an async factory (tests,
        // previews) it is the synchronous path below. A shoot already open is saved first: its
        // pending sidecars and its place in the recents.
        //
        // The span opens here, not in `open(_ session:)`: §7.3 measures from the open panel
        // returning, and on the shipped path that is a whole scan away. It takes *any* frame,
        // because the row is "a photograph on screen", and the first paint is legitimately the
        // 256 px stand-in while the display decode is still running.
        beginFrameInterval(Signposts.openToFirstPhoto, accepts: .anyFrame)
        let previous = phase
        recordRecent()
        reportedXMPFailure = false
        backend.flush()
        if let asyncSessionFactory {
            openTask?.cancel()
            // A provisional frame for a folder the user has already moved on from. Left up, the next
            // folder would be read behind a photograph from the last one, under this folder's name.
            dropProvisionalFrame()
            phase = .loading(LoadProgress(title: "Opening \(url.lastPathComponent)", fraction: 0))
            startFastPath(url)
            openTask = Task { [weak self] in
                do {
                    let session = try await asyncSessionFactory(url)
                    guard !Task.isCancelled, let self else {
                        session.flush()
                        return
                    }
                    self.open(session, folderName: url.lastPathComponent)
                } catch {
                    guard !Task.isCancelled, let self else { return }
                    // No photograph is coming, so the span must not stay open across the rest of
                    // the session: it would report the time until the next folder's first frame.
                    self.endFrameInterval()
                    self.dropProvisionalFrame()
                    self.lastError = "Couldn't open \(url.lastPathComponent): \(error.localizedDescription)"
                    // Back to the shoot that was open, if there was one, rather than to Welcome.
                    self.phase = self.allPhotos.isEmpty ? .welcome : Self.restorable(previous)
                }
            }
            return
        }
        do {
            let session = try sessionFactory(url)
            open(session, folderName: url.lastPathComponent)
        } catch {
            endFrameInterval()
            lastError = "Couldn't open \(url.lastPathComponent): \(error.localizedDescription)"
            phase = allPhotos.isEmpty ? .welcome : Self.restorable(previous)
        }
    }

    private static func restorable(_ phase: Phase) -> Phase {
        switch phase {
        case .culling, .finishing: phase
        case .welcome, .loading: .culling
        }
    }

    public func open(_ session: any SessionBackend, folderName: String? = nil) {
        fastPathTask?.cancel()
        fastPathTask = nil
        isProvisional = false
        // The real session has arrived, so whatever the provisional frame was standing in front of is
        // gone for good: this open either succeeded or the frame is about to be dropped.
        suspendedShoot = nil
        let data = session.data
        backend.listener = nil
        backendBox.session = session
        session.listener = self
        session.applyMetadataSettings(settings.metadata)
        // The core's Finish plan and `tierCounts` read the keep threshold, and it is not stored on
        // the session (the app's settings are the record). Without this, "only 5 stars" changes the
        // filmstrip while Finish still keeps every 4-star photo, and the summary sheet disagrees
        // with the ratings the user just made.
        session.setKeepThreshold(settings.keepThreshold)
        self.folderName = folderName ?? (data.folder as NSString).lastPathComponent
        skippedFiles = data.skipped
        allPhotos = data.photos.map {
            PhotoVM(
                meta: $0, rating: data.ratings[$0.id] ?? Rating(), mode: ratingMode,
                keepThreshold: settings.keepThreshold)
        }
        reindexPhotos()
        rebuildBatches(from: data.batches, visited: data.visited)
        recomputeCounts()

        // Hand the pipeline the folder and the photo list *before* anything asks it for an image.
        //
        // This was missing, and the app looked broken in a way nothing pointed at: the window came
        // up, the toolbar said "Batch 1 of 1 — IMG_6117.CR3", the HUD counted 10 photos, and both
        // the viewer and every filmstrip cell were black. The provider had no file table, so
        // `thumbnail` and `displayImage` both returned nil for every id and the app reported no
        // error at all. `setFocus` alone is not enough — it says *which* photos to keep ready, and
        // not *where they are*.
        if let provider = images as? ImageProvider {
            provider.open(folder: URL(fileURLWithPath: data.folder), photos: data.photos)
        }

        // Resume where the user left off, else the first photo of the first batch (§9.4, §11).
        if let cursor = data.cursor, let batch = batchIndexByID[cursor.batch],
            let index = photoIndex(of: cursor.photo), batches[batch].range.contains(index)
        {
            currentBatchIndex = batch
            currentPhotoIndex = index - batches[batch].range.lowerBound
        } else {
            currentBatchIndex = 0
            currentPhotoIndex = 0
        }
        markCurrentBatchVisited()
        phase = .culling
        pushCursor()
        updatePipelineFocus()
        recordRecent()
        startWatchingFolder()
        startVisualSignatures()
    }

    // MARK: - Visual signatures (task.md §5.4, phase two)

    /// Refines the ambiguous batch boundaries by how the frames look, in the background. Only for a
    /// real folder: a mock backend has no files to decode.
    private func startVisualSignatures() {
        visualSigWorker.cancel()
        guard backend.canRescan, !allPhotos.isEmpty else { return }
        let photos = allPhotos.map(\.meta)
        let index = currentBatch.map { $0.range.lowerBound + currentPhotoIndex } ?? 0
        // The pipeline's cache is shared so the filmstrip and the signatures do not each decode the
        // same photograph. `ImageProvider` satisfies it; without it the worker decodes the whole
        // shoot from the file URLs, on top of the decodes the user is waiting for.
        let shared = images as? any ThumbnailSource
        visualSigWorker.start(
            photos: photos, folder: URL(fileURLWithPath: backend.data.folder), startingAt: index,
            thumbnails: shared
        ) { [weak self] sigs in
            self?.backend.submitVisualSigs(sigs)
        }
    }

    // MARK: - Files appearing and vanishing (task.md §11)

    private func startWatchingFolder() {
        folderWatcher?.stop()
        folderWatcher = nil
        guard backend.canRescan, !backend.data.folder.isEmpty else { return }
        let watcher = FolderWatcher(folder: URL(fileURLWithPath: backend.data.folder)) { [weak self] in
            self?.folderDidChange()
        }
        watcher.start()
        folderWatcher = watcher
    }

    /// The set of photo files changed on disk (a card dump, a delete in Finder, a Finish run).
    /// Re-reads the folder through the core, which recognises renames so a rating survives one
    /// (REV-68), then rebuilds the model around what is there now while keeping the user on the
    /// photo they were looking at. Cheap when nothing that matters changed.
    public func folderDidChange() {
        // `!isProvisional`, although a provisional frame is up in `.culling`: the watcher still
        // belongs to the *previous* folder, so its event would rebuild this screen around that
        // folder's files while the frame says it is the one being read. The folder being opened
        // rescans for itself when its session arrives, so dropping the event loses nothing.
        guard phase == .culling || phase == .finishing, !isProvisional,
            let fresh = backend.rescan()
        else { return }
        let selected = currentPhoto?.id
        let before = Set(allPhotos.map(\.id))
        let after = Set(fresh.photos.map(\.id))
        skippedFiles = fresh.skipped

        if before == after {
            // Same photographs (a rename that kept its id, or a change to a sidecar): nothing to
            // rebuild, and rebuilding would drop the caches for no reason.
            return
        }

        allPhotos = fresh.photos.map {
            PhotoVM(
                meta: $0, rating: fresh.ratings[$0.id] ?? Rating(), mode: ratingMode,
                keepThreshold: settings.keepThreshold)
        }
        reindexPhotos()
        rebuildBatches(from: fresh.batches, visited: fresh.visited)
        if let provider = images as? ImageProvider {
            provider.open(folder: URL(fileURLWithPath: fresh.folder), photos: fresh.photos)
        }

        // Stay on the same photograph when it is still there.
        if let selected, let index = photoIndex[selected],
            let batch = batches.firstIndex(where: { $0.range.contains(index) })
        {
            currentBatchIndex = batch
            currentPhotoIndex = index - batches[batch].range.lowerBound
        } else {
            currentBatchIndex = min(currentBatchIndex, max(0, batches.count - 1))
            currentPhotoIndex = min(currentPhotoIndex, max(0, (currentBatch?.count ?? 1) - 1))
        }
        recomputeCounts()
        pushCursor()
        updatePipelineFocus()
        // New photographs need signatures too, and the ones already computed are cheap to redo.
        startVisualSignatures()
    }

    /// Remembers this folder and how far through it the user is. Called when a folder opens, when
    /// it closes and when the app quits, which is enough for "412 of 708 rated" on the Welcome
    /// screen without writing a file on every keystroke.
    public func recordRecent() {
        // A provisional frame is one photo of a folder that has not been read yet: recording it
        // would put "1 of 1 rated" in the Welcome list for a 708-photo shoot.
        guard !isProvisional, phase == .culling || phase == .finishing, !allPhotos.isEmpty else {
            return
        }
        let folder = URL(fileURLWithPath: backend.data.folder, isDirectory: true)
        guard !folder.path.isEmpty else { return }
        recents = RecentFoldersStore.recording(
            RecentFolder(
                path: folder.path,
                name: folderName.isEmpty ? folder.lastPathComponent : folderName,
                openedAt: Date(), totalPhotos: allPhotos.count, ratedPhotos: ratedCount),
            in: recents)
        recentsStore.save(recents)
    }

    public func forgetRecent(_ folder: RecentFolder) {
        recents.removeAll { $0.path == folder.path }
        recentsStore.save(recents)
    }

    /// Opens a folder from the Welcome list. One that has gone (an unplugged card) is reported and
    /// stays in the list, since plugging the card back in brings it back.
    public func openRecent(_ folder: RecentFolder) {
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: folder.path, isDirectory: &isDirectory),
            isDirectory.boolValue
        else {
            lastError = "\(folder.name) is not available. Connect the drive or card and try again."
            return
        }
        open(folder: folder.url)
    }

    /// Everything that must reach the disk before the process ends: the debounced XMP queue
    /// (task.md §6.3, "flushed on batch change and on quit"), the settings and the recents list.
    public func prepareForQuit() {
        endFrameInterval()
        recordRecent()
        backend.flush()
        flushSettings()
    }

    /// The first-photo fast path: one header read and one decode, in parallel with the full scan.
    ///
    /// §7.3's "folder open → first photo < 1 s" is not achievable by waiting for the scan — a cold
    /// 2,880-file shoot is over a second of header reads before anything is known. So the open does
    /// two cheap things first (see `CoreFirstPhoto`): name a file from the directory listing, read
    /// that one header, and show it. The scan, the batching and the T2 prefetch then run behind it.
    ///
    /// The frame is provisional and says so: `isProvisional` makes every mutating command a no-op
    /// until the session arrives, because the backend those commands would go through still belongs
    /// to the *previous* folder. The photograph is the first by **file name**; when the scan lands
    /// the app moves to the first in **capture order**, which is the same file for every normal card
    /// dump and a different one after a `IMG_9999 → IMG_0001` rollover — the price of a first frame
    /// that costs two small reads instead of 2,880.
    private func startFastPath(_ url: URL) {
        fastPathTask?.cancel()
        guard let firstPhoto else { return }
        fastPathTask = Task { [weak self] in
            let meta = await Task.detached(priority: .userInitiated) { firstPhoto(url) }.value
            guard !Task.isCancelled, let self, let meta else { return }
            // The open it belongs to may have been superseded while the read was running.
            guard self.phase.isLoading else { return }
            self.showProvisionalFrame(meta, folder: url)
        }
    }

    /// One photograph on screen, with nothing else the user can act on yet.
    private func showProvisionalFrame(_ meta: PhotoMeta, folder: URL) {
        // Borrowed, not kept: a folder that fails to open has to put the shoot that *was* open back
        // on screen, which is what `Phase.restorable` does for the phase alone. One copy of the
        // shoot's value-type state, taken once per open, is what buys that.
        suspendedShoot = ShootState(
            photos: allPhotos, batches: batches, photoIndex: photoIndex,
            batchIndexByID: batchIndexByID, currentBatchIndex: currentBatchIndex,
            currentPhotoIndex: currentPhotoIndex, folderName: folderName, folderURL: folderURL)
        let photo = PhotoVM(
            meta: meta, rating: Rating(), mode: ratingMode, keepThreshold: settings.keepThreshold)
        allPhotos = [photo]
        batches = [
            BatchVM(
                core: Batch(id: Self.provisionalBatchID, index: 0, photoIds: [meta.id], provisional: true),
                visited: false,
                range: 0..<1)
        ]
        currentBatchIndex = 0
        currentPhotoIndex = 0
        folderName = folder.lastPathComponent
        folderURL = folder
        // The same folder and the same `PhotoID` the scan will produce, so the reconcile in
        // `ImageProvider.open` keeps this decode instead of throwing it away.
        if let provider = images as? ImageProvider {
            provider.open(folder: folder, photos: [meta])
        }
        recomputeCounts()
        isProvisional = true
        phase = .culling
        updatePipelineFocus()
    }

    /// Puts away a provisional frame that the real open did not replace: a folder that failed, or a
    /// shoot the user closed while it was still being read. The shoot that was open before it, if
    /// there was one, comes back — a failed open has always left it alone.
    private func dropProvisionalFrame() {
        fastPathTask?.cancel()
        fastPathTask = nil
        guard isProvisional else { return }
        isProvisional = false
        if let saved = suspendedShoot {
            suspendedShoot = nil
            allPhotos = saved.photos
            batches = saved.batches
            photoIndex = saved.photoIndex
            batchIndexByID = saved.batchIndexByID
            currentBatchIndex = saved.currentBatchIndex
            currentPhotoIndex = saved.currentPhotoIndex
            folderName = saved.folderName
            folderURL = saved.folderURL
            recomputeCounts()
        } else {
            allPhotos = []
            batches = []
            photoIndex = [:]
            batchIndexByID = [:]
            currentBatchIndex = 0
            currentPhotoIndex = 0
            folderName = ""
            folderURL = nil
        }
    }

    /// Whether the user can act on the shoot. False for a provisional frame (see `isProvisional`).
    public var canAct: Bool { phase == .culling && !isProvisional }

    /// A batch id the fast path's frame cannot collide with. The real scan's batch ids come from the
    /// core, so any value outside the space it uses is safe; `UInt64.max` is the largest one it would
    /// not hand out for a first batch.
    static let provisionalBatchID = BatchID.max

    /// The model state a provisional frame replaced, kept so `dropProvisionalFrame` can put it back.
    /// All value types, so a copy is one retain per element and no aliasing to reason about.
    private struct ShootState {
        var photos: [PhotoVM]
        var batches: [BatchVM]
        var photoIndex: [PhotoID: Int]
        var batchIndexByID: [BatchID: Int]
        var currentBatchIndex: Int
        var currentPhotoIndex: Int
        var folderName: String
        var folderURL: URL?
    }

    /// The shoot a provisional frame is standing in front of, if one was open.
    private var suspendedShoot: ShootState?

    public func closeSession() {
        endFrameInterval()
        dropProvisionalFrame()
        folderWatcher?.stop()
        folderWatcher = nil
        visualSigWorker.cancel()
        recordRecent()
        backend.flush()
        backend.listener = nil
        allPhotos = []
        batches = []
        photoIndex = [:]
        batchIndexByID = [:]
        currentBatchIndex = 0
        currentPhotoIndex = 0
        folderName = ""
        folderURL = nil
        finish = .hidden
        phase = .welcome
    }

    // MARK: - The frame the user actually sees
    //
    // todo.md §7.3's sharpest target is "arrow key → sharp photo ≤ 8 ms", and the only honest way to
    // measure it is from the keystroke to the *presented frame* — not to the handler returning, and
    // not to the decode finishing. So an interval opens here, in the command handler, and is closed by
    // the view layer at the CATransaction that commits the new image.
    //
    // `pendingFrameInterval` holds the one that is open. Only one can be: two overlapping key-to-frame
    // spans are not a measurement, and holding the latest keeps key repeat from accumulating a queue
    // of intervals that will all be closed by the same frame.

    private var pendingFrameInterval: SignpostInterval?
    /// What the open span is waiting for, so the right end can be chosen. A `standIn` is not a
    /// measurement of anything §7.3 claims, so those spans stay open until the display decode lands.
    private var pendingFrameWaitsFor: FrameSpan.Accepts = .displayOnly
    private var frameIntervalStart: ContinuousClock.Instant?

    /// How long the last key-to-frame took, in milliseconds, for the debug HUD and for tests. Read
    /// only after `frameDidPresent()`.
    public private(set) var lastFrameLatencyMs: Double?
    /// The worst key-to-frame since the folder opened, which is the number a photographer feels: a
    /// cull is hundreds of arrow presses, so the p99 is a press, not a statistic.
    public private(set) var worstFrameLatencyMs: Double?
    /// How many spans have closed. A span that never closes is a `focusMisses` bug, and this is the
    /// other half of that story: keys pressed versus frames delivered.
    public private(set) var framesPresented = 0
    /// Frames that reached the user as the 256 px stand-in rather than the display decode. Anything
    /// above zero is §7.1 broken: the user saw a thumbnail where the photograph should have been.
    public private(set) var standInFramesPresented = 0

    /// Opens the span, replacing (and ending) any that is still open.
    private func beginFrameInterval(_ name: StaticString, accepts: FrameSpan.Accepts = .displayOnly) {
        endFrameInterval()
        pendingFrameInterval = SignpostInterval.begin(name)
        pendingFrameWaitsFor = accepts
        frameIntervalStart = .now
    }

    /// Called by the viewer once a frame has been committed. Public because the view layer is a
    /// different type, and @testable would make the wiring look optional when it is not.
    ///
    /// A `standIn` does **not** close a span that is waiting for the display decode. The alternative
    /// is a signpost reporting sub-millisecond key-to-frame over a soft picture, which is the exact
    /// lie §7.3 is written to rule out.
    public func frameDidPresent(_ frame: PresentedFrame = .display) {
        if frame == .standIn {
            // Counted even with no span open: this counter is "the user saw a thumbnail", and a frame
            // that arrives outside a measurement is still a frame the user looked at.
            standInFramesPresented += 1
        }
        guard let start = frameIntervalStart, pendingFrameInterval != nil else { return }
        if frame == .standIn, pendingFrameWaitsFor != .anyFrame { return }
        let elapsed = ContinuousClock.now - start
        let milliseconds =
            Double(elapsed.components.seconds) * 1000 + Double(elapsed.components.attoseconds) / 1e15
        lastFrameLatencyMs = milliseconds
        worstFrameLatencyMs = max(worstFrameLatencyMs ?? 0, milliseconds)
        framesPresented += 1
        endFrameInterval()
    }

    /// Ends any open span without a frame arriving — the window closing, a folder changing, quit.
    /// Leaving one open would put a permanently unclosed interval in every trace from that point on.
    public func endFrameInterval() {
        pendingFrameInterval?.end()
        pendingFrameInterval = nil
        frameIntervalStart = nil
    }

    // MARK: - Commands

    /// The single entry point for every user action.
    public func perform(_ command: Command) {
        switch command {
        case .photoPrevious: navigateForFrame(Signposts.keyToFrame) { movePhoto(by: -1) }
        case .photoNext: navigateForFrame(Signposts.keyToFrame) { movePhoto(by: 1) }
        case .batchPrevious: navigateForFrame(Signposts.batchToFrame) { moveBatch(by: -1) }
        case .batchNext: navigateForFrame(Signposts.batchToFrame) { moveBatch(by: 1) }

        case .setStars(let stars): rateCurrent { $0.stars = UInt8(min(max(stars, 0), 5)) }
        case .setStarsAndAdvance(let stars):
            rateCurrent { $0.stars = UInt8(min(max(stars, 1), 5)) }
            advance()  // ⇧1…⇧5 always advances, auto-advance or not (Lightroom)
        case .togglePickFlag: rateCurrent { $0.flag = $0.flag == .pick ? .none : .pick }
        case .setRatingMode(let mode): updateSettings { $0.general.ratingMode = mode }
        case .jumpBatch(let delta):
            // Leaving a batch, so `enteringBatchBehavior` does not apply: forward goes to the next
            // batch's first frame and back to the previous batch's last, whichever way the user
            // entered the batch they are leaving.
            guard delta != 0 else { return }
            moveBatch(by: delta > 0 ? 1 : -1, selecting: delta > 0 ? .first : .last)
        case .toggleKeep: rateCurrent { $0.keep = !$0.keep }
        // The two buttons, and the point of them being separate: idempotent. A "Keep" button that
        // toggled would make a second click mean "not keep", which is not what the button says.
        case .setKeep: rateCurrent { $0.keep = true }
        case .setNotKeep: rateCurrent { $0.keep = false }
        case .rejectFlag: rateCurrent { $0.flag = .reject }
        case .unflag: rateCurrent { $0.flag = .none }
        case .toggleFlag: rateCurrent { $0.flag = $0.flag == .none ? .pick : .none }
        case .setLabel(let label): rateCurrent { $0.label = label }

        case .toggleAutoAdvance:
            autoAdvanceOverride = !autoAdvance
        case .toggleInfoPanel: infoPanelVisible.toggle()
        case .toggleClippingOverlay: clippingOverlay.toggle()
        case .toggleAFOverlay: afOverlay.toggle()
        case .toggleHUD: hudVisible.toggle()
        case .toggleZoomLock: viewer.zoomLock.toggle()

        case .showLoupe: setViewMode(.loupe)
        case .showGrid: setViewMode(.grid)
        case .showCompare(let count): setViewMode(.compare(min(max(count, 2), 4)))

        case .toggleZoom(let point):
            // Click: 100% at that spot, click again: back to fit (task.md §9.2).
            if viewer.zoomed {
                viewer.zoomed = false
                viewer.anchor = nil
            } else {
                // §7.3's "100% zoom < 150 ms first time". The span opens on the click and closes on
                // the frame that answers it, so a zoom that shows a stale bitmap cannot read as fast.
                beginFrameInterval(Signposts.zoomToSharp)
                viewer.zoomed = true
                viewer.anchor = point ?? viewer.anchor
            }
            updatePipelineFocus()
        case .magnify(let factor, let point):
            if factor > 1.01 {
                if !viewer.zoomed { beginFrameInterval(Signposts.zoomToSharp) }
                viewer.zoomed = true
                viewer.anchor = point ?? viewer.anchor
            } else if factor < 0.99, viewer.zoomed {
                viewer.zoomed = false
                viewer.anchor = nil
            }
            updatePipelineFocus()

        case .undo: undo()
        case .redo: redo()
        case .openFolder: onRequestOpenFolder?()
        case .finishCull: startFinish()
        case .toggleFullScreen: onRequestToggleFullScreen?(true)
        }
    }

    // MARK: - Navigation

    /// Runs a navigation with its key-to-frame span around it, and closes the span immediately if the
    /// navigation did not move.
    ///
    /// A → at the last photo of the shoot presents no new frame, and a span left open would then
    /// report the time until the *next* keystroke — a slow number for a key that did nothing, which
    /// is worse than no number because it is a wrong one.
    private func navigateForFrame(_ name: StaticString, _ navigate: () -> Void) {
        let before = (currentBatchIndex, currentPhotoIndex)
        beginFrameInterval(name)
        navigate()
        if (currentBatchIndex, currentPhotoIndex) == before { endFrameInterval() }
    }

    public func movePhoto(by delta: Int) {
        guard let batch = currentBatch, batch.count > 0 else { return }
        let target = currentPhotoIndex + delta
        if target >= 0, target < batch.count {
            setCurrentPhoto(target)
            return
        }
        switch settings.general.arrowBehaviorAtBatchEnd {
        case .stop:
            return
        case .continueIntoNextBatch:
            // Leaving forward lands on the first photo of the next batch, leaving backward on the
            // last photo of the previous one — the `enteringBatchBehavior` setting is about explicit
            // batch navigation (‹ › / ⌘← ⌘→), not about rolling over an edge.
            if delta > 0, moveBatch(by: 1, selecting: .first) { return }
            if delta < 0, moveBatch(by: -1, selecting: .last) { return }
        }
    }

    /// `index` is within the current batch, like `currentPhotoIndex`.
    public func goToPhoto(_ index: Int) {
        guard let batch = currentBatch, index >= 0, index < batch.count else { return }
        setCurrentPhoto(index)
    }

    @discardableResult
    public func moveBatch(by delta: Int) -> Bool {
        let behavior: Selecting = settings.general.enteringBatchBehavior == .lastViewed ? .lastViewed : .first
        return moveBatch(by: delta, selecting: behavior)
    }

    @discardableResult
    func moveBatch(by delta: Int, selecting: Selecting) -> Bool {
        let target = currentBatchIndex + delta
        guard batches.indices.contains(target) else { return false }
        currentBatchIndex = target
        switch selecting {
        case .first:
            currentPhotoIndex = 0
        case .last:
            currentPhotoIndex = max(0, batches[target].count - 1)
        case .lastViewed:
            let batch = batches[target]
            if let remembered = backend.lastPhotoInBatch(batch.id),
                let index = photoIndex(of: remembered), batch.range.contains(index)
            {
                currentPhotoIndex = index - batch.range.lowerBound
            } else {
                currentPhotoIndex = 0
            }
        }
        markCurrentBatchVisited()
        pushCursor()
        // A batch change flushes the debounced XMP queue (task.md §6.3).
        backend.flush()
        updatePipelineFocus()
        return true
    }

    enum Selecting { case first, last, lastViewed }

    public func setViewMode(_ mode: ViewMode) {
        viewMode = mode
    }

    /// `index` is within the current batch.
    private func setCurrentPhoto(_ index: Int) {
        guard let batch = currentBatch, index >= 0, index < batch.count else { return }
        currentPhotoIndex = index
        markCurrentBatchVisited()
        pushCursor()
        updatePipelineFocus()
    }

    /// Steps past the current photo whatever the arrow-at-ends setting says.
    ///
    /// Auto-advance and ⇧1…⇧5 both exist so you never have to stop in the middle of a burst, and
    /// every burst ends at a batch edge — so crossing into the next batch is the useful behaviour.
    /// A plain → is the one that respects the setting.
    private func advance() {
        if let batch = currentBatch, currentPhotoIndex < batch.count - 1 {
            setCurrentPhoto(currentPhotoIndex + 1)
            return
        }
        moveBatch(by: 1, selecting: .first)
    }

    private func markCurrentBatchVisited() {
        guard batches.indices.contains(currentBatchIndex) else { return }
        if !batches[currentBatchIndex].visited {
            batches[currentBatchIndex].visited = true
            backend.markVisited(batch: batches[currentBatchIndex].id)
        }
    }

    private func pushCursor() {
        guard let batch = currentBatch, let photo = currentPhoto else { return }
        backend.setCursor(batch: batch.id, photo: photo.id)
    }

    /// Tells the pipeline what the user can reach. Called on **every** navigation: a batch switch has
    /// to hit ≤ 1 display frame, so this can't be debounced (task.md §7.3).
    public func updatePipelineFocus() {
        guard currentBatch != nil, let photo = currentPhoto else { return }
        let reach = max(1, settings.performance.lookAheadBatches)
        let lower = max(0, currentBatchIndex - reach)
        let upper = min(batches.count - 1, currentBatchIndex + reach)
        guard lower <= upper else { return }
        let windows = (lower...upper).map {
            FocusWindow(batchID: batches[$0].id, photoIDs: batches[$0].photoIDs)
        }
        images.setFocus(
            FocusRequest(
                windows: windows,
                currentPhoto: photo.id,
                viewportPixelSize: viewportPixelSize,
                zoomed: viewer.zoomed,
                zoomLock: viewer.zoomLock,
                exactRaw: settings.viewer.exactRaw))
    }

    /// The viewer area's backing size. ui calls this when the window resizes so T2 bitmaps are
    /// decoded at exactly the right pixel size (task.md §7.2).
    public func setViewportPixelSize(_ size: CGSize) {
        guard size != viewportPixelSize else { return }
        viewportPixelSize = size
        updatePipelineFocus()
    }

    // MARK: - Rating

    /// Applies a rating change to the current photo, then honours auto-advance.
    ///
    /// "Only photos in the current batch can be rated" (task.md §6.3) is enforced structurally: the
    /// only rating entry point takes the *current* photo, and `currentPhoto` is always inside
    /// `currentBatch`. There is no API that can rate a photo in another batch.
    public func rateCurrent(_ mutate: (inout Rating) -> Void) {
        // `canAct`, not `phase == .culling`: while the first-photo fast path's frame is on screen the
        // backend still belongs to the *previous* folder, so a rating written now would land in the
        // wrong shoot's database and sidecar.
        guard canAct, currentBatch != nil, let photo = currentPhoto else { return }
        let updated = RatingRules.applying(
            to: photo.rating, mode: ratingMode, keepThreshold: settings.keepThreshold, mutate)
        guard updated != photo.rating else { return }
        // The session owns persistence: DB now, XMP debounced ≤ 1 s (session-api.md). Undo works
        // because `setRating` hands back the before/after pair, and it must be recorded *before* the
        // local state moves, or undo would come back with the wrong "before".
        _ = backend.setRating(photo: photo.id, updated)
        applyRatingLocally(photo.id, updated)
    }

    /// Updates the model's copy of a photo's rating. Never touches the session — callers that change
    /// a rating persist it first.
    private func applyRatingLocally(_ id: PhotoID, _ rating: Rating) {
        guard let index = photoIndex[id] else { return }
        var photo = allPhotos[index]
        let previousRating = photo.rating
        let previousTier = photo.tier
        photo.rating = rating
        photo.tier = RatingRules.tier(of: rating, mode: ratingMode, keepThreshold: settings.keepThreshold)
        photo.isKeep = RatingRules.isKeep(rating, mode: ratingMode, keepThreshold: settings.keepThreshold)
        allPhotos[index] = photo

        if previousTier != photo.tier {
            countsByTier[previousTier, default: 0] -= 1
            countsByTier[photo.tier, default: 0] += 1
        }
        if RatingRules.isReviewed(previousRating) != RatingRules.isReviewed(rating) {
            ratedCount += RatingRules.isReviewed(rating) ? 1 : -1
        }
        if autoAdvance, currentPhoto?.id == id { advanceAfterRating() }
    }

    private func advanceAfterRating() {
        advance()
    }

    // MARK: - Undo

    /// Undo navigates to the batch and photo the change was made in, then reverts it, so the "only
    /// rate in the current batch" rule still holds visibly (task.md §6.3).
    public func undo() {
        guard canAct else { return }
        guard let change = backend.undo() else { return }
        apply(change: change, using: change.before)
    }

    public func redo() {
        guard canAct else { return }
        guard let change = backend.redo() else { return }
        apply(change: change, using: change.after)
    }

    private func apply(change: RatingChange, using rating: Rating) {
        navigate(to: change.photo, in: change.batch)
        if let index = photoIndex[change.photo] {
            allPhotos[index].rating = rating
            allPhotos[index].tier = RatingRules.tier(
                of: rating, mode: ratingMode, keepThreshold: settings.keepThreshold)
            allPhotos[index].isKeep = RatingRules.isKeep(
                rating, mode: ratingMode, keepThreshold: settings.keepThreshold)
        }
        recomputeCounts()
        updatePipelineFocus()
    }

    /// Moves the cursor to a photo in another batch if it's still there. A change whose batch was
    /// re-batched away is still reverted; the cursor simply stays where it was.
    private func navigate(to photo: PhotoID, in batch: BatchID) {
        guard let target = batchIndexByID[batch], let index = photoIndex[photo],
            batches[target].range.contains(index)
        else { return }
        let batchChanged = target != currentBatchIndex
        currentBatchIndex = target
        currentPhotoIndex = index - batches[target].range.lowerBound
        if batchChanged {
            markCurrentBatchVisited()
            pushCursor()
            backend.flush()
        }
    }

    // MARK: - Settings

    /// The one place settings change, so persistence, tier recomputation and the pipeline stay in
    /// step.
    public func updateSettings(_ mutation: (inout AppSettings) -> Void) {
        let before = settings
        mutation(&settings)
        guard settings != before else { return }

        let thresholdChanged = settings.keepThreshold != before.keepThreshold
        if settings.general.ratingMode != before.general.ratingMode {
            applyRatingModeChange(from: before.general.ratingMode)
        } else if thresholdChanged {
            recomputeTiers()
        }
        if thresholdChanged {
            // The core holds the threshold in memory rather than in the database (the app's
            // settings are the record), so it has to be told again: Finish plans and the summary
            // sheet's tier counts read it there.
            backend.setKeepThreshold(settings.keepThreshold)
        }
        if settings.viewer.zoomLock != before.viewer.zoomLock {
            viewer.zoomLock = settings.viewer.zoomLock
        }
        if settings.viewer.backgroundGray != before.viewer.backgroundGray {
            viewer.backgroundGray = settings.viewer.backgroundGray
        }
        if settings.metadata != before.metadata {
            backend.applyMetadataSettings(settings.metadata)
        }
        scheduleSave()
        updatePipelineFocus()
    }

    /// Switching mode mid-session is allowed and loses nothing: `stars` and `keep` are kept
    /// consistent all the time, so the only work is re-deriving the tiers (§6).
    private func applyRatingModeChange(from previous: RatingMode) {
        for index in allPhotos.indices {
            var photo = allPhotos[index]
            let synchronized = RatingRules.synchronized(
                photo.rating, mode: ratingMode, keepThreshold: settings.keepThreshold)
            if synchronized != photo.rating {
                photo.rating = synchronized
                backend.setRating(photo: photo.id, synchronized)
            }
            photo.tier = RatingRules.tier(
                of: photo.rating, mode: ratingMode, keepThreshold: settings.keepThreshold)
            photo.isKeep = RatingRules.isKeep(
                photo.rating, mode: ratingMode, keepThreshold: settings.keepThreshold)
            allPhotos[index] = photo
        }
        _ = previous
        recomputeCounts()
    }

    private func recomputeTiers() {
        for index in allPhotos.indices {
            allPhotos[index].tier = RatingRules.tier(
                of: allPhotos[index].rating, mode: ratingMode, keepThreshold: settings.keepThreshold)
            allPhotos[index].isKeep = RatingRules.isKeep(
                allPhotos[index].rating, mode: ratingMode, keepThreshold: settings.keepThreshold)
        }
        recomputeCounts()
    }

    private func scheduleSave() {
        saveTask?.cancel()
        let snapshot = settings
        let store = settingsStore
        saveTask = Task { [weak self] in
            // Debounced so dragging a slider doesn't hammer the disk; still atomic when it lands.
            try? await Task.sleep(for: .milliseconds(400))
            guard !Task.isCancelled else { return }
            try? store.save(snapshot)
            _ = self
        }
    }

    /// Force pending writes out — on quit, and available to qa for deterministic tests.
    public func flushSettings() {
        saveTask?.cancel()
        saveTask = nil
        try? settingsStore.save(settings)
    }

    // MARK: - Keymap

    public func updateKeymap(_ mutation: (inout Keymap) -> Void) {
        mutation(&keymap)
        keymapStore.setOverrides(keymap)
        try? keymapStore.save()
    }

    public func resetKeymap() {
        keymap = keymapStore.defaults
        keymapStore.resetToDefaults()
        try? keymapStore.save()
    }

    public func importKeymap(from url: URL) throws {
        try keymapStore.importKeymap(from: url)
        keymap = keymapStore.effective
    }

    public func exportKeymap(to url: URL) throws {
        try keymapStore.exportKeymap(to: url)
    }

    public func keymapConflicts() -> [ChordConflict] { keymap.conflicts() }

    // MARK: - Finish Cull

    public func startFinish() {
        guard canAct, !batches.isEmpty else { return }
        // The typed word protects an irreversible run; this protects the decision to start finishing
        // at all. Both are the user's settings, and both live in the model so they are testable.
        finish = settings.general.confirmBeforeFinish ? .confirm : .summary(summary)
    }

    /// The answer to `startFinish`'s question, from the sheet's Confirm button.
    public func confirmFinish() {
        guard case .confirm = finish else { return }
        finish = .summary(summary)
    }

    public func showFinishOptions() {
        guard case .summary(let current) = finish else { return }
        finish = .options(
            current,
            FinishSettings(
                unkept: settings.general.finishUnkept, kept: settings.general.finishKept,
                ratingMode: ratingMode))
    }

    public func updateFinishSettings(_ mutation: (inout FinishSettings) -> Void) {
        guard case .options(let current, var options) = finish else { return }
        mutation(&options)
        options.ratingMode = ratingMode  // the mode is what defines "kept", so it can't be faked
        finish = .options(current, options)
    }

    public func runFinishDryRun() {
        guard case .options(let current, var options) = finish else { return }
        let plan = backend.planFinish(options)
        // Whether the user has to type DELETE is decided **here**, not in the sheet: this is where
        // the plan, the settings and the stage are all in hand, and a rule that lives in a private
        // SwiftUI view is a rule no test can reach — which is why the typed gate had no coverage at
        // all. `isDestructive` alone is the fact; the setting is the user's answer to it.
        options.requiresTypedConfirmation =
            settings.general.confirmPermanentDelete && options.unkept.isDestructive && !plan.ops.isEmpty
        finish = .dryRun(current, options, plan)
    }

    public func executeFinish() {
        guard case .dryRun(let current, let options, let plan) = finish else { return }
        finish = .executing(current, options)
        let report = backend.executeFinish(plan)
        finish = .report(current, report)
        // The files moved, so the ratings on disk are the source of truth from here: re-read them.
        backend.flush()
        folderDidChange()
    }

    public func undoFinish() {
        guard case .report(let current, let report) = finish, report.undoable, !report.wasUndo else {
            return
        }
        var undoReport = backend.undoFinish()
        // One undo per run: the core consumes the run it reverses, so a second press would reverse
        // an *earlier* Finish. The sheet reads `wasUndo` to drop the button.
        undoReport.wasUndo = true
        undoReport.undoable = false
        finish = .report(current, undoReport)
        folderDidChange()
    }

    /// One step back through the sheet: dry run → options → summary. Nothing has touched the disk
    /// in any of those stages, so going back is free.
    public func backFinish() {
        switch finish {
        case .options(let summary, _): finish = .summary(summary)
        case .dryRun(let summary, let options, _): finish = .options(summary, options)
        default: break
        }
    }

    public func cancelFinish() {
        finish = .hidden
        // A Finish that moved every photo out leaves nothing to cull: go back to the Welcome
        // screen instead of an empty viewer.
        if phase != .welcome, allPhotos.isEmpty { closeSession() }
    }

    // MARK: - SessionListener

    public func sessionDidChangeBatches(_ newBatches: [Batch]) {
        let stayingID = currentBatch?.id
        let stayingPhoto = currentPhoto?.id
        // A re-batch never re-opens a batch the user has been in, so "visited" is remembered by id.
        let visited = dataVisited()
        rebuildBatches(from: newBatches, visited: visited)

        // The user must not be moved by a re-batch. Their batch was visited, so it comes back
        // unchanged under the same id; its *index* can shift if earlier unvisited batches merged.
        if let stayingID, let index = batchIndexByID[stayingID] {
            currentBatchIndex = index
        } else {
            currentBatchIndex = min(currentBatchIndex, max(0, self.batches.count - 1))
        }
        if let stayingPhoto, let index = photoIndex[stayingPhoto],
            currentBatch?.range.contains(index) == true
        {
            currentPhotoIndex = index - batches[currentBatchIndex].range.lowerBound
        } else {
            currentPhotoIndex = min(currentPhotoIndex, max(0, (currentBatch?.count ?? 1) - 1))
        }
        recomputeCounts()
        pushCursor()
        updatePipelineFocus()
    }

    public func sessionDidChangeFiles() {
        // FSEvents: new files land at the end, deleted ones drop out. The real session rebuilds its
        // snapshot and calls sessionDidChangeBatches; nothing to do here beyond a refresh.
        rebuildBatches(from: backend.data.batches, visited: dataVisited())
        recomputeCounts()
        updatePipelineFocus()
    }

    public func sessionDidFailWritingXMP(photo: PhotoID, message: String) {
        guard !reportedXMPFailure else { return }
        reportedXMPFailure = true
        lastError =
            "Ratings could not be saved next to the photos (\(message)). They are still kept in Firstcut's own record of this folder."
    }

    public func dismissError() {
        lastError = nil
    }

    public func sessionDidImportRatings(_ ratings: [PhotoID: Rating]) {
        for (id, rating) in ratings {
            applyRatingLocally(id, rating)
        }
        recomputeCounts()
    }

    // MARK: - Plumbing

    private func dataVisited() -> Set<BatchID> {
        Set(batches.filter(\.visited).map(\.id))
    }

    private func reindexPhotos() {
        photoIndex = Dictionary(
            allPhotos.enumerated().map { ($0.element.id, $0.offset) }, uniquingKeysWith: { first, _ in first }
        )
    }

    private func rebuildBatches(from core: [Batch], visited: Set<BatchID>) {
        var rebuilt: [BatchVM] = []
        for batch in core {
            let indices = batch.photoIds.compactMap { photoIndex[$0] }
            guard let start = indices.min() else { continue }
            let end = start + indices.count
            rebuilt.append(
                BatchVM(
                    core: Batch(
                        id: batch.id, index: UInt32(rebuilt.count), photoIds: batch.photoIds,
                        provisional: batch.provisional),
                    visited: visited.contains(batch.id),
                    range: start..<end))
        }
        batches = rebuilt
        batchIndexByID = Dictionary(
            rebuilt.enumerated().map { ($0.element.id, $0.offset) }, uniquingKeysWith: { first, _ in first })
    }

    private func recomputeCounts() {
        var counts: [Tier: Int] = [:]
        var reviewed = 0
        for photo in allPhotos {
            counts[photo.tier, default: 0] += 1
            if RatingRules.isReviewed(photo.rating) { reviewed += 1 }
        }
        countsByTier = counts
        ratedCount = reviewed
    }

}

@MainActor
final class SessionBox {
    var session: any SessionBackend
    init() { session = MockSession(data: SessionData(folder: "", photos: [], batches: [])) }
}

extension Array {
    /// `count(where:)` without the shadowing awkwardness of the stdlib's deprecated spelling.
    func count(where predicate: (Element) throws -> Bool) rethrows -> Int {
        var total = 0
        for element in self where try predicate(element) { total += 1 }
        return total
    }
}
