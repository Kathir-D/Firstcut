// Owner: app-logic.
//
// Swift mirrors of the Rust session types in [session-api.md].
//
// These deliberately do *not* reuse the contract's Rust names: the UniFFI bindings will land with
// names like `Change` and `SessionSnapshot`, and a same-named duplicate in this module would turn
// that landing into a wall of ambiguity errors. The mapping is one-to-one and is listed below; when
// infra swaps the generated types in, these get deleted, not adapted (REQ-app-logic-2).
//
//   Rust              →  Swift here
//   ─────────────────────────────────────────────────────────────
//   SessionSnapshot   →  SessionData
//   Change            →  RatingChange
//   Cursor            →  SessionCursor
//   FinishOptions     →  FinishSettings
//   FinishPlan        →  FinishPlanData
//   FinishReport      →  FinishReportData
//   UnkeptAction      →  UnkeptAction        (same name, no conflict)
//   KeptAction        →  KeptAction          (same name, no conflict)
//
// Rating itself (`Rating`, `Flag`, `ColorLabel`, `RatingMode`) is *not* mirrored here: those already
// live in `App/Sources/Shared/CoreTypes.swift` with the exact names the generated types will use.

import Foundation

/// One photo's identity, rating and derived tier as the UI sees it.
public struct PhotoVM: Identifiable, Equatable, Sendable {
    public let id: PhotoID
    public let fileName: String
    public let meta: PhotoMeta
    public var rating: Rating
    public var tier: Tier
    public var isKeep: Bool

    public init(meta: PhotoMeta, rating: Rating, mode: RatingMode, keepThreshold: Int) {
        self.id = meta.id
        self.fileName = (meta.relPath as NSString).lastPathComponent
        self.meta = meta
        self.rating = rating
        self.tier = RatingRules.tier(of: rating, mode: mode, keepThreshold: keepThreshold)
        self.isKeep = RatingRules.isKeep(rating, mode: mode, keepThreshold: keepThreshold)
    }

    /// The companions that travel with the RAW on a move (task.md §8, §9.7).
    public var sidecars: [String] { meta.companions }

    public var captureDate: Date? {
        meta.captureTime.map { Date(timeIntervalSince1970: Double($0.unixMs) / 1000) }
    }

    public var fileSizeDescription: String {
        ByteCountFormatter.string(fromByteCount: Int64(meta.fileSize), countStyle: .file)
    }
}

public struct BatchVM: Identifiable, Equatable, Sendable {
    public let id: BatchID
    /// Position in the shoot, 0-based.
    public var index: Int
    public var photoIDs: [PhotoID]
    public var visited: Bool
    /// True until visual signatures have refined its boundaries (batching.md).
    public var provisional: Bool
    /// Slice of `AppModel.allPhotos` holding this batch's photos, in capture order.
    public var range: Range<Int>

    public init(
        id: BatchID, index: Int, photoIDs: [PhotoID], visited: Bool, provisional: Bool,
        range: Range<Int>
    ) {
        self.id = id
        self.index = index
        self.photoIDs = photoIDs
        self.visited = visited
        self.provisional = provisional
        self.range = range
    }

    public init(core: Batch, visited: Bool, range: Range<Int>) {
        self.init(
            id: core.id, index: Int(core.index), photoIDs: core.photoIds, visited: visited,
            provisional: core.provisional, range: range)
    }

    public var count: Int { photoIDs.count }
}

public enum ViewMode: Hashable, Sendable {
    case loupe
    case grid
    case compare(Int)  // 2, 3 or 4 up

    public var compareCount: Int? {
        if case .compare(let count) = self { return count }
        return nil
    }
}

public struct ViewerState: Hashable, Sendable {
    public var zoomed: Bool = false
    /// Where the 100% view is centred, normalized. Nil = fit.
    public var anchor: NormalizedPoint? = nil
    public var zoomLock: Bool = false
    public var backgroundGray: Double = 0.12

    public init() {}
}

public struct LoadProgress: Hashable, Sendable {
    public var title: String
    public var fraction: Double  // 0...1
    public var detail: String?

    public init(title: String, fraction: Double, detail: String? = nil) {
        self.title = title
        self.fraction = min(max(fraction, 0), 1)
        self.detail = detail
    }
}

public enum Phase: Hashable, Sendable {
    case welcome
    case loading(LoadProgress)
    case culling
    case finishing
}

/// The Finish sheet's state machine (task.md §9.7). Logic only — the sheet itself is ui's.
public enum FinishStage: Hashable, Sendable {
    case hidden
    case summary(FinishSummary)
    case options(FinishSummary, FinishSettings)
    /// Dry run done, showing the file operations before anything touches the disk.
    case dryRun(FinishSummary, FinishSettings, FinishPlanData)
    case executing(FinishSummary, FinishSettings)
    case report(FinishSummary, FinishReportData)
    /// Not undoable (permanent delete) or failed partway.
    case failed(FinishSummary, String)

    public var summary: FinishSummary? {
        switch self {
        case .hidden, .executing: nil
        case .summary(let s), .options(let s, _), .dryRun(let s, _, _), .report(let s, _),
            .failed(let s, _):
            s
        }
    }

    public var isVisible: Bool { self != .hidden }
}

// MARK: - Mirror of the Rust session API

public struct SessionCursor: Hashable, Sendable {
    public var batch: BatchID
    public var photo: PhotoID

    public init(batch: BatchID, photo: PhotoID) {
        self.batch = batch
        self.photo = photo
    }
}

/// A single rating change, with both sides of it, which is what makes undo possible.
public struct RatingChange: Equatable, Sendable {
    public var id: UInt64
    public var photo: PhotoID
    public var batch: BatchID
    public var before: Rating
    public var after: Rating

    public init(id: UInt64, photo: PhotoID, batch: BatchID, before: Rating, after: Rating) {
        self.id = id
        self.photo = photo
        self.batch = batch
        self.before = before
        self.after = after
    }
}

public struct SessionData: Equatable, Sendable {
    public var folder: String
    public var photos: [PhotoMeta]
    public var batches: [Batch]
    public var ratings: [PhotoID: Rating]
    public var visited: Set<BatchID>
    public var cursor: SessionCursor?
    public var lastPhotoInBatch: [BatchID: PhotoID]
    /// Files that couldn't be parsed, with a reason (photo-meta.md guarantees they never block).
    public var skipped: [SkippedFile]

    public init(
        folder: String,
        photos: [PhotoMeta],
        batches: [Batch],
        ratings: [PhotoID: Rating] = [:],
        visited: Set<BatchID> = [],
        cursor: SessionCursor? = nil,
        lastPhotoInBatch: [BatchID: PhotoID] = [:],
        skipped: [SkippedFile] = []
    ) {
        self.folder = folder
        self.photos = photos
        self.batches = batches
        self.ratings = ratings
        self.visited = visited
        self.cursor = cursor
        self.lastPhotoInBatch = lastPhotoInBatch
        self.skipped = skipped
    }
}

public enum UnkeptAction: Hashable, Sendable, Codable {
    case markRejectedInXmp
    case moveToSubfolder(String)
    case moveToTrash
    case deletePermanently
    case nothing

    public static let `default` = UnkeptAction.moveToSubfolder("_Not kept")

    public static var allCases: [UnkeptAction] {
        [.markRejectedInXmp, .moveToSubfolder("_Not kept"), .moveToTrash, .deletePermanently, .nothing]
    }

    public var title: String {
        switch self {
        case .markRejectedInXmp: "Mark as rejected in XMP"
        case .moveToSubfolder: "Move to a subfolder"
        case .moveToTrash: "Move to Trash"
        case .deletePermanently: "Delete permanently"
        case .nothing: "Do nothing"
        }
    }

    public var folderName: String? {
        if case .moveToSubfolder(let name) = self { return name }
        return nil
    }

    public var isDestructive: Bool { self == .deletePermanently }
}

public enum KeptAction: Hashable, Sendable, Codable {
    case none
    case copyTo(String)
    case moveTo(String)
    case splitByTier(String)
    case splitByStars(String)
    case writeList(String)

    public static let `default` = KeptAction.none

    public static var allCases: [KeptAction] {
        [.none, .copyTo(""), .moveTo(""), .splitByTier(""), .splitByStars(""), .writeList("")]
    }

    public var title: String {
        switch self {
        case .none: "Do nothing"
        case .copyTo: "Copy to a folder"
        case .moveTo: "Move to a folder"
        case .splitByTier: "Split into subfolders by tier"
        case .splitByStars: "Split into subfolders by star count"
        case .writeList: "Write a list of file names"
        }
    }

    public var folderName: String? {
        switch self {
        case .none, .writeList: nil
        case .copyTo(let f), .moveTo(let f), .splitByTier(let f), .splitByStars(let f): f
        }
    }
}

public struct FinishSettings: Hashable, Sendable, Codable {
    public var unkept: UnkeptAction
    public var kept: KeptAction
    public var ratingMode: RatingMode

    public init(unkept: UnkeptAction = .default, kept: KeptAction = .default, ratingMode: RatingMode = .stars) {
        self.unkept = unkept
        self.kept = kept
        self.ratingMode = ratingMode
    }
}

public enum FileOpKind: String, Hashable, Sendable, Codable, CaseIterable {
    case move
    case copy
    case trash
    case delete
    case writeXmp
    case writeList
}

public struct FileOp: Hashable, Sendable, Codable {
    public var kind: FileOpKind
    public var from: String
    public var to: String?

    public init(kind: FileOpKind, from: String, to: String? = nil) {
        self.kind = kind
        self.from = from
        self.to = to
    }
}

public struct FinishPlanData: Hashable, Sendable {
    public var ops: [FileOp]
    public var bytesToCopy: UInt64
    public var warnings: [String]

    public init(ops: [FileOp] = [], bytesToCopy: UInt64 = 0, warnings: [String] = []) {
        self.ops = ops
        self.bytesToCopy = bytesToCopy
        self.warnings = warnings
    }

    public var opCount: Int { ops.count }

    public var bytesToCopyDescription: String {
        bytesToCopy == 0 ? "—" : ByteCountFormatter.string(fromByteCount: Int64(bytesToCopy), countStyle: .file)
    }
}

public struct FinishReportData: Hashable, Sendable {
    public var done: Int
    public var failed: [FileOpFailure]
    public var undoable: Bool

    public init(done: Int = 0, failed: [FileOpFailure] = [], undoable: Bool = false) {
        self.done = done
        self.failed = failed
        self.undoable = undoable
    }
}

/// A file the Finish step couldn't handle. Never silently dropped: the report shows every one.
public struct FileOpFailure: Hashable, Sendable {
    public var path: String
    public var reason: String

    public init(path: String, reason: String) {
        self.path = path
        self.reason = reason
    }
}

/// A file that couldn't be parsed, kept in the filmstrip with a placeholder (task.md §8).
public struct SkippedFile: Hashable, Sendable {
    public var path: String
    public var reason: String

    public init(path: String, reason: String) {
        self.path = path
        self.reason = reason
    }
}

public enum Tier: String, Hashable, Sendable, CaseIterable {
    case keep
    case good
    case maybe
    case unrated
    case rejected

    public var title: String {
        switch self {
        case .keep: "Keep"
        case .good: "Good"
        case .maybe: "Maybe"
        case .unrated: "Unrated"
        case .rejected: "Rejected"
        }
    }

    /// What Finish does with this tier by default (task.md §9.7).
    public var countsAsKept: Bool { self == .keep }
}

/// Totals for the summary sheet and the progress HUD.
public struct FinishSummary: Hashable, Sendable {
    public var counts: [Tier: Int]
    public var unvisitedBatches: Int
    public var batchCount: Int
    public var totalPhotos: Int

    public init(counts: [Tier: Int] = [:], unvisitedBatches: Int = 0, batchCount: Int = 0, totalPhotos: Int = 0) {
        self.counts = counts
        self.unvisitedBatches = unvisitedBatches
        self.batchCount = batchCount
        self.totalPhotos = totalPhotos
    }

    public subscript(tier: Tier) -> Int { counts[tier] ?? 0 }

    public var keptCount: Int { self[.keep] }
    public var unkeptCount: Int { totalPhotos - keptCount }
    public var isComplete: Bool { unvisitedBatches == 0 }
}

public struct CullProgress: Hashable, Sendable {
    public var batchNumber: Int  // 1-based; 0 when there's no session
    public var batchCount: Int
    public var photoNumber: Int  // 1-based, within the current batch
    public var photoCount: Int
    public var photosLeftInBatch: Int
    public var batchesLeft: Int
    public var unvisitedBatches: Int
    public var counts: [Tier: Int]
    public var totalPhotos: Int
    public var ratedPhotos: Int
    /// Wall-clock time spent culling, for the HUD's "elapsed" metric. Set by the model, which owns
    /// the session clock; derived here would need a start date the value does not carry.
    public var elapsed: TimeInterval

    public init(
        batchNumber: Int = 0, batchCount: Int = 0, photoNumber: Int = 0, photoCount: Int = 0,
        photosLeftInBatch: Int = 0, batchesLeft: Int = 0, unvisitedBatches: Int = 0,
        counts: [Tier: Int] = [:], totalPhotos: Int = 0, ratedPhotos: Int = 0,
        elapsed: TimeInterval = 0
    ) {
        self.batchNumber = batchNumber
        self.batchCount = batchCount
        self.photoNumber = photoNumber
        self.photoCount = photoCount
        self.photosLeftInBatch = photosLeftInBatch
        self.batchesLeft = batchesLeft
        self.unvisitedBatches = unvisitedBatches
        self.counts = counts
        self.totalPhotos = totalPhotos
        self.ratedPhotos = ratedPhotos
        self.elapsed = elapsed
    }

    public subscript(tier: Tier) -> Int { counts[tier] ?? 0 }

    public var batchTitle: String { batchCount == 0 ? "" : "Batch \(batchNumber) of \(batchCount)" }
    public var fractionComplete: Double {
        guard totalPhotos > 0 else { return 0 }
        return Double(ratedPhotos) / Double(totalPhotos)
    }

    // Derived conveniences, so the HUD reads the model rather than recomputing (REV-53: a view
    // that recomputes a count from `batches` disagrees with the finish summary the moment the
    // two are derived differently).
    public var keeps: Int { self[.keep] }
    public var good: Int { self[.good] }
    public var maybe: Int { self[.maybe] }

    /// Photos in the **whole shoot** the user has not rated yet. This is the HUD's "unrated left".
    ///
    /// Shoot-wide, not per-batch, because `ratedPhotos` and `totalPhotos` are both shoot-wide:
    /// deriving it as `photoCount - ratedPhotos` mixed a per-batch count with a shoot-wide one and
    /// produced a nonsense number on every batch after the first.
    public var unratedPhotos: Int { max(0, totalPhotos - ratedPhotos) }
}
