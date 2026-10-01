// Owner: infra + app-logic.
//
// The app's core types. **Most of what used to be hand-mirrored here is now the generated
// `FirstcutCore` type under the plain name** — see `CoreTypeAliases.swift`, which is where the
// `Ffi*` prefix stops. The types that stayed are the ones the generated type cannot express or is
// the wrong shape for (docs/contracts/session-api.md, "Swift types"): the app's own vocabulary, or
// a record it needs richer than the wire type. Don't add a mirror of a generated type here; add the
// alias.
//
// This file is part of the conversion layer (docs/contracts/build.md), which is why it imports
// `FirstcutCore`: the app types below are built *on* the generated ones (`Rating.flag` is
// `FfiFlag`), and a default value like `flag: Flag = .none` names a generated case. Nothing else in
// `App/Sources` outside the conversion layer may import it.

import FirstcutCore
import Foundation

public typealias PhotoID = UInt64
public typealias BatchID = UInt64

/// The generated `FfiAfInfo` is *richer* (imageWidth/imageHeight/pointsInFocus) and the
/// conversion drops those, which is why this is not an alias yet (todo.md §0.3).
public struct AfInfo: Sendable, Equatable {
    public var areaMode: String
    public var points: [AfPoint]
}

/// The app's photograph metadata: still an app type in this pass (the conversion in
/// `CoreTypeMapping` loses nothing yet, but `AfInfo` inside it is the one field whose
/// generated form is richer), and never persisted — nothing encodes it.
public struct PhotoMeta: Sendable, Equatable, Identifiable {
    public var id: PhotoID
    public var relPath: String
    public var companions: [String]
    public var kind: FileKind
    public var fileSize: UInt64
    public var captureTime: CaptureTime?
    public var shutterCount: UInt64?
    public var fileNumber: UInt32?
    public var cameraMake: String?
    public var cameraModel: String?
    public var cameraSerial: String?
    public var lensModel: String?
    public var focalLengthMm: Float?
    public var exposureTimeS: Float?
    public var fNumber: Float?
    public var iso: UInt32?
    public var exposureCompEv: Float?
    public var meteringMode: String?
    public var driveMode: String?
    public var shutterMode: String?
    public var orientation: UInt8
    public var width: UInt32
    public var height: UInt32
    public var af: AfInfo?
    /// The 1620×1080 `PRVW` JPEG. Good enough for a first-photo fast path; **not** the display
    /// image — a 14" viewer needs more pixels than this has.
    public var preview: EmbeddedPreview?
    /// The full-resolution JPEG (6000×4000 on an R8) from the first image track. Display decodes read
    /// this byte range, so ImageIO never parses the CR3 container (todo.md §7.5).
    public var fullPreview: EmbeddedPreview?
    public var warnings: [String]
}

public struct Batch: Sendable, Codable, Equatable, Identifiable {
    public var id: BatchID
    public var index: UInt32
    public var photoIds: [PhotoID]
    public var provisional: Bool
}

public struct VisualSig: Sendable, Codable, Equatable {
    public var dhash: UInt64
    public var hist: [UInt8]  // 48 values: 16 bins each for R, G, B
}

/// The app's rating, and the one type the wire record cannot replace: the generated
/// `FfiRating` has no defaults on its memberwise init and the app writes `Rating()` in
/// dozens of places. `flag`/`label` are the generated types under their plain names.
public struct Rating: Sendable, Equatable {
    public var stars: UInt8 = 0
    public var flag: Flag = .none
    public var label: ColorLabel? = nil
    public var keep: Bool = false
    public init(stars: UInt8 = 0, flag: Flag = .none, label: ColorLabel? = nil, keep: Bool = false) {
        self.stars = stars
        self.flag = flag
        self.label = label
        self.keep = keep
    }
}

/// The app's rating-mode vocabulary. **Not** the generated `FfiRatingMode`: Rust spells the keep
/// mode `KeepNotKeep`, Swift spells it `keep`, and a case name cannot be aliased (the database
/// string is `"keep"` on both sides, so this is vocabulary only — see CoreTypeMapping).
public enum RatingMode: String, Sendable, Codable { case stars, keep }
