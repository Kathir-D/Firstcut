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
    public private(set) var lastError: String?

    // MARK: Modes and toggles

    public private(set) var viewMode: ViewMode = .loupe
    public private(set) var infoPanelVisible: Bool = false
    public private(set) var hudVisible: Bool = true
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
    private let asyncSessionFactory: ((URL) async throws -> any SessionBackend)?
    private var openTask: Task<Void, Never>?
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
    private var viewportPixelSize: CGSize = .zero
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
        // previews) it is the synchronous path below.
        if let asyncSessionFactory {
            openTask?.cancel()
            phase = .loading(LoadProgress(title: "Opening \(url.lastPathComponent)", fraction: 0))
            openTask = Task { [weak self] in
                do {
                    let session = try await asyncSessionFactory(url)
                    guard !Task.isCancelled, let self else {
                        session.flush()
                        return
                    }
                    self.open(session, folderName: url.lastPathComponent)
                } catch {
                    guard !Task.isCancelled else { return }
                    self?.lastError = "Couldn't open \(url.lastPathComponent): \(error.localizedDescription)"
                    self?.phase = .welcome
                }
            }
            return
        }
        do {
            let session = try sessionFactory(url)
            open(session, folderName: url.lastPathComponent)
        } catch {
            lastError = "Couldn't open \(url.lastPathComponent): \(error.localizedDescription)"
            phase = .welcome
        }
    }

    public func open(_ session: any SessionBackend, folderName: String? = nil) {
        let data = session.data
        backend.listener = nil
        backendBox.session = session
        session.listener = self
        session.applyMetadataSettings(settings.metadata)
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
        visualSigWorker.start(
            photos: photos, folder: URL(fileURLWithPath: backend.data.folder), startingAt: index
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
        guard phase == .culling || phase == .finishing, let fresh = backend.rescan() else { return }
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
        guard phase == .culling || phase == .finishing, !allPhotos.isEmpty else { return }
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
    /// dropped from the list instead of failing silently every time.
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
        recordRecent()
        backend.flush()
        flushSettings()
    }

    public func closeSession() {
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

    // MARK: - Commands

    /// The single entry point for every user action.
    public func perform(_ command: Command) {
        switch command {
        case .photoPrevious: movePhoto(by: -1)
        case .photoNext: movePhoto(by: 1)
        case .batchPrevious: moveBatch(by: -1)
        case .batchNext: moveBatch(by: 1)

        case .setStars(let stars): rateCurrent { $0.stars = UInt8(min(max(stars, 0), 5)) }
        case .setStarsAndAdvance(let stars):
            rateCurrent { $0.stars = UInt8(min(max(stars, 1), 5)) }
            advance()  // ⇧1…⇧5 always advances, auto-advance or not (Lightroom)
        case .togglePickFlag: rateCurrent { $0.flag = $0.flag == .pick ? .none : .pick }
        case .toggleKeep: rateCurrent { $0.keep = !$0.keep }
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
                viewer.zoomed = true
                viewer.anchor = point ?? viewer.anchor
            }
            updatePipelineFocus()
        case .magnify(let factor, let point):
            if factor > 1.01 {
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
        guard let batch = currentBatch, let photo = currentPhoto else { return }
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
        guard phase == .culling, currentBatch != nil, let photo = currentPhoto else { return }
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
        guard let change = backend.undo() else { return }
        apply(change: change, using: change.before)
    }

    public func redo() {
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

        if settings.general.ratingMode != before.general.ratingMode {
            applyRatingModeChange(from: before.general.ratingMode)
        } else if settings.keepThreshold != before.keepThreshold {
            recomputeTiers()
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
        guard phase == .culling, !batches.isEmpty else { return }
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
        guard case .options(let current, let options) = finish else { return }
        finish = .dryRun(current, options, backend.planFinish(options))
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
        lastError = "XMP: \(message)"
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
            allPhotos.enumerated().map { ($0.element.id, $0.offset) }, uniquingKeysWith: { first, _ in first })
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
