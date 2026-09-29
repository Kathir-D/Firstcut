// Owner: infra.
//
// Temporary Swift mirrors of the Rust core's types (docs/contracts/photo-meta.md, batching.md,
// session-api.md), so the Swift agents can build and mock from day one. When the UniFFI bindings
// land, infra deletes this file and the generated `FirstcutCore` types take their place with the
// same names. Don't add logic here; request changes from infra so it stays in sync with the contracts.

import Foundation

public typealias PhotoID = UInt64
public typealias BatchID = UInt64

public enum RawFormat: String, Sendable, Codable {
    case cr3, cr2, crw, arw, sr2, srf, nef, nrw, raf, rw2, orf, pef, dng, rwl
    case threeFr, fff, iiq, srw, dcr, kdc, erf, mef, mos, gpr, x3f
}

public enum FileKind: Sendable, Codable, Equatable {
    case raw(RawFormat)
    case jpeg
    case heif
    case tiff
    case png
}

public enum TimeSource: String, Sendable, Codable { case exif, fileModified }

public struct CaptureTime: Sendable, Codable, Equatable {
    public var unixMs: Int64
    public var subsecResolutionMs: UInt16
    public var offsetMinutes: Int16?
    public var source: TimeSource
}

public struct ByteRange: Sendable, Codable, Equatable {
    public var offset: UInt64
    public var len: UInt64
}

public struct EmbeddedPreview: Sendable, Codable, Equatable {
    public var range: ByteRange
    public var width: UInt32
    public var height: UInt32
}

public struct AfPoint: Sendable, Codable, Equatable {
    public var x: Float, y: Float, w: Float, h: Float
    public var inFocus: Bool
}

public struct AfInfo: Sendable, Codable, Equatable {
    public var areaMode: String
    public var points: [AfPoint]
}

public struct PhotoMeta: Sendable, Codable, Equatable, Identifiable {
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
    public var preview: EmbeddedPreview?
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

public enum Flag: String, Sendable, Codable { case none, pick, reject }
public enum ColorLabel: String, Sendable, Codable { case red, yellow, green, blue, purple }

public struct Rating: Sendable, Codable, Equatable {
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

public enum RatingMode: String, Sendable, Codable { case stars, keep }
