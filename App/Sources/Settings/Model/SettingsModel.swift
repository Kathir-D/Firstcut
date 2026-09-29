// Owner: app-logic.
//
// The settings model behind Settings → General / Viewer / Metadata / Performance (task.md §9.8), and
// the `InfoField` values the info panel renders (§9.5).
//
// A plain `Codable` value rather than an observable object: it is mutated through
// `AppModel.updateSettings`, which is the one place that persists it (debounced) and the one place
// that reacts. That makes "what does a setting change" a single, testable code path instead of a
// property observer per field.

import Foundation

// MARK: - General

public enum ArrowBehavior: String, Codable, Equatable, Sendable, CaseIterable {
    /// Stop at the first and last photo of the batch. The default, and what a burst wants.
    case stop
    /// Roll into the next/previous batch, which is handy for a run of single-photo batches.
    case continueIntoNextBatch

    public var title: String {
        switch self {
        case .stop: "Stop at the ends of the batch"
        case .continueIntoNextBatch: "Continue into the next batch"
        }
    }
}

public enum EnteringBatchBehavior: String, Codable, Equatable, Sendable, CaseIterable {
    case firstPhoto
    /// Resume where you left off in that batch (task.md §11, resume).
    case lastViewed

    public var title: String {
        switch self {
        case .firstPhoto: "Select the first photo"
        case .lastViewed: "Select the last photo viewed"
        }
    }
}

public struct GeneralSettings: Codable, Equatable, Sendable {
    public var ratingMode: RatingMode = .stars
    /// Off by default (task.md §6.3); Caps Lock toggles it for a session.
    public var autoAdvance: Bool = false
    public var arrowBehaviorAtBatchEnd: ArrowBehavior = .stop
    public var enteringBatchBehavior: EnteringBatchBehavior = .firstPhoto
    public var finishUnkept: UnkeptAction = .default
    public var finishKept: KeptAction = .default
    public var confirmBeforeFinish: Bool = true
    public var confirmPermanentDelete: Bool = true
    public var confirmDiscardRatings: Bool = true
    /// 5 or 4 stars counts as a keep (task.md §6.1).
    public var keepThreshold: Int = RatingRules.defaultKeepThreshold

    public init() {}
}

// MARK: - Viewer

public struct ViewerSettings: Codable, Equatable, Sendable {
    public var backgroundGray: Double = 0.12
    public var zoomLock: Bool = false
    public var afOverlay: Bool = false
    /// 0…1 clip thresholds for the J overlay (task.md §9.2).
    public var clippingShadowThreshold: Double = 0.0
    public var clippingHighlightThreshold: Double = 1.0
    public var infoFields: Set<InfoField> = InfoField.all
    public var hudVisible: Bool = true
    /// Default for the T4 "Exact RAW" decode (pipeline's job once it exists).
    public var exactRaw: Bool = false

    public init() {}
}

// MARK: - Metadata

/// What a Keep is written as in XMP (task.md §6.2). Lightroom does not read pick flags, so a keep has
/// to be a rating or a label to survive an import — that's why this isn't a flag.
public enum KeepMapping: Equatable, Sendable {
    case rating(Int)  // 1...5
    case colorLabel(ColorLabel)

    public static let stars5 = KeepMapping.rating(5)

    public var title: String {
        switch self {
        case .rating(let stars): "Rating: \(stars) stars"
        case .colorLabel(let label): "Color label: \(label.titleKey)"
        }
    }
}

extension KeepMapping: Codable {
    private enum CodingKeys: String, CodingKey { case type, stars, label }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .type) {
        case "rating": self = .rating(try c.decode(Int.self, forKey: .stars))
        case "colorLabel": self = .colorLabel(try c.decode(ColorLabel.self, forKey: .label))
        case let other:
            throw DecodingError.dataCorruptedError(
                forKey: .type, in: c, debugDescription: "Unknown KeepMapping type \(other)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .rating(let stars):
            try c.encode("rating", forKey: .type)
            try c.encode(stars, forKey: .stars)
        case .colorLabel(let label):
            try c.encode("colorLabel", forKey: .type)
            try c.encode(label, forKey: .label)
        }
    }
}

public enum XmpWriteMode: String, Codable, Equatable, Sendable, CaseIterable {
    /// Keep unknown XMP content (task.md §11). The default, and the only non-destructive option.
    case merge
    case overwrite

    public var title: String {
        switch self {
        case .merge: "Merge into the existing sidecar"
        case .overwrite: "Overwrite the sidecar"
        }
    }
}

public struct MetadataSettings: Codable, Equatable, Sendable {
    public var writeXmp: Bool = true
    public var keepMapping: KeepMapping = .stars5
    public var xmpWriteMode: XmpWriteMode = .merge
    /// Writing into DNG is off by default: it modifies the original file (task.md §11).
    public var writeRatingsIntoDng: Bool = false
    public var writeSidecarsForJpegs: Bool = true

    public init() {}
}

// MARK: - Performance

public struct PerformanceSettings: Codable, Equatable, Sendable {
    /// 40% of physical RAM, ≈ 6.4 GB on a 16 GB M1 Pro (task.md §7.1).
    public var memoryBudgetFraction: Double = 0.40
    public var lookAheadBatches: Int = 2
    public var thumbnailPixels: Int = 256
    /// 0 = auto (performance core count).
    public var decodeThreads: Int = 0
    public var debugHUD: Bool = false

    public init() {}
}

// MARK: - Everything

public struct AppSettings: Codable, Equatable, Sendable {
    public var version: Int = 1
    public var general = GeneralSettings()
    public var viewer = ViewerSettings()
    public var metadata = MetadataSettings()
    public var performance = PerformanceSettings()

    public init() {}

    public var keepThreshold: Int {
        get { general.keepThreshold }
        set { general.keepThreshold = newValue }
    }

    public var memoryBudgetBytes: Int {
        let physical = ProcessInfo.processInfo.physicalMemory  // UInt64
        return Int(Double(physical) * performance.memoryBudgetFraction)
    }
}

// MARK: - Storage

public struct SettingsStore: Sendable {
    public static let fileName = "settings.json"
    public let directory: URL

    public init(directory: URL? = nil) {
        self.directory = directory ?? KeymapStore.defaultDirectory
    }

    public var fileURL: URL { directory.appendingPathComponent(Self.fileName) }

    /// Never throws: a missing or corrupt file falls back to the defaults, because bad preferences
    /// must not stop someone opening a shoot.
    public func load() -> AppSettings {
        guard let data = try? Data(contentsOf: fileURL),
            let settings = try? JSONDecoder().decode(AppSettings.self, from: data)
        else { return AppSettings() }
        return settings
    }

    public func save(_ settings: AppSettings) throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try encoder.encode(settings).write(to: fileURL, options: .atomic)
    }
}

// MARK: - Info panel fields

public enum InfoField: String, Codable, Equatable, Sendable, CaseIterable, Identifiable {
    case fileName
    case captureTime
    case camera
    case lens
    case focalLength
    case shutter
    case aperture
    case iso
    case exposureCompensation
    case metering
    case afMode
    case driveMode
    case shutterMode
    case shutterCount
    case dimensions
    case fileSize
    case folderPath
    case rating
    case batch
    case histogram

    public var id: String { rawValue }

    public static let all: Set<InfoField> = Set(InfoField.allCases)

    public var title: String {
        switch self {
        case .fileName: "File Name"
        case .captureTime: "Capture Time"
        case .camera: "Camera"
        case .lens: "Lens"
        case .focalLength: "Focal Length"
        case .shutter: "Shutter"
        case .aperture: "Aperture"
        case .iso: "ISO"
        case .exposureCompensation: "Exposure Comp"
        case .metering: "Metering"
        case .afMode: "AF Mode"
        case .driveMode: "Drive Mode"
        case .shutterMode: "Shutter Mode"
        case .shutterCount: "Shutter Count"
        case .dimensions: "Dimensions"
        case .fileSize: "File Size"
        case .folderPath: "Folder"
        case .rating: "Rating"
        case .batch: "Batch"
        case .histogram: "Histogram"
        }
    }

    /// The rendered value for the info panel. Returns nil for fields with no data on this photo
    /// (e.g. a folder with no drive mode), which the panel renders as "—".
    public func value(for photo: PhotoVM, batchNumber: Int = 0, position: Int = 0) -> String? {
        let meta = photo.meta
        return switch self {
        case .fileName: photo.fileName
        case .captureTime: InfoField.formatCaptureTime(meta)
        case .camera: [meta.cameraMake, meta.cameraModel].compactMap { $0 }.joined(separator: " ")
            .nilIfEmpty ?? meta.cameraModel
        case .lens: meta.lensModel
        case .focalLength: meta.focalLengthMm.map { "\(Int($0.rounded())) mm" }
        case .shutter: meta.exposureTimeS.map(InfoField.formatShutter)
        case .aperture: meta.fNumber.map { String(format: "ƒ/%.1f", $0) }
        case .iso: meta.iso.map(String.init)
        case .exposureCompensation: meta.exposureCompEv.map { String(format: "%+.1f EV", $0) }
        case .metering: meta.meteringMode
        case .afMode: meta.af.map { $0.areaMode + ($0.points.isEmpty ? "" : " (\($0.points.count))") }
        case .driveMode: meta.driveMode
        case .shutterMode: meta.shutterMode
        case .shutterCount: meta.shutterCount.map(String.init)
        case .dimensions: meta.width > 0 ? "\(meta.width) × \(meta.height)" : nil
        case .fileSize: photo.fileSizeDescription
        case .folderPath: (meta.relPath as NSString).deletingLastPathComponent
        case .rating: InfoField.formatRating(photo)
        case .batch: batchNumber > 0 ? "\(batchNumber), photo \(position)" : nil
        // The histogram is a picture, not text; the panel draws it from pipeline's `histogram(_:)`.
        case .histogram: nil
        }
    }

    static func formatCaptureTime(_ meta: PhotoMeta) -> String? {
        guard let capture = meta.captureTime else { return nil }
        let date = Date(timeIntervalSince1970: Double(capture.unixMs) / 1000)
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy:MM:dd HH:mm:ss.SSS"
        formatter.timeZone = TimeZone(secondsFromGMT: capture.offsetMinutes.map { Int($0) * 60 } ?? 0)
        return formatter.string(from: date)
    }

    static func formatShutter(_ seconds: Float) -> String {
        seconds >= 1 ? String(format: "%.1f s", seconds) : "1/\(Int((1 / Double(seconds)).rounded())) s"
    }

    static func formatRating(_ photo: PhotoVM) -> String {
        var parts: [String] = []
        if photo.rating.stars > 0 { parts.append("\(photo.rating.stars)★") }
        if photo.rating.flag != .none { parts.append(photo.rating.flag == .pick ? "P" : "X") }
        if let label = photo.rating.label { parts.append(label.titleKey) }
        return parts.isEmpty ? "—" : parts.joined(separator: " ")
    }
}

extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
