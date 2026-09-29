// Owner: app-logic.
//
// Turns the exiftool dumps in `tests/fixtures/exiftool/<game>.json` into the `PhotoMeta` values from
// [photo-meta.md], so the whole model layer can be built, demoed and tested before core-meta's
// parser exists (task.md §0.7). Once core-batch's `dump-meta` output lands in
// `tests/fixtures/meta/<game>.json` this file is deleted in favour of decoding that instead.
//
// Everything in here is **mock-grade on purpose**: the batch splitting is the §5.3 timing heuristic
// with the ambiguous zone joined, which is enough to drive every screen and every rule but is not
// the product's batcher. `MockSession` and `AppModel.preview(game:)` are the only entry points.

import Foundation

public enum FixturePhotos {
    public static let fixtureDirectory = "tests/fixtures/exiftool"
    public static let knownGames = ["Game1JENKS", "Gane2NC", "Game3KC", "Game4VRE"]

    // MARK: - Locating

    /// Looks for the fixtures in `FIRSTCUT_FIXTURES`, then relative to this source file, so both the
    /// test bundle and a checkout build find them without the 42 GB of RAW files.
    public static func fixtureURL(game: String) -> URL? {
        let name = "\(game).json"
        if let root = ProcessInfo.processInfo.environment["FIRSTCUT_FIXTURES"] {
            let url = URL(fileURLWithPath: root).appendingPathComponent(name)
            if FileManager.default.fileExists(atPath: url.path) { return url }
        }
        // Bundled copy first. `tests/fixtures/` is committed into the test bundles (see project.yml),
        // because walking up to the repository root means reading a path inside ~/Documents — and the
        // test host is a GUI app, so that raises a TCC consent prompt on every run and the host
        // blocks forever waiting for a click that never comes. Three hangs and about an hour on
        // 2026-09-29, all from this function. The bundle copy is read from the app's own container,
        // which needs no grant.
        if let bundled = Bundle.main.url(forResource: name, withExtension: nil, subdirectory: fixtureDirectory)
            ?? Bundle.main.url(forResource: "\(fixtureDirectory)/\(name)", withExtension: nil),
            FileManager.default.fileExists(atPath: bundled.path)
        {
            return bundled
        }
        for bundle in Bundle.allBundles {
            if let found = bundle.url(forResource: name, withExtension: nil, subdirectory: fixtureDirectory),
                FileManager.default.fileExists(atPath: found.path)
            {
                return found
            }
        }
        // Session/ → Sources/ → App/ → the repository root. Still useful for a command-line tool run
        // from a checkout, and only reached when the fixtures are not bundled.
        var directory = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { directory.deleteLastPathComponent() }
        for _ in 0..<4 {
            let candidate = directory.appendingPathComponent(fixtureDirectory).appendingPathComponent(name)
            if FileManager.default.fileExists(atPath: candidate.path) { return candidate }
            directory.deleteLastPathComponent()
        }
        return nil
    }

    public enum FixtureError: Error, CustomStringConvertible {
        case notFound(String)

        public var description: String {
            switch self {
            case .notFound(let game):
                "\(fixtureDirectory)/\(game).json not found. Set FIRSTCUT_FIXTURES to the fixtures directory."
            }
        }
    }

    // MARK: - Loading

    public static func loadPhotos(game: String) throws -> [PhotoMeta] {
        guard let url = fixtureURL(game: game) else { throw FixtureError.notFound(game) }
        let data = try Data(contentsOf: url)
        return try loadPhotos(data: data)
    }

    public static func loadPhotos(data: Data) throws -> [PhotoMeta] {
        let rows = try JSONDecoder().decode([ExifRow].self, from: data)
        return rows.map { $0.photoMeta() }
    }

    /// A whole session's worth of mock state for a game.
    public static func loadSessionData(game: String) throws -> SessionData {
        let photos = try loadPhotos(game: game)
        return SessionData(
            folder: "/Volumes/Shoot/\(game)",
            photos: photos,
            batches: batches(for: photos))
    }

    /// A synthetic shoot for when the fixtures aren't around (a built .app previewing the UI).
    /// Bursts of 12 frames at 90 ms with a 1.4 s pause between them — the shape of the fast Canon
    /// bursts in Game1JENKS.
    public static func syntheticPhotos(count: Int = 96, burstSize: Int = 12) -> [PhotoMeta] {
        var photos: [PhotoMeta] = []
        var milliseconds: Int64 = 1_700_000_000_000
        for index in 0..<count {
            if index > 0, index % burstSize == 0 { milliseconds += 1400 }
            milliseconds += 90
            let name = String(format: "IMG_%04d.CR3", index + 1)
            photos.append(
                PhotoMeta(
                    id: stableID(name),
                    relPath: name,
                    companions: [],
                    kind: .raw(.cr3),
                    fileSize: 12_000_000,
                    captureTime: CaptureTime(
                        unixMs: milliseconds, subsecResolutionMs: 10, offsetMinutes: 0, source: .exif),
                    shutterCount: UInt64(index + 1),
                    fileNumber: UInt32(index + 1),
                    cameraMake: "Canon",
                    cameraModel: "Canon EOS R8",
                    cameraSerial: "123456",
                    lensModel: "EF70-200mm f/2.8L IS II USM",
                    focalLengthMm: 200,
                    exposureTimeS: 0.0005,
                    fNumber: 2.8,
                    iso: 800,
                    exposureCompEv: 0,
                    meteringMode: "Evaluative",
                    driveMode: "Continuous Shooting",
                    shutterMode: "Electronic",
                    orientation: 1,
                    width: 6000,
                    height: 4000,
                    af: AfInfo(areaMode: "AF Point Expansion (8 point)", points: []),
                    preview: EmbeddedPreview(range: ByteRange(offset: 0, len: 0), width: 6000, height: 4000),
                    warnings: []))
        }
        return photos
    }

    public static func syntheticSessionData(count: Int = 96) -> SessionData {
        let photos = syntheticPhotos(count: count)
        return SessionData(
            folder: "/Volumes/Shoot/Preview", photos: photos, batches: batches(for: photos))
    }

    // MARK: - Batching (mock only)

    /// The §5.3 timing heuristic, metadata only: hard join below `max(2.5·f, 0.25 s)`, hard split
    /// above 2 s or on an orientation change, and the ambiguous zone joined because deciding it
    /// needs the visual signatures that only the real batcher has. Deterministic by construction.
    public static func batches(for photos: [PhotoMeta]) -> [Batch] {
        guard !photos.isEmpty else { return [] }
        let gaps = zip(photos, photos.dropFirst()).map { gapMilliseconds($0, $1) }
        let fastGaps = gaps.filter { $0 >= 0 && $0 < 500 }.sorted()
        let frameInterval = Double(fastGaps.isEmpty ? 90 : fastGaps[fastGaps.count / 2])
        let joinLimit = max(2.5 * frameInterval, 250)

        var groups: [[PhotoID]] = [[photos[0].id]]
        for index in 1..<photos.count {
            let gap = gaps[index - 1]
            let orientationChanged = photos[index].orientation != photos[index - 1].orientation
            let joins: Bool
            if gap < 0 { joins = false }  // unknown time: never guess, start a new batch
            else if gap > 2000 { joins = false }
            else if orientationChanged { joins = false }
            else { joins = Double(gap) <= joinLimit }
            if joins {
                groups[groups.count - 1].append(photos[index].id)
            } else {
                groups.append([photos[index].id])
            }
        }

        return groups.enumerated().map { offset, ids in
            Batch(
                id: stableID("batch-\(ids[0])"),
                index: UInt32(offset),
                photoIds: ids,
                provisional: true)
        }
    }

    private static func gapMilliseconds(_ a: PhotoMeta, _ b: PhotoMeta) -> Int64 {
        guard let first = a.captureTime?.unixMs, let second = b.captureTime?.unixMs else { return -1 }
        return second - first
    }

    // MARK: - Identity

    /// Stable across runs, like core-meta's "hash of the path relative to the session folder"
    /// (photo-meta.md). FNV-1a keeps it identical on every platform without a dependency.
    public static func stableID(_ string: String) -> UInt64 {
        var hash: UInt64 = 0xCBF2_9CE4_8422_2325
        for byte in string.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 0x1000_0000_01B3
        }
        return hash
    }
}

// MARK: - exiftool JSON

/// One row of `tests/fixtures/exiftool/<game>.json`. Every field except the name is optional so a
/// fixture from a different exiftool version still decodes.
struct ExifRow: Decodable {
    var fileName: String
    var fileSize: UInt64?
    var subSecDateTimeOriginal: String?
    var offsetTimeOriginal: String?
    var shutterCount: UInt64?
    var make: String?
    var model: String?
    var serialNumber: FlexibleString?
    var lensModel: String?
    var focalLength: Double?
    var exposureTime: Double?
    var fNumber: Double?
    var iso: UInt32?
    var exposureCompensation: Double?
    var meteringMode: String?
    var driveMode: String?
    var shutterMode: String?
    var orientation: UInt8?
    var imageWidth: UInt32?
    var imageHeight: UInt32?
    var afAreaMode: String?
    var quality: String?

    enum CodingKeys: String, CodingKey {
        case fileName = "FileName"
        case fileSize = "FileSize"
        case subSecDateTimeOriginal = "SubSecDateTimeOriginal"
        case offsetTimeOriginal = "OffsetTimeOriginal"
        case shutterCount = "ShutterCount"
        case make = "Make"
        case model = "Model"
        case serialNumber = "SerialNumber"
        case lensModel = "LensModel"
        case focalLength = "FocalLength"
        case exposureTime = "ExposureTime"
        case fNumber = "FNumber"
        case iso = "ISO"
        case exposureCompensation = "ExposureCompensation"
        case meteringMode = "MeteringMode"
        case driveMode = "DriveMode"
        case shutterMode = "ShutterMode"
        case orientation = "Orientation"
        case imageWidth = "ImageWidth"
        case imageHeight = "ImageHeight"
        case afAreaMode = "AFAreaMode"
        case quality = "Quality"
    }

    func photoMeta() -> PhotoMeta {
        let capture = ExifDate.parse(subSecDateTimeOriginal ?? "", offsetTimeOriginal: offsetTimeOriginal)
        return PhotoMeta(
            id: FixturePhotos.stableID(fileName),
            relPath: fileName,
            companions: [],
            kind: ExifRow.fileKind(fileName),
            fileSize: fileSize ?? 0,
            captureTime: capture,
            shutterCount: shutterCount,
            fileNumber: nil,
            cameraMake: make,
            cameraModel: model,
            cameraSerial: serialNumber?.value,
            lensModel: lensModel,
            focalLengthMm: focalLength.map(Float.init),
            exposureTimeS: exposureTime.map(Float.init),
            fNumber: fNumber.map(Float.init),
            iso: iso,
            exposureCompEv: exposureCompensation.map(Float.init),
            meteringMode: meteringMode,
            driveMode: driveMode,
            shutterMode: shutterMode,
            orientation: orientation ?? 1,
            width: imageWidth ?? 0,
            height: imageHeight ?? 0,
            af: afAreaMode.map { AfInfo(areaMode: $0, points: []) },
            // The dump has no preview offset; the real scan fills this in so the pipeline can read
            // the embedded JPEG without re-parsing (§7.4).
            preview: nil,
            warnings: capture == nil ? ["no capture time in exiftool dump"] : [])
    }

    static func fileKind(_ name: String) -> FileKind {
        switch (name as NSString).pathExtension.lowercased() {
        case "cr3", "cr2", "crw": .raw(.cr3)
        case "arw", "sr2", "srf": .raw(.arw)
        case "nef", "nrw": .raw(.nef)
        case "raf": .raw(.raf)
        case "rw2": .raw(.rw2)
        case "orf": .raw(.orf)
        case "pef": .raw(.pef)
        case "dng": .raw(.dng)
        case "rwl": .raw(.rwl)
        case "3fr": .raw(.threeFr)
        case "fff": .raw(.fff)
        case "iiq": .raw(.iiq)
        case "srw": .raw(.srw)
        case "dcr": .raw(.dcr)
        case "kdc": .raw(.kdc)
        case "erf": .raw(.erf)
        case "mef": .raw(.mef)
        case "mos": .raw(.mos)
        case "gpr": .raw(.gpr)
        case "x3f": .raw(.x3f)
        case "jpg", "jpeg": .jpeg
        case "heic", "heif", "hif": .heif
        case "tif", "tiff": .tiff
        case "png": .png
        default: .jpeg
        }
    }
}

/// exiftool prints a serial as a number here and as a string elsewhere.
struct FlexibleString: Decodable {
    var value: String

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let text = try? container.decode(String.self) {
            value = text
        } else if let number = try? container.decode(UInt64.self) {
            value = String(number)
        } else {
            value = String(describing: try container.decode(Double.self))
        }
    }
}

/// EXIF timestamps: "2026:08:27 19:54:49.84-06:00", with any of the fraction and the offset
/// missing. Returns UTC milliseconds plus the original offset, like `CaptureTime`.
enum ExifDate {
    static func parse(_ text: String, offsetTimeOriginal: String?) -> CaptureTime? {
        let trimmed = text.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return nil }
        let parts = trimmed.split(separator: " ", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return nil }

        let dateFields = parts[0].split(separator: ":").map(String.init)
        guard dateFields.count == 3,
            let year = Int(dateFields[0]), let month = Int(dateFields[1]), let day = Int(dateFields[2])
        else { return nil }

        var timePart = parts[1]
        var offsetMinutes = parseOffset(offsetTimeOriginal) ?? 0

        // Peel a trailing +HH:MM / -HH:MM off the time field: "19:54:49.84-06:00".
        // The sign is part of the offset, so it has to be parsed together with the digits.
        if let index = timePart.lastIndex(where: { $0 == "+" || $0 == "-" }),
            let parsed = parseOffset(String(timePart[index...]))
        {
            offsetMinutes = parsed
            timePart = String(timePart[timePart.startIndex..<index])
        }

        let timeFields = timePart.split(separator: ":", maxSplits: 2).map(String.init)
        guard timeFields.count >= 2, let hour = Int(timeFields[0]), let minute = Int(timeFields[1])
        else { return nil }

        var second = 0
        var milliseconds = 0
        if timeFields.count == 3 {
            let pieces = timeFields[2].split(separator: ".", maxSplits: 1).map(String.init)
            second = Int(pieces[0]) ?? 0
            if pieces.count == 2 { milliseconds = fractionMilliseconds(pieces[1]) }
        }

        let days = daysFromCivil(year: year, month: month, day: day)
        let seconds = days * 86_400 + Int64(hour) * 3600 + Int64(minute) * 60 + Int64(second)
        let utc = seconds - Int64(offsetMinutes) * 60
        return CaptureTime(
            unixMs: utc * 1000 + Int64(milliseconds),
            subsecResolutionMs: milliseconds == 0 ? 1000 : 10,
            offsetMinutes: Int16(offsetMinutes),
            source: .exif)
    }

    /// "84" at 10 ms resolution is 840 ms, i.e. right-pad the digits to three.
    static func fractionMilliseconds(_ digits: String) -> Int {
        Int(String((digits + "000").prefix(3))) ?? 0
    }

    private static func parseOffset(_ text: String?) -> Int16? {
        guard let text, !text.isEmpty else { return nil }
        let sign = text.hasPrefix("-") ? -1 : 1
        let digits = text.drop(while: { $0 == "+" || $0 == "-" }).replacingOccurrences(of: ":", with: "")
        guard digits.count >= 4, let hours = Int(digits.prefix(2)),
            let minutes = Int(digits.dropFirst(2).prefix(2))
        else { return nil }
        return Int16(sign * (hours * 60 + minutes))
    }

    /// Howard Hinnant's days-from-civil. Avoids `Calendar` and the current time zone, so a test
    /// passes identically in any locale.
    static func daysFromCivil(year: Int, month: Int, day: Int) -> Int64 {
        var y = Int64(year)
        let m = Int64(month)
        y -= m <= 2 ? 1 : 0
        let era = (y >= 0 ? y : y - 399) / 400
        let yearOfEra = y - era * 400
        let dayOfYear = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + Int64(day) - 1
        let dayOfEra = yearOfEra * 365 + yearOfEra / 4 - yearOfEra / 100 + dayOfYear
        return era * 146_097 + dayOfEra - 719_468
    }
}
