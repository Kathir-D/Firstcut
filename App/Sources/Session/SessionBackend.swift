// Owner: app-logic.
//
// The seam between `AppModel` and core-store. `AppModel` never imports UniFFI types: it talks to
// this protocol, which mirrors `Session` in [session-api.md] one-for-one. `MockSession` (mock data
// from the exiftool fixtures) and the real generated `Session` both satisfy it, so nothing waits on
// the Rust side and swapping in the real one is a one-line change at construction.
//
// Note the responsibility split the contract is explicit about: **"only rate in the current batch"
// is enforced by app-logic, not here** — the backend accepts any photo, the model never offers one.

import Foundation

@MainActor public protocol SessionListener: AnyObject {
    /// Batches were re-batched after visual signatures arrived. Only *unvisited* batches can differ
    /// (batching.md), so the current batch is guaranteed to still be there under the same id.
    func sessionDidChangeBatches(_ batches: [Batch])
    /// FSEvents: files appeared or disappeared (task.md §11).
    func sessionDidChangeFiles()
    /// XMP sidecar write failed for one photo. The DB still has the rating, so this is reported,
    /// never fatal.
    func sessionDidFailWritingXMP(photo: PhotoID, message: String)
    /// Ratings imported from existing XMP sidecars because the DB was new or missing.
    func sessionDidImportRatings(_ ratings: [PhotoID: Rating])
}

@MainActor public protocol SessionBackend: AnyObject {
    var listener: (any SessionListener)? { get set }

    /// Everything the model needs to build its state, read once when a folder is opened.
    var data: SessionData { get }

    /// Current rating, for reconciling after undo/redo and for XMP import.
    func rating(for photo: PhotoID) -> Rating
    func isVisited(_ batch: BatchID) -> Bool

    /// DB now, XMP debounced ≤ 1 s. Returns both sides of the change for undo.
    @discardableResult
    func setRating(photo: PhotoID, _ rating: Rating) -> RatingChange

    func undo() -> RatingChange?
    func redo() -> RatingChange?

    func setCursor(batch: BatchID, photo: PhotoID)
    func markVisited(batch: BatchID)
    func lastPhotoInBatch(_ batch: BatchID) -> PhotoID?

    /// Thumbnails finished for some photos; may re-batch unvisited batches.
    func submitVisualSigs(_ sigs: [(PhotoID, VisualSig)])

    // Finish Cull (task.md §9.7)
    func planFinish(_ settings: FinishSettings) -> FinishPlanData
    func executeFinish(_ plan: FinishPlanData) -> FinishReportData
    func undoFinish() -> FinishReportData

    /// Settings → Metadata (task.md §9.8): whether ratings are mirrored to `.xmp` sidecars, for
    /// which files, and what a Keep is written as. Backends with no sidecars ignore it.
    func applyMetadataSettings(_ settings: MetadataSettings)

    /// Force the debounced XMP queue out. On batch change and on quit (task.md §6.3).
    func flush()

    /// True for a backend over a real folder, which can be re-read when files appear or vanish.
    var canRescan: Bool { get }

    /// Re-reads the folder and reconciles by identity, so a rename keeps its rating (REV-68).
    /// Returns the whole state of record, or nil when this backend cannot (or the read failed).
    func rescan() -> SessionData?
}

extension SessionBackend {
    public func applyMetadataSettings(_ settings: MetadataSettings) {}
    public var canRescan: Bool { false }
    public func rescan() -> SessionData? { nil }
}
