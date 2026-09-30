// Owner: app-logic.
//
// `CoreSessionBackend` against a scripted `CoreSessionAPI`.
//
// The scripted core stands in for the Rust `Session` that `ffi.rs` does not export yet. It is a
// fake in the sense that it holds dictionaries, not a SQLite database, and it is faithful in the
// sense that it answers on a background thread whenever the real one would, so the parts of this
// file that matter — the hop to the main actor, snapshot discipline, and a cursor held by
// `BatchID` — are tested as they will actually run.
//
// What is NOT tested here, because it does not exist: any interaction with Rust. See
// `CoreBridgeTests.exportsAreNotStale` and the seam note at the top of
// `App/Sources/Session/CoreSessionBackend.swift`.

import Foundation
import Testing

@testable import Firstcut

// MARK: - The scripted core

/// A `CoreSessionAPI` that records what it was asked and can fire its listener from any thread.
final class ScriptedCoreSession: CoreSessionAPI, @unchecked Sendable {
    let lock = NSLock()
    var data: SessionData
    var mode: RatingMode = .stars
    var undoStack: [RatingChange] = []
    var redoStack: [RatingChange] = []
    var nextChangeID: UInt64 = 1
    var cursors: [SessionCursor] = []
    var visited: [BatchID] = []
    var ratingModeSets: [RatingMode] = []
    var keepThresholdSets: [Int] = []
    var submittedSigBatches = 0
    var flushed = 0
    var closed = 0
    var failRating = false
    var rescanCount = 0
    private var listener: (any CoreSessionListener)?

    init(_ data: SessionData) { self.data = data }

    func setListener(_ listener: any CoreSessionListener) { self.listener = listener }

    func snapshot() -> SessionData {
        lock.lock(); defer { lock.unlock() }
        return data
    }

    func ratingMode() -> RatingMode {
        lock.lock(); defer { lock.unlock() }
        return mode
    }

    func setRatingMode(_ mode: RatingMode) {
        lock.lock(); defer { lock.unlock() }
        ratingModeSets.append(mode)
        self.mode = mode
    }

    /// The core clamps to 4…5, so the fake does too: a test that sends 3 must not see 3 reach a
    /// "core" and conclude the app is allowed to send one.
    func setKeepThreshold(_ stars: Int) {
        lock.lock(); defer { lock.unlock() }
        keepThresholdSets.append(stars)
    }

    func setRating(photo: PhotoID, _ rating: Rating) throws -> RatingChange {
        lock.lock(); defer { lock.unlock() }
        if failRating { throw CocoaError(.fileWriteUnknown) }
        let change = RatingChange(
            id: nextChangeID, photo: photo, batch: 1, before: data.ratings[photo] ?? Rating(),
            after: rating)
        nextChangeID += 1
        data.ratings[photo] = rating
        undoStack.append(change)
        redoStack.removeAll()
        return change
    }

    func undo() -> RatingChange? {
        lock.lock(); defer { lock.unlock() }
        guard let change = undoStack.popLast() else { return nil }
        data.ratings[change.photo] = change.before
        redoStack.append(change)
        return change
    }

    func redo() -> RatingChange? {
        lock.lock(); defer { lock.unlock() }
        guard let change = redoStack.popLast() else { return nil }
        data.ratings[change.photo] = change.after
        undoStack.append(change)
        return change
    }

    func setCursor(batch: BatchID, photo: PhotoID) {
        lock.lock(); defer { lock.unlock() }
        cursors.append(SessionCursor(batch: batch, photo: photo))
        data.cursor = SessionCursor(batch: batch, photo: photo)
        data.lastPhotoInBatch[batch] = photo
    }

    func markVisited(batch: BatchID) {
        lock.lock(); defer { lock.unlock() }
        visited.append(batch)
        data.visited.insert(batch)
    }

    func submitVisualSigs(_ sigs: [(PhotoID, VisualSig)]) {
        lock.lock(); defer { lock.unlock() }
        submittedSigBatches += 1
    }

    func rescan() throws -> SessionData {
        lock.lock(); defer { lock.unlock() }
        rescanCount += 1
        return data
    }

    func planFinish(_ options: FinishSettings) throws -> FinishPlanData {
        FinishPlanData(ops: [FileOp(kind: .move, from: "a", to: "b")], bytesToCopy: 10)
    }

    func executeFinish(_ plan: FinishPlanData) throws -> FinishReportData {
        FinishReportData(done: plan.ops.count, undoable: true)
    }

    func undoFinish() throws -> FinishReportData { FinishReportData(done: 0, undoable: false) }

    func flush() {
        lock.lock(); defer { lock.unlock() }
        flushed += 1
    }

    func close() {
        lock.lock(); defer { lock.unlock() }
        closed += 1
    }

    /// Fires a callback the way Rust does: on a queue of its own, not the main thread.
    func fireOffMainThread(_ body: @escaping @Sendable (any CoreSessionListener) -> Void) {
        let listener = self.listener
        DispatchQueue.global().async { if let listener { body(listener) } }
    }

    var isCallbackThreadMain: Bool { Thread.isMainThread }
}

private func shootData(photoCount: Int = 6, batchCount: Int = 2) -> SessionData {
    let photos = FixturePhotos.syntheticPhotos(count: photoCount, burstSize: photoCount / batchCount)
    return SessionData(
        folder: "/tmp/Game1JENKS", photos: photos, batches: FixturePhotos.batches(for: photos))
}

private final class RecordingListener: SessionListener, @unchecked Sendable {
    var batchChanges = 0
    var fileChanges = 0
    var xmpErrors: [(PhotoID, String)] = []
    var imported = 0
    /// Set on a background thread, read on the main: proves the hop happened.
    var arrivedOnMainThread: [Bool] = []

    func sessionDidChangeBatches(_ batches: [Batch]) {
        batchChanges += 1
        arrivedOnMainThread.append(Thread.isMainThread)
    }

    func sessionDidChangeFiles() {
        fileChanges += 1
        arrivedOnMainThread.append(Thread.isMainThread)
    }

    func sessionDidFailWritingXMP(photo: PhotoID, message: String) {
        xmpErrors.append((photo, message))
        arrivedOnMainThread.append(Thread.isMainThread)
    }

    func sessionDidImportRatings(_ ratings: [PhotoID: Rating]) {
        imported += 1
        arrivedOnMainThread.append(Thread.isMainThread)
    }
}

// MARK: - Tests

@Suite("Core session backend")
@MainActor
struct CoreSessionBackendTests {
    private static func makeBackend() -> (CoreSessionBackend, ScriptedCoreSession, RecordingListener) {
        let data = shootData()
        let core = ScriptedCoreSession(data)
        let backend = CoreSessionBackend(core: core, initial: data)
        let listener = RecordingListener()
        backend.listener = listener
        return (backend, core, listener)
    }

    @Test("The snapshot is taken once at open and not again for a whole cull (REV-37)")
    func snapshotIsNotTakenPerKeystroke() {
        let (backend, _, _) = Self.makeBackend()
        #expect(backend.snapshotReads == 1)

        for stars in 1...5 { _ = backend.setRating(photo: 1, Rating(stars: UInt8(stars))) }
        backend.setCursor(batch: 1, photo: 2)
        backend.markVisited(batch: 1)
        backend.flush()
        backend.submitVisualSigs([(1, VisualSig(dhash: 1, hist: []))])

        #expect(backend.snapshotReads == 1, "five keystrokes must not each copy the whole shoot")
    }

    @Test("A callback from a worker thread is delivered on the main actor")
    func callbacksHopToTheMainActor() async {
        let (backend, core, listener) = Self.makeBackend()
        let batches = backend.data.batches

        core.fireOffMainThread { $0.batchesChanged(batches) }
        core.fireOffMainThread { $0.filesChanged() }
        core.fireOffMainThread { $0.xmpError(photo: 3, message: "sidecar is not XMP") }
        core.fireOffMainThread { $0.ratingsImported([1: Rating(stars: 3)]) }

        // Give the four Tasks time to land, on the main actor, so the awaits are the ones doing
        // the work rather than the test thread happening to be the main actor already.
        for _ in 0..<50 where listener.arrivedOnMainThread.count < 4 {
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(listener.arrivedOnMainThread.count == 4)
        #expect(listener.arrivedOnMainThread.allSatisfy { $0 })
        #expect(listener.batchChanges == 1)
        #expect(listener.fileChanges == 1)
        #expect(listener.xmpErrors.count == 1)
        #expect(listener.imported == 1)
    }

    @Test("A re-batch refreshes the snapshot exactly once, and the model sees the new batches")
    func reBatchRefreshesOnce() async {
        let (backend, core, listener) = Self.makeBackend()
        // The pipeline re-batches: a provisional batch of 3 splits into 1 + 2.
        let refined: [Batch] = [
            Batch(id: 10, index: 0, photoIds: [1], provisional: false),
            Batch(id: 11, index: 1, photoIds: [2, 3], provisional: false),
        ]
        core.fireOffMainThread { $0.batchesChanged(refined) }
        for _ in 0..<50 where listener.batchChanges == 0 {
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(listener.batchChanges == 1)
        #expect(backend.snapshotReads == 2, "one refresh per batches_changed, not one per batch")
        #expect(backend.data.batches.map(\.id) == [10, 11])
    }

    @Test("The cursor is stored by id, so a re-batch cannot move the user (REV-49)")
    func cursorIsByIdNotIndex() async {
        let (backend, core, listener) = Self.makeBackend()
        backend.setCursor(batch: 2, photo: 4)
        #expect(backend.data.cursor == SessionCursor(batch: 2, photo: 4))

        // Batches 1 and 2 merge into one new batch at index 0. An index-based cursor would now
        // point at a different photo; a `BatchID` cursor is simply no longer valid, and the model
        // leaves the user where they are.
        let merged: [Batch] = [Batch(id: 99, index: 0, photoIds: [1, 2, 3, 4, 5, 6], provisional: false)]
        core.fireOffMainThread { $0.batchesChanged(merged) }
        for _ in 0..<50 where listener.batchChanges == 0 {
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(backend.data.batches.first?.id == 99)
        #expect(backend.data.cursor?.batch == 2, "the id the user was in, not the new index 0")
        // The batch the user was in no longer exists, so the model leaves the cursor alone
        // rather than reinterpreting index 0 as batch 2.
        #expect(backend.data.cursor?.batch != backend.data.batches.first?.id)
    }

    @Test("A rating lands in the core and in the backend's own copy, without another snapshot")
    func ratingWritesThrough() {
        let (backend, core, _) = Self.makeBackend()
        let change = backend.setRating(photo: 2, Rating(stars: 5))
        #expect(change.after.stars == 5)
        #expect(change.before == Rating())
        #expect(core.data.ratings[2]?.stars == 5)
        #expect(backend.data.ratings[2]?.stars == 5)
        #expect(backend.rating(for: 2).stars == 5)
        #expect(backend.snapshotReads == 1)
    }

    @Test("Undo and redo move the core and the cached copy together")
    func undoRedo() {
        let (backend, _, _) = Self.makeBackend()
        _ = backend.setRating(photo: 1, Rating(stars: 4))
        let undone = backend.undo()
        #expect(undone?.before == Rating())
        #expect(backend.rating(for: 1) == Rating())
        let redone = backend.redo()
        #expect(redone?.after.stars == 4)
        #expect(backend.rating(for: 1).stars == 4)
        #expect(backend.undo() != nil)
        #expect(backend.undo() == nil, "the history is exactly one change deep")
    }

    @Test("A failed write is reported through the listener, not swallowed and not fatal")
    func failedWriteIsReported() {
        let (backend, core, listener) = Self.makeBackend()
        core.failRating = true
        let change = backend.setRating(photo: 5, Rating(stars: 3))
        #expect(listener.xmpErrors.count == 1)
        #expect(listener.xmpErrors.first?.0 == 5)
        #expect(change.after.stars == 3, "the model keeps its own copy; the DB still has the old one")
        #expect(backend.rating(for: 5) == Rating(), "and the backend did not pretend it was saved")
    }

    @Test("Visited batches and the last photo in a batch are remembered by id")
    func visitedAndCursorMemory() {
        let (backend, core, _) = Self.makeBackend()
        backend.markVisited(batch: 3)
        backend.setCursor(batch: 3, photo: 6)
        #expect(backend.isVisited(3))
        #expect(backend.isVisited(4) == false)
        #expect(backend.lastPhotoInBatch(3) == 6)
        #expect(core.visited == [3])
        #expect(core.cursors.count == 1)
    }

    @Test("The rating mode is a session setting, and setting it is the core's business")
    func ratingModeRoundTrips() {
        let (backend, core, _) = Self.makeBackend()
        #expect(backend.ratingMode == .stars)
        backend.setRatingMode(.keep)
        core.mode = .keep
        #expect(backend.ratingMode == .keep)
        #expect(core.ratingModeSets == [.keep])
    }

    @Test("Finish goes through to the core and reports what it returned")
    func finishRoundTrips() {
        let (backend, _, _) = Self.makeBackend()
        let plan = backend.planFinish(FinishSettings())
        #expect(plan.opCount == 1)
        #expect(plan.bytesToCopy == 10)
        #expect(backend.executeFinish(plan).undoable)
        #expect(backend.undoFinish().undoable == false)
    }

    @Test("Close flushes first: the debounced XMP queue goes out on the way (todo.md §6.3)")
    func closeFlushes() {
        let (backend, core, _) = Self.makeBackend()
        backend.close()
        #expect(core.flushed >= 1)
        #expect(core.closed == 1)
    }

    @Test("The session is the real generated one, and it can run Finish")
    func whatIsReal() {
        #expect(FirstcutCoreBridge.hasSessionAPI)
        #expect(MainActor.assumeIsolated { SessionFactory.backendName }.contains("SQLite + XMP sidecars"))
        #expect(MainActor.assumeIsolated { SessionFactory.canFinish })
    }

    @Test("A folder moved since the last open is recorded, not dropped")
    func sessionMoved() async {
        let (backend, core, _) = Self.makeBackend()
        core.fireOffMainThread { $0.sessionMoved(from: "/Volumes/Old/Card") }
        for _ in 0..<50 where backend.movedFrom == nil {
            try? await Task.sleep(for: .milliseconds(10))
        }
        #expect(backend.movedFrom == "/Volumes/Old/Card")
    }
}
