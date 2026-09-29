// Owner: qa.
//
// Test harness for the integration and performance suites.
//
// Two rules from task.md §12 drive everything here:
//   1. The 42 GB of test photos in ~/Documents/testing are NEVER committed. Tests find them through
//      FIRSTCUT_TEST_PHOTOS (default ~/Documents/testing) and SKIP when the folder is absent, so the
//      suite is green on a machine that has no photos and on CI.
//   2. Anything a test needs to assert on must come from a fixture that IS committed, so CI can
//      check the same thing without the photos.
//
// NOTE(infra): this file is duplicated verbatim in App/Tests/Performance/Support/. See REQ-qa-3 —
// once App/Tests/Support/ is a folder both test targets compile, delete both copies and keep one.

import Darwin
import Foundation
import XCTest

// MARK: - Environment

/// Where the committed fixtures and the uncommitted test photos live.
public enum TestEnvironment {
    /// Env var name that relocates the test photos (build.md "Names").
    public static let photosEnvVar = "FIRSTCUT_TEST_PHOTOS"

    public static let defaultPhotosFolder = "~/Documents/testing"

    /// Root of the repo checkout this test bundle was compiled from, found by walking up from this
    /// file until `project.yml` appears. Works in every agent worktree.
    public static let repositoryRoot: URL? = {
        var url = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        while url.path != "/" {
            if FileManager.default.fileExists(
                atPath: url.appendingPathComponent("project.yml").path)
            {
                return url
            }
            url = url.deletingLastPathComponent()
        }
        return nil
    }()

    /// Set to `1` to run the tests that read the 42 GB of real RAW files.
    ///
    /// **They are opt-in, and the reason is that they otherwise hang the whole suite.** The test
    /// bundle is hosted by `Firstcut.app`, a GUI app. `~/Documents/testing` is inside a
    /// TCC-protected folder, so enumerating it from a GUI app triggers a consent prompt. The host
    /// is ad-hoc signed and therefore a *new code identity on every rebuild*, so the prompt
    /// reappears on every run and -- with nobody there to click Allow -- the test host blocks in
    /// `mach_msg` indefinitely. `xcodebuild test` then dies on a timeout with no diagnostic at
    /// all, which is exactly how this cost an hour: it presents as a slow test, not a hang.
    ///
    /// So anything that touches the real photos asks for this first and skips by default. Those
    /// assertions are still real and still run -- deliberately, by a human who has granted the
    /// folder once:
    ///
    ///     FIRSTCUT_TEST_PHOTOS=~/Documents/testing FIRSTCUT_ALLOW_PHOTO_TESTS=1 \
    ///       xcodebuild ... test
    ///
    /// Committed fixtures are unaffected: they are read from inside the test bundle, which is not
    /// subject to TCC. See `fixtureURL(_:)`.
    public static let allowPhotoTestsEnvVar = "FIRSTCUT_ALLOW_PHOTO_TESTS"

    /// True only when the caller has explicitly opted in to reading the real photos.
    public static var photoTestsAllowed: Bool {
        let raw = ProcessInfo.processInfo.environment[allowPhotoTestsEnvVar] ?? ""
        return raw == "1" || raw.lowercased() == "true" || raw.lowercased() == "yes"
    }

    /// The test photos, or nil when this machine has none. Never creates anything.
    ///
    /// Nil when photo access has not been granted, deliberately: it routes the photo-dependent
    /// tests through their normal "no photos on this machine" path instead of blocking.
    /// See `photoTestsAllowed`.
    public static var testPhotos: URL? {
        guard photoTestsAllowed else { return nil }
        let raw =
            ProcessInfo.processInfo.environment[photosEnvVar]
            ?? (NSString(string: defaultPhotosFolder).expandingTildeInPath)
        guard !raw.isEmpty else { return nil }
        let url = URL(fileURLWithPath: (NSString(string: raw).expandingTildeInPath))
        var isDirectory: ObjCBool = false
        guard
            FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory),
            isDirectory.boolValue
        else { return nil }
        return url
    }

    /// Absolute URL of a committed fixture, or nil when neither the test bundle nor the repo root
    /// has it.
    ///
    /// **The test bundle is checked first, and the reason is a hang, not tidiness.** These tests are
    /// hosted by `Firstcut.app`, which is built inside the repository — and the repository lives in
    /// `~/Documents`, a TCC-protected folder. A GUI app reading a file from there triggers a
    /// consent prompt; an ad-hoc-signed rebuild is a new code identity, so it re-prompts on every
    /// run, and with nobody there to click Allow the test host sits in `mach_msg` forever and
    /// `xcodebuild test` times out with no diagnostic at all.
    ///
    /// A copy of `tests/fixtures/exiftool` is built into each test bundle's resources (see
    /// project.yml) and is not subject to TCC, so the fixtures are read from there. The repository
    /// path stays as a fallback for fixtures that are not bundled, and for a checkout outside a
    /// protected folder.
    public static func fixtureURL(_ relativePath: String) -> URL? {
        // "tests/fixtures/exiftool/Game1JENKS.json" -> bundle resource "exiftool/Game1JENKS.json".
        // project.yml copies in the *contents* of tests/fixtures/<kind>, so the leading
        // "tests/fixtures/" does not appear inside the bundle.
        let parts = relativePath.split(separator: "/").map(String.init)
        guard let last = parts.last else { return nil }
        let directories = Array(parts.dropLast())
        let inBundle = directories.count > 2
            ? directories.dropFirst(2).joined(separator: "/")
            : directories.joined(separator: "/")
        if let bundle = Bundle.allBundles.first(where: { $0.bundlePath.hasSuffix(".xctest") }),
            let resourceURL = bundle.resourceURL
        {
            let candidate = URL(fileURLWithPath: inBundle, relativeTo: resourceURL)
                .appendingPathComponent(last)
            if FileManager.default.fileExists(atPath: candidate.path) { return candidate }
            if let flat = bundle.url(forResource: last, withExtension: "json"),
                FileManager.default.fileExists(atPath: flat.path)
            {
                return flat
            }
        }
        return repositoryRoot?.appendingPathComponent(relativePath)
    }

    /// Folders in `~/Documents/testing` that look like a game, i.e. contain at least one file with a
    /// RAW extension. Sorted by name so runs are reproducible.
    public static func discoveredPhotoFolders() -> [URL] {
        guard let root = testPhotos else { return [] }
        let contents =
            (try? FileManager.default.contentsOfDirectory(
                at: root, includingPropertiesForKeys: [.isDirectoryKey], options: [.skipsHiddenFiles]))
            ?? []
        return contents
            .filter { url in
                guard let isDir = try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory,
                    isDir == true
                else { return false }
                return rawFileCount(in: url) > 0
            }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
    }

    public static func rawFileCount(in folder: URL) -> Int {
        let names =
            (try? FileManager.default.contentsOfDirectory(atPath: folder.path)) ?? []
        return names.filter { Self.rawExtensions.contains(URL(fileURLWithPath: $0).pathExtension.lowercased()) }
            .count
    }

    public static let rawExtensions: Set<String> = ["cr3", "cr2", "crw", "arw", "dng", "nef", "raf"]
}

// MARK: - Games

/// The four test games. Raw values match the exiftool fixture file names in
/// `tests/fixtures/exiftool/<rawValue>.json`.
public enum Game: String, CaseIterable, Sendable {
    case game1JENKS = "Game1JENKS"
    case gane2NC = "Gane2NC"
    case game3KC = "Game3KC"
    case game4VRE = "Game4VRE"

    /// Number of photos exiftool dumped for this game. If a committed fixture ever stops matching,
    /// the fixtures are stale — that is a bug in the fixture, not in the loader.
    public var expectedPhotoCount: Int {
        switch self {
        case .game1JENKS: 708
        case .gane2NC: 529
        case .game3KC: 920
        case .game4VRE: 723
        }
    }

    /// Folder name under `~/Documents/testing`, which is not always the fixture file name
    /// (`Gane2NC` is a typo in the folder on disk too, but not in every folder).
    public var photoFolderName: String { rawValue }

    /// The ambiguous-zone high-speed tail senior-dev and task.md §5.4 single out.
    public static let ambiguousTailGame = Game.game1JENKS
    public static let ambiguousTailRange = "IMG_6117"..."IMG_6164"

    public var exifToolFixtureURL: URL? {
        TestEnvironment.fixtureURL("tests/fixtures/exiftool/\(rawValue).json")
    }

    /// The committed `Vec<PhotoMeta>` dump, once core-batch produces it (photo-meta.md, REV-16).
    public var metaFixtureURL: URL? {
        TestEnvironment.fixtureURL("tests/fixtures/meta/\(rawValue).json")
    }

    /// Ground-truth boundaries, once core-batch publishes them (batching.md §5.4, REV-55).
    public var groundTruthURL: URL? {
        TestEnvironment.fixtureURL("tests/fixtures/ground-truth/\(rawValue).json")
    }

    public var photos: URL? {
        guard let root = TestEnvironment.testPhotos else { return nil }
        let candidate = root.appendingPathComponent(photoFolderName, isDirectory: true)
        var isDirectory: ObjCBool = false
        guard
            FileManager.default.fileExists(atPath: candidate.path, isDirectory: &isDirectory),
            isDirectory.boolValue
        else { return nil }
        return candidate
    }
}

// MARK: - exiftool fixture

/// One record of `tests/fixtures/exiftool/<game>.json`. Field names are exiftool's own; this is the
/// committed stand-in for `PhotoMeta` until `tests/fixtures/meta/<game>.json` lands.
public struct ExifToolRecord: Decodable, Sendable {
    public var fileName: String
    public var fileSize: Int64
    public var subSecDateTimeOriginal: String?
    public var offsetTimeOriginal: String?
    public var shutterCount: Int64?
    public var make: String?
    public var model: String?
    public var serialNumber: Int64?
    public var lensModel: String?
    public var focalLength: Double?
    public var exposureTime: Double?
    public var fNumber: Double?
    public var iso: Int?
    public var exposureCompensation: Double?
    public var meteringMode: String?
    public var continuousDrive: String?
    public var shutterMode: String?
    public var orientation: Int?
    public var imageWidth: Int?
    public var imageHeight: Int?
    public var afAreaMode: String?
    /// exiftool writes `AFPointsInFocus` either as a single integer or as a comma-separated list of
    /// AF point indices, depending on the AF area mode — 2,722 records of the first form and 158 of
    /// the second across the 2,880 test photos. Decoded leniently into one list; see BUG-1 in
    /// docs/qa/bugs.md, because any strict decoder of the committed fixture crashes on those 158.
    public var afPointsInFocus: [Int]?

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
        case continuousDrive = "ContinuousDrive"
        case shutterMode = "ShutterMode"
        case orientation = "Orientation"
        case imageWidth = "ImageWidth"
        case imageHeight = "ImageHeight"
        case afAreaMode = "AFAreaMode"
        case afPointsInFocus = "AFPointsInFocus"
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        fileName = try container.decode(String.self, forKey: .fileName)
        fileSize = try container.decode(Int64.self, forKey: .fileSize)
        subSecDateTimeOriginal = try container.decodeIfPresent(String.self, forKey: .subSecDateTimeOriginal)
        offsetTimeOriginal = try container.decodeIfPresent(String.self, forKey: .offsetTimeOriginal)
        shutterCount = try container.decodeIfPresent(Int64.self, forKey: .shutterCount)
        make = try container.decodeIfPresent(String.self, forKey: .make)
        model = try container.decodeIfPresent(String.self, forKey: .model)
        serialNumber = try container.decodeIfPresent(Int64.self, forKey: .serialNumber)
        lensModel = try container.decodeIfPresent(String.self, forKey: .lensModel)
        focalLength = try container.decodeIfPresent(Double.self, forKey: .focalLength)
        exposureTime = try container.decodeIfPresent(Double.self, forKey: .exposureTime)
        fNumber = try container.decodeIfPresent(Double.self, forKey: .fNumber)
        iso = try container.decodeIfPresent(Int.self, forKey: .iso)
        exposureCompensation = try container.decodeIfPresent(Double.self, forKey: .exposureCompensation)
        meteringMode = try container.decodeIfPresent(String.self, forKey: .meteringMode)
        continuousDrive = try container.decodeIfPresent(String.self, forKey: .continuousDrive)
        shutterMode = try container.decodeIfPresent(String.self, forKey: .shutterMode)
        orientation = try container.decodeIfPresent(Int.self, forKey: .orientation)
        imageWidth = try container.decodeIfPresent(Int.self, forKey: .imageWidth)
        imageHeight = try container.decodeIfPresent(Int.self, forKey: .imageHeight)
        afAreaMode = try container.decodeIfPresent(String.self, forKey: .afAreaMode)
        afPointsInFocus = try container.decodeLossyIntListIfPresent(forKey: .afPointsInFocus)
    }

    /// Capture instant in UTC microseconds, parsed from `SubSecDateTimeOriginal` plus
    /// `OffsetTimeOriginal`. Returns nil when the fields are missing or malformed; no test should
    /// rely on a nil here for the Canon set.
    ///
    /// Canon R8 writes `SubSecDateTimeOriginal` as "2026:08:27 19:54:49.84-06:00": 2-digit
    /// hundredths, i.e. a 10 ms resolution. Other bodies write 3 or 4 digits, so the fraction is
    /// read as a decimal and scaled rather than assumed to be hundredths.
    public var captureUnixMicroseconds: Int64? {
        guard let sub = subSecDateTimeOriginal else { return nil }
        let parts = sub.split(separator: " ", maxSplits: 1)
        guard parts.count == 2 else { return nil }
        let datePart = String(parts[0])
        let timePart = String(parts[1])
        let timeDigits = timePart.prefix { $0.isNumber || $0 == ":" }
        let fractionPart = timePart.dropFirst(timeDigits.count).prefix {
            $0.isNumber || $0 == "."
        }
        guard let seconds = ExifToolDateParser.utcFormatter.date(from: datePart + " " + timeDigits) else {
            return nil
        }
        var fraction = 0.0
        if let fractionStart = fractionPart.firstIndex(of: ".") {
            let digits = fractionPart[fractionPart.index(after: fractionStart)...]
            var value = 0.0
            var count = 0
            for char in digits {
                guard let digit = char.wholeNumberValue else { break }
                value = value * 10 + Double(digit)
                count += 1
            }
            if count > 0 { fraction = value / pow(10, Double(count)) }
        }
        let utc = seconds.timeIntervalSince1970 + offsetMinutes * 60 + fraction
        return Int64((utc * 1_000_000).rounded())
    }

    /// Offset of `OffsetTimeOriginal` in minutes, 0 when absent.
    public var offsetMinutes: Double {
        guard let raw = offsetTimeOriginal else { return 0 }
        let sign: Double = raw.hasPrefix("-") ? -1 : 1
        let digits = raw.dropFirst().filter(\.isNumber)
        guard digits.count >= 4 else { return 0 }
        let hours = Double(digits.prefix(2)) ?? 0
        let minutes = Double(digits.suffix(2)) ?? 0
        return sign * (hours * 60 + minutes)
    }

    /// ISO-8601 UTC instant, the form the human-facing QA checklist uses.
    public var captureISO8601: String? {
        guard let us = captureUnixMicroseconds else { return nil }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd'T'HH:mm:ss.SSS'Z'"
        return formatter.string(from: Date(timeIntervalSince1970: Double(us) / 1_000_000))
    }
}

/// Decodes a JSON value that may be an integer, a numeric string, or a comma-separated list of
/// either, into an array of integers. exiftool's output is not type-stable across AF area modes and
/// the committed fixtures are raw exiftool output, so every decoder built on them has to be this
/// tolerant — or the mock and the real parser disagree, which is the failure mode BUG-1 records.
extension KeyedDecodingContainer {
    func decodeLossyIntListIfPresent(forKey key: Key) throws -> [Int]? {
        if let single = ((try? decodeIfPresent(Int.self, forKey: key)) ?? nil) {
            return [single]
        }
        guard let text = ((try? decodeIfPresent(String.self, forKey: key)) ?? nil) else { return nil }
        return
            text
            .split(separator: ",")
            .compactMap { Int($0.trimmingCharacters(in: .whitespaces)) }
    }
}

/// Shared date parsing. `DateFormatter` is not Sendable, so the instance is isolated to one queue
/// rather than shared across the parallel test threads XCTest uses.
enum ExifToolDateParser {
    private static let queue = DispatchQueue(label: "com.kathird.firstcut.tests.exiftool-date")
    private static let formatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy:MM:dd HH:mm:ss"
        return formatter
    }()

    /// exiftool writes `DateTimeOriginal` with colons in the date part ("2026:08:27 19:54:49").
    static func date(from string: String) -> Date? {
        queue.sync { formatter.date(from: string) }
    }

    static var utcFormatter: DateFormatter { formatter }
}

public enum FixtureError: Error, CustomStringConvertible {
    case repositoryRootNotFound
    case fixtureMissing(String)
    case photoCountMismatch(game: Game, expected: Int, actual: Int)

    public var description: String {
        switch self {
        case .repositoryRootNotFound:
            "Could not find project.yml above \(#filePath) — is this test bundle from another checkout?"
        case .fixtureMissing(let path):
            "Missing committed fixture: \(path)"
        case .photoCountMismatch(let game, let expected, let actual):
            "\(game.rawValue) fixture has \(actual) records, expected \(expected)"
        }
    }
}

// MARK: - Loader

public enum Fixtures {
    /// Decodes `tests/fixtures/exiftool/<game>.json`.
    public static func exifToolRecords(for game: Game) throws -> [ExifToolRecord] {
        guard let url = game.exifToolFixtureURL, FileManager.default.fileExists(atPath: url.path)
        else { throw FixtureError.fixtureMissing(game.exifToolFixtureURL?.path ?? "tests/fixtures/exiftool/\(game.rawValue).json") }
        let data = try Data(contentsOf: url)
        let records = try JSONDecoder().decode([ExifToolRecord].self, from: data)
        guard records.count == game.expectedPhotoCount else {
            throw FixtureError.photoCountMismatch(
                game: game, expected: game.expectedPhotoCount, actual: records.count)
        }
        return records
    }

    /// Decodes the `{"schema": "photo-meta/1", "photos": [...]}` dump once it exists. Raw JSON
    /// rather than `PhotoMeta`, so this keeps compiling across the UniFFI swap (REV-7).
    public static func photoMetaJSON(for game: Game) throws -> Data {
        guard let url = game.metaFixtureURL, FileManager.default.fileExists(atPath: url.path) else {
            throw FixtureError.fixtureMissing("tests/fixtures/meta/\(game.rawValue).json")
        }
        return try Data(contentsOf: url)
    }

    /// True when a required optional fixture has landed. Lets a test assert "not yet" without
    /// failing the suite while the owning agent is still on it.
    public static func exists(_ url: URL?) -> Bool {
        guard let url else { return false }
        return FileManager.default.fileExists(atPath: url.path)
    }

    /// Absolute URL for a test photo, or nil when this machine has no photos.
    public static func photoURL(_ fileName: String, in game: Game) -> URL? {
        game.photos?.appendingPathComponent(fileName)
    }
}

// MARK: - XCTest conveniences

extension XCTestCase {
    /// Skips the calling test when the machine has no test photos. Call at the top of any test that
    /// touches RAW data; the committed-fixture tests must NOT use this.
    public func skipUnlessTestPhotos(_ message: String = "Needs the test photos in ~/Documents/testing (set \(TestEnvironment.photosEnvVar))") throws {
        try XCTSkipUnless(
            TestEnvironment.testPhotos != nil,
            message
        )
    }

    /// Skips unless a required optional fixture has landed, naming who owns it.
    public func skipUnlessFixture(_ url: URL?, ownedBy owner: String, named name: String) throws {
        try XCTSkipUnless(
            Fixtures.exists(url),
            "\(name) has not landed yet (owner: \(owner)). Skipping rather than failing."
        )
    }

    /// Skips unless the machine is idle enough for a timing measurement (perf suite only).
    /// §7.3 numbers are only comparable on an otherwise quiet M1 Pro; a run while Xcode is
    /// indexing or nine agents are compiling records a baseline nobody can reproduce.
    public func skipUnlessMachineIsQuiet(_ maximumLoadAverage: Double = 1.0) throws {
        let load = SystemLoad.oneMinuteAverage
        try XCTSkipUnless(
            load <= maximumLoadAverage,
            "System load \(String(format: "%.2f", load)) > \(maximumLoadAverage); run the perf "
                + "suite on an otherwise idle machine."
        )
    }
}

/// 1-minute load average, via libc `getloadavg`. `ProcessInfo.systemLoadAverage` is not exposed to
/// Swift, and a busy-machine check that does not compile is worse than none. macOS reports a
/// per-core average, so the same threshold works on any machine.
public enum SystemLoad {
    public static var oneMinuteAverage: Double {
        var loads = [Double](repeating: 0, count: 3)
        guard getloadavg(&loads, 3) == 3 else { return 0 }
        return loads[0]
    }
}
