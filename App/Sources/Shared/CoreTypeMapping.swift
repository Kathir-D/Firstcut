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
// The same swap, half-done deliberately: `CoreTypeAliases.swift` gives the generated types their
// plain names for every type that *is* the same type, and what is left here are the conversions for
// the six session types the app genuinely owns (docs/contracts/session-api.md) plus `Rating`, whose
// defaulted initializer the app builds everywhere. The conversions are extensions on the Swift
// types rather than free functions, so the swap can continue one type at a time.
//
// Two mappings here are **not** mechanical and are the ones worth reading:
//
// * `RatingMode` — Rust spells the keep mode `KeepNotKeep` and Swift spells it `keep`. The raw
//   value in the database is `"keep"` in both ([session-api.md], `RatingMode::as_str`).
// * `Tier` — Rust's `Tier` is the output of `display_tier`, the single rating-mode mapping
//   (rating.rs). `RatingMode`/`Tier` conversion is what the *finish summary* reads, which is why
//   the app asks the core for `tierCounts(mode:)` rather than counting tiers itself.

import FirstcutCore
import Foundation

// MARK: - Rating

extension Rating {
    init(_ ffi: FfiRating) {
        // `Flag` and `ColorLabel` are the generated types under their plain names, so stars/flag/
        // label/keep move across as they are; what this conversion still adds is the *defaults*
        // (the app writes `Rating()` 35 times) and it is why `Rating` stays an app type.
        self.init(stars: ffi.stars, flag: ffi.flag, label: ffi.label, keep: ffi.keep)
    }

    var ffi: FfiRating {
        FfiRating(stars: stars, flag: flag, label: label, keep: keep)
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
            kind: ffi.kind,
            fileSize: ffi.fileSize,
            captureTime: ffi.captureTime,
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
            preview: ffi.preview,
            fullPreview: ffi.fullPreview,
            warnings: ffi.warnings)
    }
}

extension AfInfo {
    init(_ ffi: FfiAfInfo) {
        self.init(areaMode: ffi.areaMode, points: ffi.points)
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
            cursor: ffi.cursor,
            lastPhotoInBatch: ffi.lastPhotoInBatch,
            skipped: ffi.skipped.map { SkippedFile(path: $0.relPath, reason: $0.reason) })
    }
}

extension RatingChange {
    init(_ ffi: FfiChange) {
        self.init(
            id: ffi.id, photo: ffi.photo, batch: ffi.batch, before: Rating(ffi.before),
            after: Rating(ffi.after))
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
