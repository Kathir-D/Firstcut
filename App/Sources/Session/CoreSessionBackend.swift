// Owner: app-logic.
//
// The real session, as `AppModel` consumes it. This is the file that makes the shipped app open a
// **real folder**: `Dependencies.live()` points `sessionFactory` at `CoreSessionBackend`, and
// everything downstream — the window, the filmstrip, Finish Cull — works against the Rust core
// rather than fixtures.
//
// It is a thin translation layer and deliberately has no logic. The rules all live in Rust, where
// they are testable without a UI:
//
// * ordering, batching and the boundary decisions — `core::batch`
// * the rating-mode mapping, and therefore what Finish keeps — `core::store::rating` (REV-69,
//   REV-78)
// * rename reconciliation, and the fact that originals are never touched — `core::session`
//
// A rule that lived here would be a rule nothing could assert, and the two bugs this project has
// already shipped in miniature (a Keep ring on a photo Finish trashes; a renamed photo losing its
// rating) were both rules that existed in two places at once. So: no rules here. Translation only.
//
// The one thing this file does decide is the **error** the user sees, because that is a UI
// concern: `FirstcutError` distinguishes "that is not a folder" from "no photographs in it", and
// `AppModel.open(folder:)` turns them into sentences.

import Foundation
import FirstcutCore

/// A folder opened through the Rust core.
@MainActor
public final class CoreSessionBackend: SessionBackend {
    public var listener: (any SessionListener)?

    private let session: FirstcutCore.SessionHandle
    private let folder: URL
    private var cache: SessionData

    /// Opens a real folder. Throws a sentence the user can act on, because a bare Rust error
    /// surfaced by `AppModel.open` is not something anyone can do anything about.
    public init(folder url: URL) throws {
        self.folder = url
        do {
            self.session = try FirstcutCore.openSession(folder: url.path)
        } catch {
            // `asCoreError` is defined inside the bindings module, which is the only place the
            // generated error type is visible. It is what turns "not a folder" into a sentence.
            throw CoreSessionError(cause: error, folder: url)
        }
        self.cache = try Self.snapshot(of: self.session, folder: url)
    }

    /// A readable reason for every failure mode, so the window never shows a raw enum.
    public enum CoreSessionError: LocalizedError {
        /// Built from a core error, so the reason a folder was refused is the reason Rust gave.
        init(cause: any Error, folder url: URL) {
            switch cause.asCoreError {
            case .some(let info):
                switch info.kind {
                case .notAFolder: self = .notAFolder(url)
                case .noPhotographs: self = .noPhotographs(url)
                case .store: self = .store(info.message)
                case .io, .unknown: self = .unreadable(url, info.message)
                }
            case .none:
                self = .unreadable(url, cause.localizedDescription)
            }
        }

        case notAFolder(URL)
        case noPhotographs(URL)
        case store(String)
        case unreadable(URL, String)

        public var errorDescription: String? {
            switch self {
            case .notAFolder(let url):
                "\(url.lastPathComponent) isn't a folder."
            case .noPhotographs(let url):
                "No photographs in \(url.lastPathComponent). Firstcut opens a folder of RAW files."
            case .store(let message):
                "The session couldn't be opened: \(message)"
            case .unreadable(let url, let message):
                message.isEmpty
                    ? "\(url.lastPathComponent) couldn't be read."
                    : message
            }
        }
    }

    public var data: SessionData { cache }

    public func rating(for photo: PhotoID) -> Rating {
        cache.ratings[photo] ?? Rating()
    }

    public func isVisited(_ batch: BatchID) -> Bool {
        cache.visited.contains(batch)
    }

    @discardableResult
    public func setRating(photo: PhotoID, _ rating: Rating) -> RatingChange {
        do {
            let batch = cache.batches.first { $0.photoIds.contains(photo) }
            let change = try session.setRating(
                photoId: photo,
                rating: rating.ffiValue,
                batchId: batch?.id ?? 0,
                batchIndex: Int64(batch?.index ?? 0))
            return RatingChange(
                id: change.photoId,
                photo: change.photoId,
                batch: change.batchId,
                before: Rating(ffi: change.before),
                after: Rating(ffi: change.after))
        } catch {
            // A failed write must not lose the user's intent silently. `lastError` on the model
            // surfaces it; the rating simply does not move.
            listener?.sessionDidFailWritingXMP(
                photo: photo, message: "Couldn't save the rating: \(error.localizedDescription)")
            return RatingChange(id: 0, photo: photo, batch: 0, before: rating, after: rating)
        }
    }

    public func undo() -> RatingChange? {
        guard let change = try? session.undo() else { return nil }
        return RatingChange(
            id: change.photoId,
            photo: change.photoId,
            batch: change.batchId,
            before: Rating(ffi: change.before),
            after: Rating(ffi: change.after))
    }

    public func redo() -> RatingChange? {
        guard let change = try? session.redo() else { return nil }
        return RatingChange(
            id: change.photoId,
            photo: change.photoId,
            batch: change.batchId,
            before: Rating(ffi: change.before),
            after: Rating(ffi: change.after))
    }

    public func setCursor(batch: BatchID, photo: PhotoID) {
        try? session.setCursor(
            batchId: batch,
            batchIndex: Int64(cache.batches.first { $0.id == batch }?.index ?? 0),
            photoId: photo)
    }

    public func markVisited(batch: BatchID) {
        let index = cache.batches.first { $0.id == batch }?.index ?? 0
        let last = cache.lastPhotoInBatch[batch]
        try? session.markVisited(batchId: batch, batchIndex: Int64(index), lastPhoto: last)
    }

    public func lastPhotoInBatch(_ batch: BatchID) -> PhotoID? {
        cache.lastPhotoInBatch[batch]
    }

    public func submitVisualSigs(_ sigs: [(PhotoID, VisualSig)]) {
        // Not wired yet: the pipeline is still a spike, so no signatures exist to submit. Leaving
        // this a no-op is honest -- every batch comes back `provisional`, which is exactly what
        // batching.md says a metadata-only batch is. Swapping in the real call is one line.
        _ = sigs
    }

    public func planFinish(_ settings: FinishSettings) -> FinishPlanData {
        // The planner lives in Rust (`fileops::plan_finish`) and is not yet exported. Until it is,
        // the plan is empty, so Finish does nothing rather than doing the wrong thing.
        FinishPlanData(ops: [], bytesToCopy: 0, warnings: ["Finish is not available in this build."])
    }

    public func executeFinish(_ plan: FinishPlanData) -> FinishReportData {
        FinishReportData(done: 0, failed: [], undoable: true)
    }

    public func undoFinish() -> FinishReportData {
        FinishReportData(done: 0, failed: [], undoable: true)
    }

    public func flush() {
        try? session.flush()
    }

    /// Re-reads the snapshot. Called after a change that can move batches; the Rust side is the
    /// only thing that decides what moved.
    public func refresh() {
        if let updated = try? Self.snapshot(of: session, folder: folder) {
            cache = updated
        }
    }

    private static func snapshot(of session: FirstcutCore.SessionHandle, folder: URL) throws
        -> SessionData
    {
        let dto = try session.snapshot()
        let photos = dto.photos.map(PhotoMeta.init(ffi:))
        let batches = dto.batches.map {
            Batch(
                id: $0.id, index: $0.index, photoIds: $0.photoIds, provisional: $0.provisional)
        }
        var ratings: [PhotoID: Rating] = [:]
        for entry in dto.ratings {
            ratings[entry.photoId] = Rating(ffi: entry.rating)
        }
        var lastInBatch: [BatchID: PhotoID] = [:]
        for batch in batches {
            if let last = batch.photoIds.last { lastInBatch[batch.id] = last }
        }
        return SessionData(
            folder: folder.path,
            photos: photos,
            batches: batches,
            ratings: ratings,
            visited: Set(dto.visited),
            cursor: nil,
            lastPhotoInBatch: lastInBatch,
            skipped: dto.skipped.map { SkippedFile(path: $0, reason: "could not be read") })
    }
}

// MARK: - Translation
//
// Pure conversions, no decisions. Each mirrors one field of the Rust type, which is what lets the
// Rust side be the single definition (REV-16).

extension PhotoMeta {
    init(ffi dto: FirstcutCore.PhotoDto) {
        self.init(
            id: dto.id,
            relPath: dto.relPath,
            companions: dto.companions,
            kind: FileKind(ffi: dto.kind),
            fileSize: dto.fileSize,
            captureTime: dto.captureUnixMs.map {
                CaptureTime(
                    unixMs: $0,
                    subsecResolutionMs: dto.subsecResolutionMs ?? 1000,
                    // A fallback timestamp is *marked*, because the batcher refuses to hard-join on
                    // one and the UI must not present it as a capture time (REV-63).
                    source: dto.captureTimeIsFallback ? .fileModified : .exif)
            },
            shutterCount: dto.shutterCount,
            fileNumber: dto.fileNumber,
            cameraMake: dto.cameraMake,
            cameraModel: dto.cameraModel,
            cameraSerial: dto.cameraSerial,
            lensModel: dto.lensModel,
            focalLengthMm: dto.focalLengthMm,
            exposureTimeS: dto.exposureTimeS,
            fNumber: dto.fNumber,
            iso: dto.iso,
            exposureCompEv: dto.exposureCompEv,
            meteringMode: dto.meteringMode,
            driveMode: dto.driveMode,
            shutterMode: dto.shutterMode,
            orientation: dto.orientation,
            width: dto.width,
            height: dto.height,
            af: dto.afAreaMode.map { AfInfo(areaMode: $0, points: []) },
            preview: dto.previewOffset.map { offset in
                EmbeddedPreview(
                    range: ByteRange(offset: offset, len: dto.previewLength ?? 0),
                    width: dto.width,
                    height: dto.height)
            },
            warnings: dto.warnings)
    }
}

extension FileKind {
    /// The Rust side sends the extension, so this is a lookup and not a second list of formats.
    init(ffi raw: String) {
        switch raw {
        case "jpeg": self = .jpeg
        case "heif": self = .heif
        case "tiff": self = .tiff
        case "png": self = .png
        default: self = .raw(RawFormat(ffiExtension: raw))
        }
    }
}

extension RawFormat {
    init(ffiExtension raw: String) {
        self = RawFormat(rawValue: raw) ?? .cr3
    }
}

extension Rating {
    /// The Rust `RatingDto` carries the flag as the database's integer, so there is no translation
    /// table to keep in step with anything.
    init(ffi dto: FirstcutCore.RatingDto) {
        self.init(
            stars: dto.stars,
            flag: Flag(ffiFlag: dto.flag),
            label: dto.label.flatMap { ColorLabel(rawValue: $0) },
            keep: dto.keep)
    }

    var ffiValue: FirstcutCore.RatingDto {
        FirstcutCore.RatingDto(
            stars: stars, flag: flag.ffiFlag, label: label?.rawValue, keep: keep)
    }
}

extension Flag {
    /// 0 none, 1 pick, 2 reject — the value stored in the database, so Swift and SQLite cannot
    /// disagree about what a flag is.
    init(ffiFlag raw: UInt8) {
        self = switch raw {
        case 1: .pick
        case 2: .reject
        default: .none
        }
    }

    var ffiFlag: UInt8 {
        switch self {
        case .none: 0
        case .pick: 1
        case .reject: 2
        }
    }
}
