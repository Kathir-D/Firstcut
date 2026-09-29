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
// Still missing, and the app is explicit about it rather than quietly substituting something:
//   * **`plan_finish` / `execute_finish` / `undo_finish`** — no export. The Finish sheet reaches
//     `planFinish` and gets a plan with a warning, which the sheet shows verbatim. Nothing is
//     deleted, moved or copied, so the failure mode is a refused Finish rather than a wrong one.
//   * **`ratings_imported`** — the listener has no XMP-import callback. Opening a folder with
//     sidecars and no database therefore reports nothing today; the session DB is still the
//     source of truth and the ratings in it are still loaded.
//
// ## Threading
//
// `ffi.rs` documents the contract: *"Every method is called on a core thread, never the main
// thread — hop to the main actor before touching UI state."* `CoreSessionBridge` below is the one
// place that hop happens.

import Foundation
import FirstcutCore

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
    public var matched: MatchKind { MatchKind(session.matched()) }

    /// The per-tier totals for the finish summary, straight from `Rating::tier` in rating.rs.
    public func tierCounts(_ mode: RatingMode) -> [Tier: Int] {
        [Tier: Int](session.tierCounts(mode: mode.ffi))
    }

    // MARK: CoreSessionAPI

    public func setListener(_ listener: any CoreSessionListener) {
        // The listener is fixed at construction, because UniFFI takes it as a `Box<dyn
        // SessionListener>` on `Session::open`. It stays in the protocol because the seam is the
        // contract; here it can only confirm that it was handed back the one it already holds.
        precondition(
            (listener as AnyObject) === bridge,
            "UniFFI passes the listener to Session.open; it cannot be swapped afterwards")
    }

    public func snapshot() -> SessionData {
        snapshotReads += 1
        return SessionData(session.snapshot())
    }

    public func ratingMode() -> RatingMode { RatingMode(session.ratingMode()) }

    public func setRatingMode(_ mode: RatingMode) {
        try? session.setRatingMode(mode: mode.ffi)
    }

    public func setRating(photo: PhotoID, _ rating: Rating) throws -> RatingChange {
        RatingChange(try session.setRating(photo: photo, rating: rating.ffi))
    }

    public func undo() -> RatingChange? { session.undo().map(RatingChange.init) }

    public func redo() -> RatingChange? { session.redo().map(RatingChange.init) }

    public func setCursor(batch: BatchID, photo: PhotoID) {
        try? session.setCursor(cursor: SessionCursor(batch: batch, photo: photo).ffi)
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
        if !scanned.skipped.isEmpty { refreshed.skipped = scanned.skipped.map { SkippedFile(path: $0.relPath, reason: $0.reason) } }
        return refreshed
    }

    public func submitVisualSigs(_ sigs: [(PhotoID, VisualSig)]) {
        session.submitVisualSigs(sigs: sigs.map { $0.1.ffiEntry(photo: $0.0) })
    }

    public func planFinish(_ options: FinishSettings) throws -> FinishPlanData {
        throw CoreSessionUnavailable(
            missing: ["Session.planFinish", "Session.executeFinish", "Session.undoFinish"])
    }

    public func executeFinish(_ plan: FinishPlanData) throws -> FinishReportData {
        throw CoreSessionUnavailable(
            missing: ["Session.planFinish", "Session.executeFinish", "Session.undoFinish"])
    }

    public func undoFinish() throws -> FinishReportData {
        throw CoreSessionUnavailable(
            missing: ["Session.planFinish", "Session.executeFinish", "Session.undoFinish"])
    }

    public func flush() { session.flush() }

    public func close() { session.close() }
}
