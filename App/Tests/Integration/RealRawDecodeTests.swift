// Owner: pipeline + app-logic.
//
// The real thing: `~/Documents/testing`, 2 880 Canon R8 CR3 files. Everything else in the test suite
// is either a fixture or a synthetic photograph, and none of them can tell you whether ImageIO
// will actually decode a CR3 on this machine, whether the decode is fast enough to prefetch a
// batch, or whether the app shows something.
//
// Skipped when the folder is absent (task.md §12), so CI stays green.
//
// What this asserts, and why it is worth a minute of wall clock:
//   1. `CGImageSourceCreateThumbnailAtIndex` on a CR3 returns pixels, from the embedded preview,
//      with no RAW decoder anywhere in the app. (The premise the whole pipeline rests on.)
//   2. `CGImageSourceCreateImageAtIndex` returns the camera's full-size preview, 6000 × 4000.
//   3. The decode cost is per file, not per pixel — which is what makes one 256 px prefetch enough
//      and what sets the worker count.
//   4. The whole pipeline, end to end: scan a real folder, prefetch the focus window, and answer
//      every frame from the cache with `focusMisses == 0`.

import CoreGraphics
import Foundation
import XCTest

@testable import Firstcut

final class RealRawDecodeTests: XCTestCase {
    private func realCR3s(count: Int) throws -> [URL] {
        try skipUnlessTestPhotos()
        let folder = try XCTUnwrap(Game.game1JENKS.photos, "Game1JENKS folder is missing")
        let names = try FileManager.default.contentsOfDirectory(atPath: folder.path)
            .filter { $0.lowercased().hasSuffix(".cr3") }.sorted()
        XCTAssertGreaterThanOrEqual(names.count, count, "expected \(count) CR3s to measure")
        return Array(names.prefix(count)).map { folder.appendingPathComponent($0) }
    }

    // MARK: The two decode calls the pipeline is built on

    func testThumbnailReadsTheEmbeddedPreviewFromACR3() throws {
        let files = try realCR3s(count: 1)
        let source = try XCTUnwrap(
            CGImageSourceCreateWithURL(files[0] as CFURL, nil), "ImageIO could not open a CR3")
        XCTAssertEqual(CGImageSourceGetType(source) as String?, "com.canon.cr3-raw-image")
        XCTAssertEqual(CGImageSourceGetCount(source), 1, "a CR3 is a one-frame container")

        let thumbnail = try XCTUnwrap(
            DecodeEngine.decodeThumbnail(url: files[0], maxPixel: 256),
            "CreateThumbnailAtIndex returned nothing for a CR3")
        XCTAssertLessThanOrEqual(max(thumbnail.width, thumbnail.height), 256)
        XCTAssertGreaterThan(min(thumbnail.width, thumbnail.height), 32, "a placeholder-sized decode")
    }

    func testFullReadIsTheCamerasFullSizePreview() throws {
        let files = try realCR3s(count: 1)
        let full = try XCTUnwrap(
            DecodeEngine.decodeFull(url: files[0]), "CreateImageAtIndex returned nothing for a CR3")
        // A Canon R8's sensor is 6000 × 4000 and the embedded JPEG is that size, so the loupe's
        // "100%" really is 100% of the preview without a RAW decode.
        XCTAssertEqual(full.width, 6000)
        XCTAssertEqual(full.height, 4000)
    }

    /// The measurement that sets `maxConcurrentDecodes`, and the reason the UI treats thumbnails
    /// as arriving over time. Printed, not asserted: absolute timings depend on the machine and on
    /// what else is running, so pinning them would make this a flaky test. The *shape* is asserted
    /// below.
    func testDecodeCostIsPerFileNotPerPixel() throws {
        let files = try realCR3s(count: 3)
        func time(_ body: () -> CGImage?) -> TimeInterval {
            let start = Date()
            _ = body()
            return Date().timeIntervalSince(start)
        }
        let small = time { DecodeEngine.decodeThumbnail(url: files[0], maxPixel: 128) }
        let large = time { DecodeEngine.decodeThumbnail(url: files[1], maxPixel: 1600) }
        let full = time { DecodeEngine.decodeFull(url: files[2]) }
        print(
            "CR3 decode: 128px \(String(format: "%.0f", small * 1000)) ms · 1600px "
                + "\(String(format: "%.0f", large * 1000)) ms · full \(String(format: "%.0f", full * 1000)) ms")

        // Asking for 12× the pixels must not cost 12× the time. If it does, a future change that
        // moved the filmstrip onto per-frame exact-size decodes would be 12× slower, and this is
        // what would catch it. A generous factor: on a loaded machine the small one can measure
        // much lower simply because of IO scheduling.
        XCTAssertLessThan(
            large, small * 8 + 1.0,
            "a 1600 px thumbnail cost \(large / max(small, 0.001))× a 128 px one")
    }

    // MARK: The whole thing

    @MainActor
    func testAFolderOfCR3sScansPrefetchesAndAnswersFromCache() async throws {
        try skipUnlessTestPhotos()
        let folder = try XCTUnwrap(Game.game1JENKS.photos)

        // A real scan of a real folder. 708 files, so this is the seconds-long path the loading
        // screen exists for.
        let start = Date()
        let sessionData = try PhotoFolderScanner.scan(folder)
        let elapsed = Date().timeIntervalSince(start)
        XCTAssertEqual(sessionData.photos.count, 708, "Game1JENKS is 708 photos")
        XCTAssertGreaterThan(sessionData.batches.count, 10, "a shoot is not one batch")
        print("scanned 708 CR3 headers in \(String(format: "%.1f", elapsed))s")

        // Real metadata, read out of the file rather than off the file system.
        let first = try XCTUnwrap(sessionData.photos.first)
        XCTAssertEqual(first.cameraModel, "Canon EOS R8")
        XCTAssertEqual(first.cameraMake, "Canon")
        XCTAssertEqual(first.width, 6000)
        XCTAssertEqual(first.height, 4000)
        XCTAssertNotNil(first.captureTime, "every frame in a burst has a capture time")
        XCTAssertNotNil(first.fNumber)
        XCTAssertNotNil(first.iso)
        XCTAssertEqual(first.kind, .raw(.cr3))

        // The pipeline, on those real files.
        let images = ImageProvider(memoryBudgetBytes: 2 << 30, prefetchPixels: 256, maxConcurrentDecodes: 4)
        images.open(folder: folder, photos: sessionData.photos)
        let batch = try XCTUnwrap(sessionData.batches.first)
        images.setFocus(
            FocusRequest(
                windows: [FocusWindow(batchID: batch.id, photoIDs: batch.photoIds)],
                currentPhoto: try XCTUnwrap(batch.photoIds.first)))

        let prefetchStart = Date()
        let wentIdle = await images.waitUntilIdle(timeout: 300)
        let prefetch = Date().timeIntervalSince(prefetchStart)
        XCTAssertTrue(wentIdle, "the prefetch never finished")
        print(
            "prefetched \(batch.photoIds.count) CR3 thumbnails in "
                + "\(String(format: "%.1f", prefetch))s "
                + "(\(String(format: "%.1f", Double(batch.photoIds.count) / prefetch)) photos/s)")

        // The whole promise, on real files: every frame of the focus window comes back from the
        // cache and nothing has to be decoded on demand.
        for id in batch.photoIds {
            XCTAssertNotNil(
                images.thumbnail(for: id, size: CGSize(width: 74, height: 74)),
                "\(id) was not decoded")
        }
        XCTAssertEqual(images.stats.focusMisses, 0, "task.md §7.1: this must stay 0")
        XCTAssertEqual(images.thumbnailProgress, 1.0)
        XCTAssertEqual(images.stats.decodeFailures, 0, "not one of 708 CR3s failed to decode")
        XCTAssertGreaterThan(images.stats.thumbnailDecodes, 0)
    }

    @MainActor
    func testEveryGameInTheTestFolderDecodes() async throws {
        try skipUnlessTestPhotos()
        let root = try XCTUnwrap(TestEnvironment.testPhotos)
        let games = try FileManager.default.contentsOfDirectory(atPath: root.path)
            .filter { name in
                var isDirectory: ObjCBool = false
                let url = root.appendingPathComponent(name)
                return FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory)
                    && isDirectory.boolValue
            }
        XCTAssertFalse(games.isEmpty)

        for game in games.sorted() {
            let folder = root.appendingPathComponent(game, isDirectory: true)
            // One photo is enough to prove the format decodes; 2 880 would be seven minutes of
            // wall clock for the same answer.
            let name = try XCTUnwrap(
                try FileManager.default.contentsOfDirectory(atPath: folder.path)
                    .first { $0.lowercased().hasSuffix(".cr3") },
                "\(game) has no CR3s")
            let url = folder.appendingPathComponent(name)
            XCTAssertNotNil(
                DecodeEngine.decodeThumbnail(url: url, maxPixel: 256), "\(game)/\(name) did not decode")
        }
    }
}
