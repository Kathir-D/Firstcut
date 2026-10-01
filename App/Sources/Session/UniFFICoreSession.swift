// Owner: app-logic.
//
// `CoreSessionAPI` against the real generated `FirstcutCore.Session`.
//
// This is the end of the seam described at the top of `CoreSessionBackend.swift`: the
// `#[uniffi::export]`s landed in `core/firstcut-core/src/ffi.rs` while this work was in progress,
// so this file is the adapter the seam was written for. Everything above it — the model, the
// listener hop, the snapshot discipline, the cursor — is unchanged and was written to survive the
// swap.
//
// ## What the core does and does not have
//
// Present (`SessionProtocol` in `App/Generated/FirstcutCore.swift`): `open`, `folder`, `matched`,
// `snapshot`, `ratingMode`, `setRatingMode`, `setRating`, `undo`, `redo`, `setCursor`,
// `markVisited`, `submitVisualSigs`, `tierCounts`, `rescan`, `flush`, `close`, and four listener
// callbacks.
//
// Also present: `planFinish`, `executeFinish` and `undoFinish` (Finish Cull, todo.md §9.7).
//
// Still missing, and the app is explicit about it rather than quietly substituting something:
//   * **`ratings_imported`** — the listener has no XMP-import callback. Opening a folder with
//     sidecars and no database therefore reports nothing today; the session DB is still the
//     source of truth and the ratings in it are still loaded.
//
// ## Threading
//
// `ffi.rs` documents the contract: *"Every method is called on a core thread, never the main
// thread — hop to the main actor before touching UI state."* `CoreSessionBridge` below is the one
// place that hop happens.

import FirstcutCore
import Foundation

/// The real `Session`, wrapped so the app sees `CoreSessionAPI`.
public final class UniFFICoreSession: CoreSessionAPI, @unchecked Sendable {
    private let session: Session
    /// The listener the core was constructed with. Kept so `setListener` can say, precisely, that
    /// UniFFI took the listener by value at `Session::open` and there is nothing to swap — and so
    /// the app's non-blocking open can hand the same one to the backend it builds after the hop.
    let bridge: SessionListenerBridge
    /// How many snapshots this wrapper has taken, for REV-37.
    public private(set) var snapshotReads = 0

    private init(_ session: Session, _ bridge: SessionListenerBridge) {
        self.session = session
        self.bridge = bridge
    }

    /// `Session::open`. The database lives in Application Support unless `sessionsDir` is given,
    /// which is what the tests use so they never write into a developer's real session store.
    ///
    /// Blocking and potentially slow — it scans every header in the folder — so the app calls it
    /// off the main actor and shows a loading screen while it runs.
    static func make(folder: String, listener: SessionListenerBridge, sessionsDir: String? = nil)
        throws -> UniFFICoreSession
    {
        let session: Session
        if let sessionsDir {
            session = try Session.openIn(folder: folder, sessionsDir: sessionsDir, listener: listener)
        } else {
            session = try Session.open(folder: folder, listener: listener)
        }
        return UniFFICoreSession(session, listener)
    }

    public var canonicalFolder: String { session.folder() }
    /// The generated match kind under its plain name (`CoreTypeAliases.swift`).
    public var matched: MatchKind { session.matched() }

    /// The per-tier totals for the finish summary, straight from `Rating::tier` in rating.rs.
    public func tierCounts(_ mode: RatingMode) -> [Tier: Int] {
        [Tier: Int](session.tierCounts(mode: mode.ffi))
    }

    // MARK: CoreSessionAPI

    public func setListener(_ listener: any CoreSessionListener) {
        // The listener is fixed at construction, because UniFFI takes it as a `Box<dyn
        // SessionListener>` on `Session::open` and cannot swap it afterwards. It stays in the
        // protocol because the seam is the contract.
        //
        // This used to be a `precondition`, which trapped with EXC_BREAKPOINT and killed the app on
        // launch: `AppModel.init` does `backend.listener = self`, and the bridge it hands in is a
        // different object from the one held here, so the check failed on a correct program. A
        // documented no-op is the honest behaviour — the model always gets the bridge back, because
        // the bridge is what forwards to whatever the model registered — and a trap here can only
        // ever be a false alarm about code that is working as designed.
        _ = listener
    }

    public func snapshot() -> SessionData {
        snapshotReads += 1
        return SessionData(session.snapshot())
    }

    public func ratingMode() -> RatingMode { RatingMode(session.ratingMode()) }

    public func setRatingMode(_ mode: RatingMode) {
        try? session.setRatingMode(mode: mode.ffi)
    }

    /// The core clamps to 4…5 (`clamp_keep_stars`), so anything outside that is a no-op rather
    /// than an error. Called after a folder opens and whenever the setting changes: the core holds
    /// it in memory, so a shoot opened later in the same launch would otherwise plan Finish with
    /// the default 4.
    public func setKeepThreshold(_ stars: Int) {
        session.setKeepThreshold(stars: UInt8(clamping: stars))
    }

    public func setRating(photo: PhotoID, _ rating: Rating) throws -> RatingChange {
        RatingChange(try session.setRating(photo: photo, rating: rating.ffi))
    }

    /// The core reports an undo as the transition it just made (`before` = the rating being
    /// undone, `after` = the one restored). The app's contract, like `MockSession`, is "the change
    /// that was undone", whose `before` is the value now in place, so the pair is swapped here.
    /// Without the swap the cached rating kept the undone value and ⌘Z looked like it did nothing.
    public func undo() -> RatingChange? {
        session.undo().map { ffi in
            let transition = RatingChange(ffi)
            return RatingChange(
                id: transition.id, photo: transition.photo, batch: transition.batch,
                before: transition.after, after: transition.before)
        }
    }

    public func redo() -> RatingChange? { session.redo().map(RatingChange.init) }

    public func setCursor(batch: BatchID, photo: PhotoID) {
        try? session.setCursor(cursor: SessionCursor(batch: batch, photo: photo))
    }

    public func markVisited(batch: BatchID) {
        try? session.markVisited(batch: batch)
    }

    public func rescan() throws -> SessionData {
        let scanned = try session.rescan()
        snapshotReads += 1
        var refreshed = SessionData(session.snapshot())
        // `FfiScanResult` is the rescan's own report; `snapshot()` is the state of record. Keep the
        // skipped list from the rescan when it is the more complete of the two.
        if !scanned.skipped.isEmpty {
            refreshed.skipped = scanned.skipped.map { SkippedFile(path: $0.relPath, reason: $0.reason) }
        }
        return refreshed
    }

    public func submitVisualSigs(_ sigs: [(PhotoID, VisualSig)]) {
        session.submitVisualSigs(sigs: sigs.map { $0.1.ffiEntry(photo: $0.0) })
    }

    // MARK: Finish (todo.md §9.7)

    /// The dry run. The session plans with its own rating mode, so the preview cannot disagree with
    /// the filmstrip about what is kept (REV-78); `options.ratingMode` is not sent.
    public func planFinish(_ options: FinishSettings) throws -> FinishPlanData {
        FinishPlanData(try session.planFinish(options: options.ffi))
    }

    public func executeFinish(_ plan: FinishPlanData) throws -> FinishReportData {
        FinishReportData(try session.executeFinish(plan: plan.ffi))
    }

    public func undoFinish() throws -> FinishReportData {
        FinishReportData(try session.undoFinish())
    }

    public func applyMetadataSettings(_ settings: MetadataSettings) {
        var stars: UInt8 = 5
        var label: ColorLabel?
        switch settings.keepMapping {
        case .rating(let value): stars = UInt8(min(5, max(1, value)))
        case .colorLabel(let value): label = value
        }
        try? session.setXmpSettings(
            writeSidecars: settings.writeXmp,
            sidecarsForNonRaw: settings.writeSidecarsForJpegs,
            keepStars: stars,
            keepLabel: label)
    }

    public func flush() { session.flush() }

    public func close() { session.close() }
}

// MARK: - Finish types ↔ the generated FFI types

extension FinishSettings {
    /// Without `ratingMode`: the core plans with the session's own (REV-78).
    var ffi: FfiFinishOptions { FfiFinishOptions(unkept: unkept.ffi, kept: kept.ffi) }
}

extension UnkeptAction {
    var ffi: FfiUnkeptAction {
        switch self {
        case .markRejectedInXmp: .markRejectedInXmp
        case .moveToSubfolder(let name): .moveToSubfolder(name: name)
        case .moveToTrash: .moveToTrash
        case .deletePermanently: .deletePermanently
        case .nothing: .nothing
        }
    }
}

extension KeptAction {
    var ffi: FfiKeptAction {
        switch self {
        case .none: .none
        case .copyTo(let folder): .copyTo(folder: folder)
        case .moveTo(let folder): .moveTo(folder: folder)
        case .splitByTier(let folder): .splitByTier(folder: folder)
        case .splitByStars(let folder): .splitByStars(folder: folder)
        case .writeList(let file): .writeList(file: file)
        }
    }
}

extension FileOpKind {
    init(_ ffi: FfiFileOpKind) {
        switch ffi {
        case .move: self = .move
        case .copy: self = .copy
        case .trash: self = .trash
        case .delete: self = .delete
        case .markRejected: self = .writeXmp
        case .writeList: self = .writeList
        }
    }

    var ffi: FfiFileOpKind {
        switch self {
        case .move: .move
        case .copy: .copy
        case .trash: .trash
        case .delete: .delete
        case .writeXmp: .markRejected
        case .writeList: .writeList
        }
    }
}

extension FinishPlanData {
    init(_ ffi: FfiFinishPlan) {
        self.init(
            ops: ffi.ops.map { FileOp(kind: FileOpKind($0.kind), from: $0.from, to: $0.to) },
            bytesToCopy: ffi.bytesToCopy,
            warnings: ffi.warnings)
    }

    var ffi: FfiFinishPlan {
        FfiFinishPlan(
            ops: ops.map { FfiFileOp(kind: $0.kind.ffi, from: $0.from, to: $0.to) },
            bytesToCopy: bytesToCopy,
            warnings: warnings)
    }
}

extension FinishReportData {
    init(_ ffi: FfiFinishReport) {
        self.init(
            done: Int(ffi.done),
            failed: ffi.failed,
            undoable: ffi.undoable)
    }
}
