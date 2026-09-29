// Owner: app-logic + infra.
//
// The translation between the generated `Ffi*` types and the Swift ones, in one place.
//
// `App/Generated/FirstcutCore.swift` is generated from `core/firstcut-core/src/ffi.rs` and is
// **git-ignored**, so its names are whatever the `#[derive(uniffi::Record)]` types are called
// there — `FfiPhotoMeta`, `FfiRating`, `FfiSessionSnapshot`, and so on. `CoreTypes.swift` is
// hand-written with the *concept* names (`PhotoMeta`, `Rating`, `SessionData`) and everything in
// `App/Sources` is written against those. This file is the only place the two vocabularies meet.
//
// REV-7's swap, made mechanical: when the generated types take the plain names, every `init`
// extension below loses its prefix and the rest of the app does not change. That is the whole
// reason the conversions are extensions on the Swift types rather than free functions sprinkled
// through the app.
//
// Two mappings here are **not** mechanical and are the ones worth reading:
//
// * `RatingMode` — Rust spells the keep mode `KeepNotKeep` and Swift spells it `keep`. The raw
//   value in the database is `"keep"` in both ([session-api.md], `RatingMode::as_str`).
// * `Tier` — Rust's `Tier` is the output of `display_tier`, the single rating-mode mapping
//   (rating.rs). `RatingMode`/`Tier` conversion is what the *finish summary* reads, which is why
//   the app asks the core for `tierCounts(mode:)` rather than counting tiers itself.

import Foundation
import FirstcutCore

// MARK: - Rating

extension Rating {
    init(_ ffi: FfiRating) {
        self.init(
            stars: ffi.stars,
            flag: Flag(ffi.flag),
            label: ffi.label.map(ColorLabel.init),
            keep: ffi.keep)
    }

    var ffi: FfiRating {
        FfiRating(stars: stars, flag: flag.ffi, label: label.map(\.ffi), keep: keep)
    }
}

extension Flag {
    init(_ ffi: FfiFlag) {
        switch ffi {
        case .none: self = .none
        case .pick: self = .pick
        case .reject: self = .reject
        }
    }

    var ffi: FfiFlag {
        switch self {
        case .none: .none
        case .pick: .pick
        case .reject: .reject
        }
    }
}

extension ColorLabel {
    init(_ ffi: FfiColorLabel) {
        switch ffi {
        case .red: self = .red
        case .yellow: self = .yellow
        case .green: self = .green
        case .blue: self = .blue
        case .purple: self = .purple
        }
    }

    var ffi: FfiColorLabel {
        switch self {
        case .red: .red
        case .yellow: .yellow
        case .green: .green
        case .blue: .blue
        case .purple: .purple
        }
    }
}

// MARK: - Modes and tiers

extension RatingMode {
    init(_ ffi: FfiRatingMode) {
        switch ffi {
        case .stars: self = .stars
        case .keepNotKeep: self = .keep
        }
    }

    var ffi: FfiRatingMode {
        switch self {
        case .stars: .stars
        case .keep: .keepNotKeep
        }
    }
}

extension Tier {
    init(_ ffi: FfiTier) {
        switch ffi {
        case .keep: self = .keep
        case .good: self = .good
        case .maybe: self = .maybe
        case .unrated: self = .unrated
        case .rejected: self = .rejected
        }
    }

    var ffi: FfiTier {
        switch self {
        case .keep: .keep
        case .good: .good
        case .maybe: .maybe
        case .unrated: .unrated
        case .rejected: .rejected
        }
    }
}

// MARK: - Photo metadata

extension PhotoMeta {
    init(_ ffi: FfiPhotoMeta) {
        self.init(
            id: ffi.id,
            relPath: ffi.relPath,
            companions: ffi.companions,
            kind: FileKind(ffi.kind),
            fileSize: ffi.fileSize,
            captureTime: ffi.captureTime.map(CaptureTime.init),
            shutterCount: ffi.shutterCount,
            fileNumber: ffi.fileNumber,
            cameraMake: ffi.cameraMake,
            cameraModel: ffi.cameraModel,
            cameraSerial: ffi.cameraSerial,
            lensModel: ffi.lensModel,
            focalLengthMm: ffi.focalLengthMm,
            exposureTimeS: ffi.exposureTimeS,
            fNumber: ffi.fNumber,
            iso: ffi.iso,
            exposureCompEv: ffi.exposureCompEv,
            meteringMode: ffi.meteringMode,
            driveMode: ffi.driveMode,
            shutterMode: ffi.shutterMode,
            orientation: ffi.orientation,
            width: ffi.width,
            height: ffi.height,
            af: ffi.af.map(AfInfo.init),
            preview: ffi.preview.map(EmbeddedPreview.init),
            warnings: ffi.warnings)
    }
}

extension FileKind {
    init(_ ffi: FfiFileKind) {
        switch ffi {
        case .raw(let format): self = .raw(RawFormat(format))
        case .jpeg: self = .jpeg
        case .heif: self = .heif
        case .tiff: self = .tiff
        case .png: self = .png
        }
    }
}

extension RawFormat {
    init(_ ffi: FfiRawFormat) {
        switch ffi {
        case .cr3: self = .cr3
        case .cr2: self = .cr2
        case .crw: self = .crw
        case .arw: self = .arw
        case .sr2: self = .sr2
        case .srf: self = .srf
        case .nef: self = .nef
        case .nrw: self = .nrw
        case .raf: self = .raf
        case .rw2: self = .rw2
        case .orf: self = .orf
        case .pef: self = .pef
        case .dng: self = .dng
        case .rwl: self = .rwl
        case .threeFr: self = .threeFr
        case .fff: self = .fff
        case .iiq: self = .iiq
        case .srw: self = .srw
        case .dcr: self = .dcr
        case .kdc: self = .kdc
        case .erf: self = .erf
        case .mef: self = .mef
        case .mos: self = .mos
        case .gpr: self = .gpr
        case .x3f: self = .x3f
        }
    }
}

extension CaptureTime {
    init(_ ffi: FfiCaptureTime) {
        self.init(
            unixMs: ffi.unixMs,
            subsecResolutionMs: ffi.subsecResolutionMs,
            offsetMinutes: ffi.offsetMinutes,
            source: TimeSource(ffi.source))
    }
}

extension TimeSource {
    init(_ ffi: FfiTimeSource) {
        switch ffi {
        case .exif: self = .exif
        case .fileModified: self = .fileModified
        }
    }
}

extension EmbeddedPreview {
    init(_ ffi: FfiEmbeddedPreview) {
        self.init(range: ByteRange(ffi.range), width: ffi.width, height: ffi.height)
    }
}

extension ByteRange {
    init(_ ffi: FfiByteRange) {
        self.init(offset: ffi.offset, len: ffi.len)
    }
}

extension AfInfo {
    init(_ ffi: FfiAfInfo) {
        self.init(areaMode: ffi.areaMode, points: ffi.points.map(AfPoint.init))
    }
}

extension AfPoint {
    init(_ ffi: FfiAfPoint) {
        self.init(x: ffi.x, y: ffi.y, w: ffi.w, h: ffi.h, inFocus: ffi.inFocus)
    }
}

// MARK: - Session state

extension Batch {
    init(_ ffi: FfiBatch) {
        self.init(id: ffi.id, index: ffi.index, photoIds: ffi.photoIds, provisional: ffi.provisional)
    }
}

extension VisualSig {
    init(_ entry: FfiVisualSigEntry) {
        // Rust's `hist` is a packed 48-byte array (16 bins each for R, G, B), and the Swift type
        // is `[UInt8]`, so it only needs unwrapping.
        self.init(dhash: entry.dhash, hist: [UInt8](entry.hist))
    }

    /// The generated record carries the photo id, because Rust's `submit_visual_sigs` takes a
    /// `Vec<(PhotoId, VisualSig)>` and the tuple has nowhere to live on the Swift side.
    func ffiEntry(photo: PhotoID) -> FfiVisualSigEntry {
        FfiVisualSigEntry(photo: photo, dhash: dhash, hist: Data(hist))
    }
}

extension SessionData {
    init(_ ffi: FfiSessionSnapshot) {
        self.init(
            folder: ffi.folder,
            photos: ffi.photos.map(PhotoMeta.init),
            batches: ffi.batches.map(Batch.init),
            ratings: ffi.ratings.mapValues(Rating.init),
            visited: Set(ffi.visited),
            cursor: ffi.cursor.map(SessionCursor.init),
            lastPhotoInBatch: ffi.lastPhotoInBatch,
            skipped: ffi.skipped.map { SkippedFile(path: $0.relPath, reason: $0.reason) })
    }
}

extension SessionCursor {
    init(_ ffi: FfiCursor) {
        self.init(batch: ffi.batch, photo: ffi.photo)
    }

    var ffi: FfiCursor { FfiCursor(batch: batch, photo: photo) }
}

extension RatingChange {
    init(_ ffi: FfiChange) {
        self.init(
            id: ffi.id, photo: ffi.photo, batch: ffi.batch, before: Rating(ffi.before),
            after: Rating(ffi.after))
    }
}

extension MatchKind {
    init(_ ffi: FfiMatchKind) {
        switch ffi {
        case .created: self = .created
        case .exact: self = .exact
        case .moved(let from): self = .moved(from: from)
        }
    }
}

extension Dictionary where Key == Tier, Value == Int {
    /// `Session::tier_counts` — the per-tier totals for the finish summary, produced by the one
    /// `display_tier` in rating.rs. The app asks for these rather than counting itself, which is
    /// REV-69's point: the number in the summary and the ring on the filmstrip come from one
    /// function.
    init(_ counts: [FfiTier: UInt32]) {
        var result = [Tier: Int](minimumCapacity: counts.count)
        for (tier, count) in counts { result[Tier(tier)] = Int(count) }
        self = result
    }
}
