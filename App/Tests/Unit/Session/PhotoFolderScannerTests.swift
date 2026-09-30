// Owner: app-logic.
//
// `PhotoFolderScanner` and `FileSession` — opening a folder of real files.
//
// Written JPEGs with real EXIF, not the exiftool fixture: the fixtures are a *dump* of what
// exiftool read, so they would let a broken ImageIO call pass. The Canon R8 CR3s in
// `~/Documents/testing` are the real thing and are exercised by `RealRawDecodeTests` in the
// integration target.

import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers

@testable import Firstcut

@Suite("Folder scanning")
struct PhotoFolderScannerTests {
    /// A JPEG with EXIF written into it, so the scanner's `CGImageSourceCopyPropertiesAtIndex` path
    /// has something to read. 6000 × 4000 and a Canon body serial are the values observed on the
    /// real CR3s, so a field the scanner forgets to map shows up as nil here too.
    @discardableResult
    private func writePhoto(
        _ folder: URL, _ name: String, width: Int = 1200, height: Int = 800, orientation: Int = 8,
        captured: String = "2026:08:27 19:54:49", subsecond: String = "84"
    ) -> URL {
        let url = folder.appendingPathComponent(name)
        let context = CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue)!
        context.setFillColor(CGColor(red: 0.3, green: 0.4, blue: 0.5, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 8, height: 8))
        let image = context.makeImage()!
        let exif: [CFString: Any] = [
            kCGImagePropertyExifDateTimeOriginal: captured,
            kCGImagePropertyExifSubsecTimeOriginal: subsecond,
            kCGImagePropertyExifLensModel: "EF70-200mm f/2.8L IS II USM",
            kCGImagePropertyExifBodySerialNumber: "122022006902",
            kCGImagePropertyExifFNumber: 2.8,
            kCGImagePropertyExifExposureTime: 0.0005,
            kCGImagePropertyExifFocalLength: 200,
            kCGImagePropertyExifISOSpeedRatings: [800],
            kCGImagePropertyExifExposureBiasValue: 0,
            kCGImagePropertyExifMeteringMode: 5,
        ]
        let tiff: [CFString: Any] = [
            kCGImagePropertyTIFFMake: "Canon",
            kCGImagePropertyTIFFModel: "Canon EOS R8",
            kCGImagePropertyTIFFOrientation: orientation,
        ]
        let properties: [CFString: Any] = [
            kCGImagePropertyPixelWidth: width,
            kCGImagePropertyPixelHeight: height,
            kCGImagePropertyExifDictionary: exif,
            kCGImagePropertyTIFFDictionary: tiff,
        ]
        let destination = CGImageDestinationCreateWithURL(
            url as CFURL, UTType.jpeg.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(destination, image, properties as CFDictionary)
        #expect(CGImageDestinationFinalize(destination))
        return url
    }

    private func folder() -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("FirstcutScan-\(UUID().uuidString)", isDirectory: true)
        try! FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    @Test("A real EXIF block becomes a real PhotoMeta")
    func readsRealMetadata() throws {
        let folder = folder()
        writePhoto(folder, "IMG_3181.jpg")
        let data = try PhotoFolderScanner.scan(folder)
        let photo = try #require(data.photos.first)

        #expect(photo.relPath == "IMG_3181.jpg")
        // The pixel dimensions come from the file, not from a property bag: ImageIO writes the
        // real ones, so this cannot be faked. The 6000 × 4000 of a Canon R8 is asserted in
        // `RealRawDecodeTests`, against the real CR3s.
        #expect(photo.width == 1200)
        #expect(photo.height == 800)
        #expect(photo.orientation == 8)
        #expect(photo.cameraMake == "Canon")
        #expect(photo.cameraModel == "Canon EOS R8")
        #expect(photo.cameraSerial == "122022006902")
        #expect(photo.lensModel == "EF70-200mm f/2.8L IS II USM")
        #expect(photo.focalLengthMm == 200)
        #expect(photo.fNumber == 2.8)
        #expect(photo.iso == 800)
        #expect(photo.meteringMode == "Evaluative")
        #expect(photo.fileSize > 0)
        // 19:54:49.84 with no zone, which is all ImageIO gives: 19:54:49 UTC, 840 ms.
        #expect(photo.captureTime?.unixMs == 1_787_860_489_840)
        #expect(photo.captureTime?.source == .exif)
    }

    @Test("The fields ImageIO does not carry are nil, not invented")
    func nothingIsInvented() throws {
        let folder = folder()
        writePhoto(folder, "IMG_3181.jpg")
        let photo = try #require(try PhotoFolderScanner.scan(folder).photos.first)
        // Canon's shutter count is a vendor tag; the drive and shutter modes are too. A plausible
        // number here would be a lie in the info panel that nothing could correct.
        #expect(photo.shutterCount == nil)
        #expect(photo.driveMode == nil)
        #expect(photo.shutterMode == nil)
        // The AF point table and the embedded-preview byte range are core-meta's parser's job.
        #expect(photo.af == nil)
        #expect(photo.preview == nil)
    }

    @Test("Photos come back in capture order, not in file-name order")
    func captureOrderWins() throws {
        let folder = folder()
        // Names sort as 1, 2, 3 but they were captured 3, 1, 2.
        writePhoto(folder, "IMG_0001.jpg", captured: "2026:08:27 19:00:10")
        writePhoto(folder, "IMG_0002.jpg", captured: "2026:08:27 19:00:30")
        writePhoto(folder, "IMG_0003.jpg", captured: "2026:08:27 19:00:20")
        let photos = try PhotoFolderScanner.scan(folder).photos
        #expect(photos.map(\.relPath) == ["IMG_0001.jpg", "IMG_0003.jpg", "IMG_0002.jpg"])
    }

    @Test("A `.xmp` sidecar is attached to the RAW beside it and nothing else")
    func sidecarsAttachToTheirRaw() throws {
        let folder = folder()
        writePhoto(folder, "IMG_0001.jpg")
        writePhoto(folder, "IMG_0002.jpg")
        try Data("<x:xmpmeta/>".utf8).write(to: folder.appendingPathComponent("IMG_0002.xmp"))
        try Data("<x:xmpmeta/>".utf8).write(to: folder.appendingPathComponent("orphan.xmp"))
        let photos = try PhotoFolderScanner.scan(folder).photos
        #expect(photos.count == 2, "a sidecar is not a photo")
        #expect(photos[0].companions.isEmpty)
        #expect(photos[1].companions == ["IMG_0002.xmp"])
    }

    @Test("A burst splits into batches and a pause splits the burst")
    func batchesFollowTheTiming() throws {
        let folder = folder()
        // Twelve frames 90 ms apart, then a 3 s gap, then three more.
        //
        // The sub-second is written as three digits — `840`, not `81` — because that is what exiftool
        // emits and what `ExifDate` expects. The earlier version of this fixture passed `millis % 1000`
        // unpadded, so the ninth frame asked for a "sub-second" of 810 ms: a value that is not a
        // sub-second, which failed to parse, left that photo with no capture time, and made every
        // gap unknown. `FixturePhotos.batches` then split *every* pair and the test failed with 15
        // one-photo batches instead of 12 + 3. A fixture that does not round-trip is a fixture that
        // tests nothing.
        for index in 0..<12 {
            let wholeSeconds = index * 90 / 1000
            let millis = index * 90 % 1000
            writePhoto(
                folder, String(format: "IMG_%04d.jpg", index),
                captured: String(
                    format: "2026:08:27 19:10:%02d", wholeSeconds),
                subsecond: String(format: "%03d", millis))
        }
        // The trailing three are a burst too, so they are 90 ms apart like the first — the point of
        // the test is that a *pause* splits and nothing else does. Spacing them a second apart
        // (the first attempt at this fixture) is not a burst by the product's own rule:
        // `joinLimit = max(2.5 * frameInterval, 250)` is 250 ms for a 90 ms shoot, so each 1 s gap
        // opened its own batch and the assertion read `[12, 1, 1, 1]`. The heuristic was right and
        // the fixture was wrong.
        for index in 0..<3 {
            let millis = index * 90
            writePhoto(
                folder, String(format: "IMG_%04d.jpg", 12 + index),
                captured: "2026:08:27 19:14:00", subsecond: String(format: "%03d", millis))
        }
        let data = try PhotoFolderScanner.scan(folder)
        #expect(data.photos.count == 15)
        #expect(data.batches.count == 2)
        #expect(data.batches[0].photoIds.count == 12)
        #expect(data.batches[1].photoIds.count == 3)
        // core-batch is not linked, so every batch is provisional and the HUD says so.
        #expect(data.batches.allSatisfy { $0.provisional })
        #expect(data.batches.flatMap(\.photoIds).count == 15)
    }

    @Test("A file that is not an image is skipped, never fatal (todo.md §8)")
    func unreadableFileDoesNotBlockTheFolder() throws {
        let folder = folder()
        writePhoto(folder, "IMG_0001.jpg")
        writePhoto(folder, "IMG_0002.jpg")
        try Data("this is not a photo".utf8).write(to: folder.appendingPathComponent("broken.jpg"))
        let data = try PhotoFolderScanner.scan(folder)
        #expect(data.photos.count == 2)
        #expect(data.skipped.count == 1)
        #expect(data.skipped.first?.path.hasSuffix("broken.jpg") == true)
    }

    @Test("An empty folder and a missing folder both fail with a reason")
    func failuresAreExplained() throws {
        let empty = folder()
        #expect(throws: PhotoFolderScanError.self) { try PhotoFolderScanner.scan(empty) }
        let missing = empty.appendingPathComponent("nope")
        #expect(throws: PhotoFolderScanError.self) { try PhotoFolderScanner.scan(missing) }
    }

    @Test("The concurrent scan gives the same answer as the sequential one")
    func asyncScanMatches() async throws {
        let folder = folder()
        for index in 0..<24 {
            writePhoto(
                folder, String(format: "IMG_%04d.jpg", index),
                captured: String(format: "2026:08:27 19:%02d:%02d", index / 12, index % 12))
        }
        let sequential = try PhotoFolderScanner.scan(folder)
        let reported = Report()
        let concurrent = try await PhotoFolderScanner.scan(folder) { done, total in
            reported.record(done: done, total: total)
        }
        #expect(concurrent.photos == sequential.photos)
        #expect(concurrent.batches == sequential.batches)
        #expect(reported.total == 24)
        #expect(reported.lastDone == 24)
    }

    /// Thread-safe progress collector; the callback is `@Sendable` and called from eight workers.
    private final class Report: @unchecked Sendable {
        private let lock = NSLock()
        private var _total = 0
        private var _lastDone = 0
        var total: Int { lock.lock(); defer { lock.unlock() }; return _total }
        var lastDone: Int { lock.lock(); defer { lock.unlock() }; return _lastDone }
        func record(done: Int, total: Int) {
            lock.lock(); defer { lock.unlock() }
            _total = total
            _lastDone = done
        }
    }
}

@Suite("File session")
@MainActor
struct FileSessionTests {
    @Test("Opening a real folder produces a session the model can cull")
    func opensAFolder() throws {
        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("FirstcutFileSession-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        for index in 0..<6 {
            let context = CGContext(
                data: nil, width: 8, height: 8, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue)!
            context.setFillColor(CGColor(red: 0.2, green: 0.2, blue: 0.2, alpha: 1))
            context.fill(CGRect(x: 0, y: 0, width: 8, height: 8))
            let url = folder.appendingPathComponent("IMG_\(1000 + index).jpg")
            let destination = CGImageDestinationCreateWithURL(
                url as CFURL, "public.jpeg" as CFString, 1, nil)!
            CGImageDestinationAddImage(destination, context.makeImage()!, nil)
            #expect(CGImageDestinationFinalize(destination))
        }

        let session = try FileSession.open(folder)
        #expect(session.data.photos.count == 6)
        #expect(session.data.folder == folder.path)

        // And the model drives it end to end, which is the whole point of the file-session path.
        let model = AppModel(.testing(backend: session))
        model.open(session, folderName: folder.lastPathComponent)
        #expect(model.phase == .culling)
        #expect(model.currentPhoto?.fileName == "IMG_1000.jpg")
        model.perform(.setStars(5))
        #expect(session.rating(for: model.currentPhoto!.id).stars == 5)
    }

    @Test("The shipped factory names the backend it is about to hand over")
    func backendNameIsHonest() {
        // Not a "which one is active" test for its own sake: this string is what a screenshot
        // check or an About box can print, and it must not claim a session database that is not
        // there.
        //
        // This used to assert the Rust exports were *absent*. They are not any more: `ffi.rs` exports
        // the real `Session`, so `hasSessionAPI` is true and the shipped path is the Rust core with
        // SQLite and XMP sidecars. Asserting the fallback name here would have kept passing only for
        // as long as the product was unfinished, which is the wrong thing for a test to do. So the
        // assertion is on the *invariant* — the name states which backend is in use, and it agrees
        // with `hasSessionAPI` — and the stale "no session database yet" expectation is gone.
        let name = SessionFactory.backendName
        #expect(!name.isEmpty)
        if FirstcutCoreBridge.hasSessionAPI {
            #expect(name.contains("Rust core"))
            #expect(name.contains("SQLite"))
        } else {
            #expect(name.contains("no session database yet"))
        }
        // Whichever it is, the factory must reach the same conclusion — the two are computed from
        // the same read of the generated bindings, and this catches them drifting apart.
        #expect(SessionFactory.live() != nil)
    }
}
