// Owner: app-logic.
//
// In-memory `SessionBackend` for previews and tests, built from the exiftool fixtures
// ([session-api.md]'s "Mock" section). The real `Session` is core-store's; this exists so nobody
// waits for it, and so the model can be exercised deterministically — same fixtures in, same state
// out, every run.
//
// It deliberately never touches the filesystem: `executeFinish` records what *would* have happened
// and returns a report. Real file operations are core-store's, behind a real Session.

import Foundation

@MainActor public final class MockSession: SessionBackend {
    public weak var listener: (any SessionListener)?

    public private(set) var data: SessionData

    private var ratings: [PhotoID: Rating]
    private var batchByPhoto: [PhotoID: BatchID] = [:]
    private var undoStack: [RatingChange] = []
    private var redoStack: [RatingChange] = []
    private var nextChangeID: UInt64 = 1
    private var index = 0

    /// Test hooks. `xmpWrites` is the *pending* queue that `flush()` drains; `xmpWriteCount` is the
    /// running total, so a test can still count writes across a batch change.
    public private(set) var xmpWrites: [PhotoID] = []
    public private(set) var xmpWriteCount = 0
    public private(set) var flushCount = 0
    public private(set) var executedPlans: [FinishPlanData] = []
    public private(set) var undoneFinishes = 0
    public private(set) var cursorHistory: [SessionCursor] = []
    public var visitedOverride: Set<BatchID> = []
    /// What the next `rescan()` returns, so a test can play "files appeared / vanished". A mock
    /// with nothing queued behaves like a backend that cannot rescan.
    public var nextRescan: SessionData?

    public var canRescan: Bool { nextRescan != nil }

    public func rescan() -> SessionData? {
        guard let fresh = nextRescan else { return nil }
        data = fresh
        return fresh
    }

    public init(data: SessionData) {
        self.data = data
        self.ratings = data.ratings
        for batch in data.batches {
            for id in batch.photoIds { batchByPhoto[id] = batch.id }
        }
    }

    public convenience init(photos: [PhotoMeta]) {
        self.init(
            data: SessionData(
                folder: "/mock", photos: photos, batches: FixturePhotos.batches(for: photos)))
    }

    /// A provisional session for a real folder: file name, size and modification time only.
    ///
    /// core-meta's `scan_folder` replaces this the moment its FFI lands — it reads the headers and
    /// gives real capture times, shutter counts and previews (REQ-app-logic-3). Until then this is
    /// enough to open a folder and exercise every rule with real file names, which is what makes
    /// `AppModel.open(folder:)` usable in the app at all.
    public static func dataForFolder(_ url: URL) throws -> SessionData {
        let keys: [URLResourceKey] = [.fileSizeKey, .contentModificationDateKey, .isRegularFileKey]
        let contents = try FileManager.default.contentsOfDirectory(
            at: url, includingPropertiesForKeys: keys, options: [.skipsHiddenFiles])

        var photos: [PhotoMeta] = []
        var skipped: [SkippedFile] = []
        for file in contents.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
            let values = try? file.resourceValues(forKeys: Set(keys))
            guard values?.isRegularFile == true else { continue }
            let name = file.lastPathComponent
            let isSidecar = name.lowercased().hasSuffix(".xmp")
            guard !isSidecar, isPhotoExtension(name) else {
                if isSidecar, !photos.isEmpty { attach(companion: name, to: &photos) }
                continue
            }
            let meta = PhotoMeta(
                id: FixturePhotos.stableID(name),
                relPath: name,
                companions: [],
                kind: ExifRow.fileKind(name),
                fileSize: UInt64(values?.fileSize ?? 0),
                captureTime: (values?.contentModificationDate).map {
                    CaptureTime(
                        unixMs: Int64($0.timeIntervalSince1970 * 1000), subsecResolutionMs: 1000,
                        offsetMinutes: nil, source: .fileModified)
                },
                shutterCount: nil,
                fileNumber: nil,
                cameraMake: nil,
                cameraModel: nil,
                cameraSerial: nil,
                lensModel: nil,
                focalLengthMm: nil,
                exposureTimeS: nil,
                fNumber: nil,
                iso: nil,
                exposureCompEv: nil,
                meteringMode: nil,
                driveMode: nil,
                shutterMode: nil,
                orientation: 1,
                width: 0,
                height: 0,
                af: nil,
                preview: nil,
                warnings: ["metadata parsed from the file system; core-meta's header parser not linked yet"])
            photos.append(meta)
        }

        if photos.isEmpty {
            throw CocoaError(.fileReadCorruptFile, userInfo: [NSLocalizedDescriptionKey: "No photos found in \(url.lastPathComponent)"])
        }
        return SessionData(
            folder: url.path, photos: photos, batches: FixturePhotos.batches(for: photos), skipped: skipped)
    }

    static func isPhotoExtension(_ name: String) -> Bool {
        switch (name as NSString).pathExtension.lowercased() {
        case "cr3", "cr2", "crw", "arw", "sr2", "srf", "nef", "nrw", "raf", "rw2", "orf", "pef",
            "dng", "rwl", "3fr", "fff", "iiq", "srw", "dcr", "kdc", "erf", "mef", "mos", "gpr",
            "x3f", "jpg", "jpeg", "heic", "heif", "hif", "tif", "tiff", "png":
            true
        default:
            false
        }
    }

    /// A `.xmp` or paired JPEG belongs to the RAW with the same base name (task.md §8).
    private static func attach(companion name: String, to photos: inout [PhotoMeta]) {
        let base = (name as NSString).deletingPathExtension
        guard let index = photos.lastIndex(where: { ($0.relPath as NSString).deletingPathExtension == base })
        else { return }
        photos[index].companions.append(name)
    }

    public func rating(for photo: PhotoID) -> Rating { ratings[photo] ?? Rating() }

    public func isVisited(_ batch: BatchID) -> Bool {
        visitedOverride.contains(batch) || data.visited.contains(batch)
    }

    public func batchID(for photo: PhotoID) -> BatchID? { batchByPhoto[photo] }

    public var canUndo: Bool { !undoStack.isEmpty }
    public var canRedo: Bool { !redoStack.isEmpty }

    @discardableResult
    public func setRating(photo: PhotoID, _ rating: Rating) -> RatingChange {
        let change = RatingChange(
            id: nextChangeID,
            photo: photo,
            batch: batchByPhoto[photo] ?? 0,
            before: ratings[photo] ?? Rating(),
            after: rating)
        nextChangeID += 1
        ratings[photo] = rating
        data.ratings[photo] = rating
        undoStack.append(change)
        redoStack.removeAll()
        xmpWrites.append(photo)  // stands in for the debounced ≤ 1 s XMP queue
        xmpWriteCount += 1
        return change
    }

    public func undo() -> RatingChange? {
        guard let change = undoStack.popLast() else { return nil }
        ratings[change.photo] = change.before
        data.ratings[change.photo] = change.before
        redoStack.append(change)
        xmpWrites.append(change.photo)
        xmpWriteCount += 1
        return change
    }

    public func redo() -> RatingChange? {
        guard let change = redoStack.popLast() else { return nil }
        ratings[change.photo] = change.after
        data.ratings[change.photo] = change.after
        undoStack.append(change)
        xmpWrites.append(change.photo)
        xmpWriteCount += 1
        return change
    }

    public func setCursor(batch: BatchID, photo: PhotoID) {
        data.cursor = SessionCursor(batch: batch, photo: photo)
        data.lastPhotoInBatch[batch] = photo
        cursorHistory.append(SessionCursor(batch: batch, photo: photo))
    }

    public func markVisited(batch: BatchID) {
        data.visited.insert(batch)
    }

    public func lastPhotoInBatch(_ batch: BatchID) -> PhotoID? { data.lastPhotoInBatch[batch] }

    public func flush() {
        flushCount += 1
        xmpWrites.removeAll()
    }

    /// Re-batches unvisited batches using the visual signatures, the way the real two-phase batcher
    /// does: a boundary is only *added*, never removed, and only where the timing was ambiguous
    /// (0.2 s … 2 s), and never inside a batch the user has already been in (task.md §5.4).
    public func submitVisualSigs(_ sigs: [(PhotoID, VisualSig)]) {
        let hashes = Dictionary(sigs.map { ($0.0, $0.1.dhash) }, uniquingKeysWith: { first, _ in first })
        guard !hashes.isEmpty else { return }
        let times = Dictionary(
            data.photos.map { ($0.id, $0.captureTime?.unixMs ?? -1) }, uniquingKeysWith: { first, _ in first })

        var rebuilt: [Batch] = []
        for batch in data.batches {
            guard !isVisited(batch.id) else {
                rebuilt.append(batch)
                continue
            }
            var pieces: [[PhotoID]] = [[batch.photoIds[0]]]
            for index in 1..<batch.photoIds.count {
                let previous = batch.photoIds[index - 1]
                let current = batch.photoIds[index]
                let gap = (times[current] ?? 0) - (times[previous] ?? 0)
                let ambiguous = gap >= 200 && gap <= 2000
                let distance = MockSession.hamming(hashes[previous], hashes[current])
                if ambiguous, distance >= 24 {
                    pieces.append([current])
                } else {
                    pieces[pieces.count - 1].append(current)
                }
            }
            for piece in pieces {
                rebuilt.append(
                    Batch(
                        id: FixturePhotos.stableID("batch-\(piece[0])"),
                        index: UInt32(rebuilt.count),
                        photoIds: piece,
                        provisional: false))
            }
        }
        guard rebuilt != data.batches else { return }
        data.batches = rebuilt
        batchByPhoto.removeAll(keepingCapacity: true)
        for batch in rebuilt {
            for id in batch.photoIds { batchByPhoto[id] = batch.id }
        }
        listener?.sessionDidChangeBatches(rebuilt)
    }

    static func hamming(_ a: UInt64?, _ b: UInt64?) -> Int {
        guard let a, let b else { return 0 }
        return (a ^ b).nonzeroBitCount
    }

    // MARK: - Finish

    /// A dry run over the ratings it currently holds. Real planning (free space, collisions,
    /// read-only volumes) is core-store's; this produces the same shape so the flow is testable.
    public func planFinish(_ settings: FinishSettings) -> FinishPlanData {
        let plan = FinishPlanner(
            photos: data.photos,
            ratings: ratings,
            settings: settings,
            keepThreshold: RatingRules.defaultKeepThreshold
        ).plan()
        return plan
    }

    public func executeFinish(_ plan: FinishPlanData) -> FinishReportData {
        executedPlans.append(plan)
        let destructive = plan.ops.contains { $0.kind == .delete }
        return FinishReportData(done: plan.ops.count, failed: [], undoable: !destructive)
    }

    public func undoFinish() -> FinishReportData {
        undoneFinishes += 1
        let ops = executedPlans.last?.ops.count ?? 0
        return FinishReportData(done: ops, failed: [], undoable: false)
    }
}

/// Turns a session's ratings into the file operations Finish would perform (task.md §9.7).
/// Split out from `MockSession` because qa and ui want to test the *plan* on its own.
public struct FinishPlanner {
    public let photos: [PhotoMeta]
    public let ratings: [PhotoID: Rating]
    public let settings: FinishSettings
    public let keepThreshold: Int

    public init(
        photos: [PhotoMeta], ratings: [PhotoID: Rating], settings: FinishSettings,
        keepThreshold: Int = RatingRules.defaultKeepThreshold
    ) {
        self.photos = photos
        self.ratings = ratings
        self.settings = settings
        self.keepThreshold = keepThreshold
    }

    public func plan() -> FinishPlanData {
        var ops: [FileOp] = []
        var warnings: [String] = []
        var bytes: UInt64 = 0
        let base = photos.first.map { ($0.relPath as NSString).deletingLastPathComponent } ?? ""

        for photo in photos {
            let rating = ratings[photo.id] ?? Rating()
            let isKept = RatingRules.isKeep(rating, mode: settings.ratingMode, keepThreshold: keepThreshold)
            let group = [photo.relPath] + photo.companions  // RAW + JPEG/HEIF + .xmp travel together

            if isKept {
                switch settings.kept {
                case .copyTo(let folder), .moveTo(let folder), .splitByTier(let folder),
                    .splitByStars(let folder):
                    for file in group {
                        ops.append(
                            FileOp(
                                kind: settings.kept.isCopy ? .copy : .move,
                                from: join(base, file),
                                to: join(folder, (file as NSString).lastPathComponent)))
                        bytes += photo.fileSize
                    }
                case .writeList(let path):
                    ops.append(FileOp(kind: .writeList, from: (photo.relPath as NSString).lastPathComponent, to: join(base, path)))
                case .none:
                    break
                }
            } else {
                switch settings.unkept {
                case .nothing:
                    break
                case .markRejectedInXmp:
                    for file in group {
                        ops.append(FileOp(kind: .writeXmp, from: join(base, file), to: nil))
                    }
                case .moveToSubfolder(let folder):
                    for file in group {
                        ops.append(
                            FileOp(
                                kind: .move,
                                from: join(base, file),
                                to: join(folder, (file as NSString).lastPathComponent)))
                    }
                case .moveToTrash:
                    for file in group { ops.append(FileOp(kind: .trash, from: join(base, file), to: nil)) }
                case .deletePermanently:
                    for file in group { ops.append(FileOp(kind: .delete, from: join(base, file), to: nil)) }
                }
            }
        }

        if settings.unkept == .moveToTrash, ops.contains(where: { $0.kind == .trash }) {
            warnings.append("Trash is recoverable in Finder, but Firstcut can't undo it.")
        }
        if settings.unkept == .deletePermanently {
            warnings.append("Permanent delete cannot be undone.")
        }
        if settings.kept.isCopy, bytes > 0 {
            warnings.append("Copies need free space at the destination; Firstcut checks before copying.")
        }
        return FinishPlanData(ops: ops, bytesToCopy: bytes, warnings: warnings)
    }

    private func join(_ folder: String, _ file: String) -> String {
        folder.isEmpty ? file : folder + "/" + file
    }
}

extension KeptAction {
    /// Copy vs move decides the FileOp kind; everything else that touches files is a move.
    public var isCopy: Bool {
        if case .copyTo = self { return true }
        return false
    }
}
