// Owner: app-logic.
//
// A real scan of a real folder, for the app to open before core-meta's FFI lands.
//
// `MockSession.dataForFolder` reads only the file system: name, size, mtime. That is enough to
// prove a folder opens, and it is what the app used to do, which is why the info panel showed a
// camera model of "—" and a 0 × 0 size for every Canon R8. ImageIO reads the **real** EXIF out of a
// CR3 without a RAW decoder, so this fills in what a photographer actually looks at.
//
// Verified on `~/Documents/testing/Game1JENKS/IMG_3181.CR3` (Canon EOS R8, body 122022006902):
// ImageIO returns 6000 × 4000, orientation 8, `DateTimeOriginal` 2026:08:27 19:54:49 with
// `SubsecTimeOriginal` 84, 200 mm ƒ/2.8 at 1/2000 s, EF70-200mm f/2.8L IS II USM.
//
// **Not** the ISO, despite what an earlier version of this comment claimed: ImageIO does not expose
// `kCGImagePropertyExifISOSpeedRatings` for a CR3 on this OS, so `iso` comes back nil here. The Rust
// core reads it (CMT2, tag 0x8827) and the shipped app uses the core, so the info panel is correct in
// the app and empty only in a mock shoot. Measured, not assumed:
// `RealRawDecodeTests.testAFolderOfCR3sScansBatchesPrefetchesAndAnswersFromCache` goes through the
// core and asserts the ISO, the shutter count and the full-preview byte range all arrive.
//
// Three things it deliberately does **not** fake:
// - `shutterCount`, `driveMode` and `shutterMode` are not in the EXIF ImageIO exposes, and neither is
//   a CR3's ISO (see above). `nil` and a "—" in the info panel, rather than a plausible-looking
//   number.
// - `EmbeddedPreview.range` needs the byte offset of the embedded JPEG inside the CR3 container,
//   which is core-meta's job (`PhotoMeta.preview`). `nil` here; the pipeline opens files by
//   `PhotoID` → path, so nothing depends on it yet.
//
// Batching is still `FixturePhotos.batches(for:)`, the §5.3 timing heuristic, and every batch comes
// back `provisional` — core-batch's real batcher and its `submit_visual_sigs` pass are not linked.

import CoreGraphics
import Foundation
import ImageIO

public enum PhotoFolderScanError: Error, CustomStringConvertible {
    case notAFolder(String)
    case noPhotos(String)

    public var description: String {
        switch self {
        case .notAFolder(let path): "\(path) is not a folder"
        case .noPhotos(let name): "No photos found in \(name)"
        }
    }
}

public enum PhotoFolderScanner {
    /// Extensions Firstcut treats as a photo. Same list `MockSession.isPhotoExtension` used, kept
    /// here so the scanner and the mock can never disagree about what a photo is.
    public static let photoExtensions: Set<String> = [
        "cr3", "cr2", "crw", "arw", "sr2", "srf", "nef", "nrw", "raf", "rw2", "orf", "pef", "dng",
        "rwl", "3fr", "fff", "iiq", "srw", "dcr", "kdc", "erf", "mef", "mos", "gpr", "x3f", "jpg",
        "jpeg", "heic", "heif", "hif", "tif", "tiff", "png",
    ]

    // MARK: - Scanning

    /// Scans one folder on the calling thread. Right for tests and small folders; the app uses
    /// `scan(_:progress:)` below, which does the same work across eight threads.
    public static func scan(_ folder: URL) throws -> SessionData {
        let listing = try Listing(of: folder)
        var photos: [PhotoMeta] = []
        var skipped: [SkippedFile] = []
        for url in listing.photoURLs {
            guard let meta = photo(at: url, in: folder) else {
                skipped.append(SkippedFile(path: url.lastPathComponent, reason: "no image data"))
                continue
            }
            photos.append(meta)
        }
        return try session(
            folder: folder, photos: photos, companions: listing.companions,
            skipped: skipped)
    }

    /// The same scan, concurrently, reporting progress. Reading 2 880 CR3 headers is seconds of
    /// work, which is why the folder-open path goes through here and shows a loading screen.
    public static func scan(
        _ folder: URL, progress: @escaping @Sendable (Int, Int) -> Void
    ) async throws -> SessionData {
        let listing = try Listing(of: folder)
        let total = listing.photoURLs.count
        guard total > 0 else { throw PhotoFolderScanError.noPhotos(folder.lastPathComponent) }

        progress(0, total)
        let slots = Slots(urls: listing.photoURLs)
        await withTaskGroup(of: Void.self) { group in
            let width = min(8, max(1, ProcessInfo.processInfo.activeProcessorCount))
            for _ in 0..<width {
                group.addTask {
                    while let index = slots.claim() {
                        slots.store(index, Self.photo(at: listing.photoURLs[index], in: folder))
                        progress(slots.done, total)
                    }
                }
            }
        }
        progress(total, total)
        return try session(
            folder: folder, photos: slots.photos,
            companions: listing.companions, skipped: slots.skipped)
    }

    private static func session(
        folder: URL, photos: [PhotoMeta], companions: [String: [String]], skipped: [SkippedFile]
    ) throws -> SessionData {
        var photos = photos
        for index in photos.indices {
            let base = (photos[index].relPath as NSString).deletingPathExtension
            photos[index].companions = companions[base] ?? []
        }
        // Capture order is the whole contract: batches are contiguous ranges of this array
        // (app-model.md), so an out-of-order array is a broken shoot, not a cosmetic problem.
        photos.sort { lhs, rhs in
            switch (lhs.captureTime?.unixMs, rhs.captureTime?.unixMs) {
            case (let left?, let right?) where left != right: return left < right
            case (nil, _?): return false
            case (_?, nil): return true
            default: return lhs.relPath.localizedStandardCompare(rhs.relPath) == .orderedAscending
            }
        }
        guard !photos.isEmpty else { throw PhotoFolderScanError.noPhotos(folder.lastPathComponent) }
        return SessionData(
            folder: folder.path, photos: photos, batches: FixturePhotos.batches(for: photos),
            skipped: skipped)
    }

    // MARK: - One file

    /// Real metadata for one photo file, read through ImageIO. nil when ImageIO cannot make sense
    /// of it, which puts it in the skipped list rather than in the filmstrip as a photo with no
    /// picture (photo-meta.md: a bad file never blocks the folder).
    public static func photo(at url: URL, in folder: URL) -> PhotoMeta? {
        let values = try? url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
        guard values?.isRegularFile == true,
            let source = CGImageSourceCreateWithURL(url as CFURL, nil),
            let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        else { return nil }

        let name = url.lastPathComponent
        let tiff = (properties[kCGImagePropertyTIFFDictionary] ?? [:]) as? [CFString: Any] ?? [:]
        let exif = (properties[kCGImagePropertyExifDictionary] ?? [:]) as? [CFString: Any] ?? [:]

        let width =
            int(properties[kCGImagePropertyPixelWidth])
            ?? int(exif[kCGImagePropertyExifPixelXDimension]) ?? 0
        let height =
            int(properties[kCGImagePropertyPixelHeight])
            ?? int(exif[kCGImagePropertyExifPixelYDimension]) ?? 0
        // Orientation drives the aspect ratio everywhere a photo is drawn, and 0 would divide.
        let orientation = UInt8(clamping: int(tiff[kCGImagePropertyTIFFOrientation]) ?? 1)
        // ISO comes through as an array on some files and as a bare number on others; ImageIO
        // hands back the ISO 800 we asked about either way.
        let iso = exif[kCGImagePropertyExifISOSpeedRatings].flatMap { value -> UInt32? in
            if let list = value as? [NSNumber] { return UInt32(exactly: list.first?.intValue ?? -1) }
            if let number = value as? NSNumber { return UInt32(exactly: number.intValue) }
            return nil
        }

        return PhotoMeta(
            id: FixturePhotos.stableID(name),
            relPath: name,
            companions: [],
            kind: fileKind(name),
            fileSize: UInt64(values?.fileSize ?? 0),
            captureTime: captureTime(exif),
            shutterCount: nil,  // not in the EXIF ImageIO exposes
            fileNumber: nil,
            cameraMake: string(tiff[kCGImagePropertyTIFFMake]),
            cameraModel: string(tiff[kCGImagePropertyTIFFModel]),
            cameraSerial: string(exif[kCGImagePropertyExifBodySerialNumber]),
            lensModel: string(exif[kCGImagePropertyExifLensModel]),
            focalLengthMm: float(exif[kCGImagePropertyExifFocalLength]),
            exposureTimeS: float(exif[kCGImagePropertyExifExposureTime]),
            fNumber: float(exif[kCGImagePropertyExifFNumber]),
            iso: iso,
            exposureCompEv: float(exif[kCGImagePropertyExifExposureBiasValue]),
            meteringMode: meteringMode(int(exif[kCGImagePropertyExifMeteringMode])),
            driveMode: nil,  // Canon's drive mode is a vendor tag, not standard EXIF
            shutterMode: nil,
            orientation: orientation,
            width: UInt32(clamping: width),
            height: UInt32(clamping: height),
            af: nil,  // the AF point table is core-meta's parser, not ImageIO's
            preview: nil,  // needs the embedded-JPEG byte offset inside the CR3 (core-meta)
            warnings: [])
    }

    /// `DateTimeOriginal` plus `SubsecTimeOriginal`, through the same parser the exiftool fixtures
    /// use, so a folder scan and a fixture row produce the same `CaptureTime`. ImageIO does not
    /// expose `OffsetTimeOriginal`, so the offset stays unknown rather than being guessed at.
    private static func captureTime(_ exif: [CFString: Any]) -> CaptureTime? {
        guard let stamp = string(exif[kCGImagePropertyExifDateTimeOriginal]) else { return nil }
        let subsecond = string(exif[kCGImagePropertyExifSubsecTimeOriginal]) ?? ""
        return ExifDate.parse(
            subsecond.isEmpty ? stamp : "\(stamp).\(subsecond)", offsetTimeOriginal: nil)
    }

    private static func meteringMode(_ value: Int?) -> String? {
        switch value {
        case 0: "Unknown"
        case 1: "Average"
        case 2: "Center-weighted"
        case 3: "Spot"
        case 4: "Multi-spot"
        case 5: "Evaluative"
        case 6: "Partial"
        case 255: "Other"
        default: nil
        }
    }

    // MARK: - The directory

    private struct Listing {
        var photoURLs: [URL] = []
        /// Base name (no extension) → the companions that travel with that RAW (todo.md §8).
        var companions: [String: [String]] = [:]

        init(of folder: URL) throws {
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(atPath: folder.path, isDirectory: &isDirectory),
                isDirectory.boolValue
            else { throw PhotoFolderScanError.notAFolder(folder.path) }

            let names = try FileManager.default.contentsOfDirectory(atPath: folder.path)
                .filter { !$0.hasPrefix(".") }
                .sorted()
            for name in names {
                let url = folder.appendingPathComponent(name)
                guard (try? url.resourceValues(forKeys: [.isRegularFileKey]))?.isRegularFile == true
                else { continue }
                let ext = (name as NSString).pathExtension.lowercased()
                if photoExtensions.contains(ext) {
                    photoURLs.append(url)
                } else if ext == "xmp" {
                    companions[(name as NSString).deletingPathExtension, default: []].append(name)
                }
                // Anything else — .AAE, ._CR3 resource forks, exports — is not part of a shoot.
            }
        }
    }

    /// Shared scratch for the concurrent scan: claims an index, stores its result. A class because
    /// `withTaskGroup`'s child tasks are nonisolated and cannot take an `inout`.
    private final class Slots: @unchecked Sendable {
        private let lock = NSLock()
        private let urls: [URL]
        private var results: [PhotoMeta?]
        private var cursor = 0
        private var completed = 0

        init(urls: [URL]) {
            self.urls = urls
            results = [PhotoMeta?](repeating: nil, count: urls.count)
        }

        var done: Int {
            lock.lock()
            defer { lock.unlock() }
            return completed
        }

        func claim() -> Int? {
            lock.lock()
            defer { lock.unlock() }
            guard cursor < results.count else { return nil }
            let index = cursor
            cursor += 1
            return index
        }

        func store(_ index: Int, _ meta: PhotoMeta?) {
            lock.lock()
            defer { lock.unlock() }
            results[index] = meta
            completed += 1
        }

        var photos: [PhotoMeta] {
            lock.lock()
            defer { lock.unlock() }
            return results.compactMap { $0 }
        }

        var skipped: [SkippedFile] {
            lock.lock()
            defer { lock.unlock() }
            return results.indices.compactMap { index in
                results[index] == nil
                    ? SkippedFile(path: urls[index].lastPathComponent, reason: "no image data")
                    : nil
            }
        }
    }

    // MARK: - Property helpers

    private static func string(_ value: Any?) -> String? {
        if let text = value as? String, !text.isEmpty { return text }
        if let number = value as? NSNumber { return number.stringValue }
        return nil
    }

    private static func int(_ value: Any?) -> Int? {
        if let number = value as? NSNumber { return number.intValue }
        if let text = value as? String { return Int(text) }
        return nil
    }

    private static func float(_ value: Any?) -> Float? {
        if let number = value as? NSNumber { return number.floatValue }
        if let text = value as? String { return Float(text) }
        return nil
    }

    static func fileKind(_ name: String) -> FileKind { ExifRow.fileKind(name) }
}
