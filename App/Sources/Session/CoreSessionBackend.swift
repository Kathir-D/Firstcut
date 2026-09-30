// Owner: app-logic.
//
// The real `SessionBackend`: app-logic's `Session` from [session-api.md] §"Session object", reached
// through `CoreBridge.swift`.
//
// ─────────────────────────────────────────────────────────────────────────────
// THE SEAM
// ─────────────────────────────────────────────────────────────────────────────
//
// Everything here is written against `CoreSessionAPI` rather than against the generated
// `FirstcutCore.Session` directly, and the reason is historical: when this file was written,
// `core/firstcut-core/src/ffi.rs` exported two functions (`hello`, `core_version`) and there was
// no `Session` on the FFI boundary at all. The conformer was a stub that threw and named the
// export it wanted, because a plausible-looking Swift wrapper for a function that does not exist
// compiles, demos green, and is wrong in a way nothing catches.
//
// The exports landed mid-task. `UniFFICoreSession` in `UniFFICoreSession.swift` is now the real
// adapter, and this file did not change to accommodate it — which is the evidence that the seam
// was in the right place. `CoreBridgeTests.exportsAreNotStale` is what keeps the two honest about
// each other.
//
// What is real here and tested, independent of what the core implements:
//   * the callback hop — Rust calls the listener on whatever thread it was working on, and every
//     callback is delivered to `SessionListener` on the main actor;
//   * **REV-37**: `snapshot()` is called once per open and once per `batches_changed`, never per
//     keystroke. `snapshotReads` counts the calls so a test can prove it;
//   * **REV-49**: the cursor is stored as a `BatchID`, never an index, so a re-batch cannot move
//     the user. Nothing in this file knows what a "current batch index" is;
//   * local state stays in step with the core so `data` is correct without another snapshot.
// ─────────────────────────────────────────────────────────────────────────────

import FirstcutCore
import Foundation

// MARK: - The Rust surface

/// core-store's `Session` as Swift will see it once `#[uniffi::export]`s exist. One method per
/// entry in [session-api.md] §"Session object"; the parameter types are the Swift mirrors in
/// `App/Sources/Session/SessionTypes.swift` and `CoreTypes.swift`, one-for-one.
///
/// The type is deliberately `Sendable`: a `Session` in Rust is shared across threads behind its own
/// interior mutability, and this backend hands it to whichever worker the pipeline is on.
public protocol CoreSessionAPI: AnyObject, Sendable {
    // `Session::open` is a **static** method, and Swift protocols cannot have static requirements,
    // so it is not on this protocol: `CoreSessionBackend.open(folder:)` calls the concrete
    // conformer directly. That one line is the whole swap.
    //
    // The listener arrives on this instead, because in Rust it is a `Box<dyn SessionListener>`
    // passed *to* `Session::open`; keeping it off the static signature is the one concession the
    // language forces, and the bridge is attached before the first call can land.

    /// Attaches the listener the core will call back on. Idempotent; the last one wins.
    func setListener(_ listener: any CoreSessionListener)

    /// `Session::snapshot`. Called once per open and once per `batches_changed` (REV-37).
    func snapshot() -> SessionData

    /// `Session::rating_mode` / `set_rating_mode`.
    func ratingMode() -> RatingMode
    func setRatingMode(_ mode: RatingMode)

    /// DB now, XMP debounced ≤ 1 s. Returns both sides of the change, which is what makes undo
    /// possible. Must return in < 1 ms.
    func setRating(photo: PhotoID, _ rating: Rating) throws -> RatingChange

    func undo() -> RatingChange?
    func redo() -> RatingChange?

    /// `Session::set_cursor` / `mark_visited`.
    func setCursor(batch: BatchID, photo: PhotoID)
    func markVisited(batch: BatchID)

    /// From the pipeline; may re-batch unvisited batches and then call `batchesChanged`.
    func submitVisualSigs(_ sigs: [(PhotoID, VisualSig)])

    /// FSEvents found files added or removed, and renames are recognised so a rating survives one
    /// (REV-68). Returns the whole state of record, not a delta.
    func rescan() throws -> SessionData

    func planFinish(_ options: FinishSettings) throws -> FinishPlanData
    func executeFinish(_ plan: FinishPlanData) throws -> FinishReportData
    func undoFinish() throws -> FinishReportData

    /// `Session::set_xmp_settings`. A core with no sidecar support ignores it.
    func applyMetadataSettings(_ settings: MetadataSettings)

    /// `Session::set_keep_stars`. Clamped to 4…5 by the core, so a bogus value is harmless.
    func setKeepThreshold(_ stars: Int)

    /// Force the debounced XMP queue out. On batch change and on quit (todo.md §6.3).
    func flush()
    func close()
}

extension CoreSessionAPI {
    public func applyMetadataSettings(_ settings: MetadataSettings) {}
}

/// The Rust `SessionListener` trait. **Every method may be called on any thread** — FSEvents fires
/// on its own queue, the XMP writer on its own, the re-batcher wherever the thumbnails finished.
public protocol CoreSessionListener: AnyObject, Sendable {
    func batchesChanged(_ batches: [Batch])
    func filesChanged()
    func xmpError(photo: PhotoID, message: String)
    /// Opening a folder with sidecars but no database imports the ratings from them.
    func ratingsImported(_ ratings: [PhotoID: Rating])
    /// The shoot was re-matched after the folder moved or was renamed ([session-api.md]'s
    /// `session_moved`). The user should be told; nothing in the app shows this yet.
    func sessionMoved(from: String)
}

// MARK: - Backend

@MainActor
public final class CoreSessionBackend: SessionBackend {
    public weak var listener: (any SessionListener)?

    /// The cached snapshot. `data` never reads through to the core: [session-api.md] promises a
    /// snapshot is a whole-shoot copy, and taking one per keystroke is REV-37.
    public private(set) var data: SessionData

    /// How many times `snapshot()` has been called. Once per open, once per `batches_changed`, and
    /// that is all — a test asserts it does not move while rating.
    public private(set) var snapshotReads = 0

    public let folder: String
    public let matched: MatchKind
    /// Set when the core re-matched this shoot after its folder moved or was renamed
    /// ([session-api.md]'s `session_moved`). Nil until it happens.
    public private(set) var movedFrom: String?

    private let core: any CoreSessionAPI
    private let bridge: SessionListenerBridge

    /// The usual path: the backend makes its own bridge and hands it to the core.
    public convenience init(core: any CoreSessionAPI, initial data: SessionData, matched: MatchKind = .created) {
        self.init(core: core, initial: data, bridge: SessionListenerBridge(), matched: matched)
    }

    /// The path the app's non-blocking open takes: the bridge already exists (UniFFI took the
    /// listener by value at `Session::open`) and only its `owner` is still unassigned.
    init(core: any CoreSessionAPI, initial data: SessionData, bridge: SessionListenerBridge,
         matched: MatchKind = .created)
    {
        self.core = core
        self.data = data
        self.snapshotReads = 1
        self.folder = data.folder
        self.matched = matched
        self.bridge = bridge
        self.bridge.owner = self
        self.core.setListener(self.bridge)
    }

    /// `Session::open`. The one place a snapshot is taken, and the one place the listener is
    /// attached.
    public static func open(folder: URL, sessionsDir: String? = nil) throws -> CoreSessionBackend {
        let bridge = SessionListenerBridge()
        let core = try UniFFICoreSession.make(
            folder: folder.path, listener: bridge, sessionsDir: sessionsDir)
        // One snapshot, at open. Nothing else may take one except a `batches_changed` (REV-37).
        return CoreSessionBackend(core: core, initial: core.snapshot(), matched: core.matched)
    }

    // MARK: Reads

    public func rating(for photo: PhotoID) -> Rating { data.ratings[photo] ?? Rating() }

    public func isVisited(_ batch: BatchID) -> Bool { data.visited.contains(batch) }

    public func lastPhotoInBatch(_ batch: BatchID) -> PhotoID? { data.lastPhotoInBatch[batch] }

    public var ratingMode: RatingMode { core.ratingMode() }

    // MARK: Writes

    @discardableResult
    public func setRating(photo: PhotoID, _ rating: Rating) -> RatingChange {
        do {
            let change = try core.setRating(photo: photo, rating)
            data.ratings[photo] = rating
            return change
        } catch {
            // session-api.md's error table: an `Io` failure leaves the rating in the database, so
            // the model keeps its own copy and the user is told, rather than the write vanishing.
            listener?.sessionDidFailWritingXMP(
                photo: photo, message: "could not save the rating: \(error.localizedDescription)")
            return RatingChange(
                id: 0, photo: photo, batch: 0, before: self.rating(for: photo), after: rating)
        }
    }

    public func undo() -> RatingChange? {
        guard let change = core.undo() else { return nil }
        data.ratings[change.photo] = change.before
        return change
    }

    public func redo() -> RatingChange? {
        guard let change = core.redo() else { return nil }
        data.ratings[change.photo] = change.after
        return change
    }

    /// Stored **by id**. A re-batch changes indices and nothing else, so a cursor held as a
    /// `BatchID` survives one and a cursor held as an index would silently move the user (REV-49).
    public func setCursor(batch: BatchID, photo: PhotoID) {
        core.setCursor(batch: batch, photo: photo)
        data.cursor = SessionCursor(batch: batch, photo: photo)
        data.lastPhotoInBatch[batch] = photo
    }

    public func markVisited(batch: BatchID) {
        core.markVisited(batch: batch)
        data.visited.insert(batch)
    }

    public func setRatingMode(_ mode: RatingMode) {
        core.setRatingMode(mode)
    }

    public func submitVisualSigs(_ sigs: [(PhotoID, VisualSig)]) {
        core.submitVisualSigs(sigs)
    }

    // MARK: Finish

    public func planFinish(_ settings: FinishSettings) -> FinishPlanData {
        do {
            return try core.planFinish(settings)
        } catch {
            return FinishPlanData(warnings: ["Finish could not be planned: \(Self.describe(error))"])
        }
    }

    public func executeFinish(_ plan: FinishPlanData) -> FinishReportData {
        do {
            return try core.executeFinish(plan)
        } catch {
            return FinishReportData(failed: [FileOpFailure(path: "", reason: Self.describe(error))])
        }
    }

    public func undoFinish() -> FinishReportData {
        do {
            return try core.undoFinish()
        } catch {
            return FinishReportData(failed: [FileOpFailure(path: "", reason: Self.describe(error))])
        }
    }

    /// The core's own message, which is shown in the Finish sheet verbatim: "cannot be undone" or
    /// "no such folder" is more use to the user than "an error occurred".
    private static func describe(_ error: Error) -> String {
        if let unavailable = error as? CoreSessionUnavailable { return unavailable.description }
        return String(describing: error)
    }

    public func applyMetadataSettings(_ settings: MetadataSettings) {
        core.applyMetadataSettings(settings)
    }

    public func setKeepThreshold(_ stars: Int) {
        core.setKeepThreshold(stars)
    }

    public var canRescan: Bool { true }

    public func rescan() -> SessionData? {
        guard let refreshed = try? core.rescan() else { return nil }
        // The state of record after reconciling renames: a new snapshot, by design (REV-37 allows
        // one per real change to the file set).
        data = refreshed
        snapshotReads += 1
        return refreshed
    }

    public func flush() { core.flush() }

    public func close() {
        // The core's `close` flushes and stops the writer itself, but the explicit `flush` keeps
        // the "on the way out, not on the way in" ordering the contract asks for.
        core.flush()
        bridge.owner = nil
        core.close()
    }

    // MARK: - Internal, called by the bridge on the main actor

    fileprivate func importRatings(_ ratings: [PhotoID: Rating]) {
        for (id, rating) in ratings { data.ratings[id] = rating }
    }

    fileprivate func noteSessionMoved(from: String) {
        movedFrom = from
    }

    fileprivate func refreshSnapshot(batches: [Batch]?) {
        var snapshot = core.snapshot()
        snapshotReads += 1
        if let batches { snapshot.batches = batches }
        data = snapshot
    }
}

/// How the session database was found ([session-api.md]'s `MatchKind`). Swift shows a note when it
/// is `.moved`.
public enum MatchKind: Equatable, Sendable {
    case created
    case exact
    case moved(from: String)
}

// MARK: - The thread hop

/// The only place a Rust callback becomes a `SessionListener` call, and the reason it is a separate
/// type: the callbacks are synchronous and land on an arbitrary thread, so each one has to hand
/// off to the main actor explicitly. Doing it here means `CoreSessionBackend` itself is a plain
/// `@MainActor` class with no threading in it, and a test can drive the hop by calling any of these
/// from a background queue.
///
/// One type satisfies both listener protocols on purpose — the seam's `CoreSessionListener` and
/// the generated `FirstcutCore.FfiSessionListener` — so the callback a test fires by hand and the
/// callback Rust fires go through the *same* hop. A test double that only implemented the seam
/// would prove nothing about the real path.
final class SessionListenerBridge: CoreSessionListener, FfiSessionListener, @unchecked Sendable {
    weak var owner: CoreSessionBackend?

    /// The UniFFI adapter: `ffi.rs` names these `on_batches_changed` etc. They are the same
    /// callbacks with a different name, so they forward rather than duplicating.
    func onBatchesChanged(batches: [FfiBatch]) { batchesChanged(batches.map(Batch.init)) }
    func onFilesChanged() { filesChanged() }
    func onXmpError(photo: UInt64, message: String) { xmpError(photo: photo, message: message) }
    func onSessionMoved(from: String) { sessionMoved(from: from) }

    func batchesChanged(_ batches: [Batch]) {
        let owner = owner
        Task { @MainActor in
            guard let owner else { return }
            // The refresh and the notification are one main-actor step, so the model can never see
            // a `sessionDidChangeBatches` carrying batches that are not the ones now in `data`.
            owner.refreshSnapshot(batches: batches)
            owner.listener?.sessionDidChangeBatches(batches)
        }
    }

    func filesChanged() {
        let owner = owner
        Task { @MainActor in
            guard let owner else { return }
            owner.refreshSnapshot(batches: nil)
            owner.listener?.sessionDidChangeFiles()
        }
    }

    func xmpError(photo: PhotoID, message: String) {
        let owner = owner
        Task { @MainActor in owner?.listener?.sessionDidFailWritingXMP(photo: photo, message: message) }
    }

    func ratingsImported(_ ratings: [PhotoID: Rating]) {
        let owner = owner
        Task { @MainActor in
            guard let owner else { return }
            owner.importRatings(ratings)
            owner.listener?.sessionDidImportRatings(ratings)
        }
    }

    /// The core reports that the shoot was re-matched after the folder moved ([session-api.md]'s
    /// `session_moved`). The `AppModel` has no note for it yet, so this is recorded on the backend
    /// and the model can read it when it grows one — it is not dropped.
    func sessionMoved(from: String) {
        let owner = owner
        Task { @MainActor in owner?.noteSessionMoved(from: from) }
    }
}

// MARK: - What the core does not have yet

/// Thrown by the three Finish entry points, which `ffi.rs` does not export yet. The message is
/// shown in the Finish sheet verbatim, so the user is told the truth ("this build cannot move your
/// files") rather than watching a progress bar that does nothing.
public struct CoreSessionUnavailable: Error, CustomStringConvertible {
    public let missing: [String]

    public var description: String {
        """
        This build of the Rust core cannot run Finish yet. \
        Not exported: \(missing.joined(separator: ", ")). Nothing has been moved or deleted.
        """
    }
}
