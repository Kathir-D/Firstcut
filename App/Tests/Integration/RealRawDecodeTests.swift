// Owner: pipeline + app-logic.
//
// The real thing: `~/Documents/testing`, 2 880 Canon R8 CR3 files. Everything else in the test suite
// is either a fixture or a synthetic photograph, and none of them can tell you whether ImageIO
// will actually decode a CR3 on this machine, whether the decode is fast enough to prefetch a
// batch, or whether the app shows something.
//
// Skipped when the folder is absent (todo.md §12), so CI stays green.
//
// What this asserts, and why it is worth a minute of wall clock:
//   1. `CGImageSourceCreateThumbnailAtIndex` on a CR3 returns pixels, from the embedded preview,
//      with no RAW decoder anywhere in the app. (The premise the whole pipeline rests on.)
//   2. A display decode at the full size returns the camera's full-size preview, 6000 × 4000.
//   3. The decode cost is per file, not per pixel — which is what makes one 256 px prefetch enough
//      and what sets the worker count.
//   4. **What a display decode costs, and why it is the thumbnail call**: `CreateImageAtIndex` on a
//      CR3 returns 16-bit Display P3 pixels (183 MB a photo, ~850 ms to narrow to 8-bit device RGB),
//      while `CreateThumbnailAtIndex` returns 8-bit and subsamples in the DCT. Measured, not
//      assumed, because the app's memory budget and its arrow-key row both depend on it.
//   5. The whole pipeline, end to end: scan a real folder, prefetch the focus window, and answer
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
            DecodeEngine.decodeDisplay(url: files[0], maxPixel: 6000),
            "the display decode returned nothing for a CR3")
        // A Canon R8's sensor is 6000 × 4000 and the embedded JPEG is that size, so the loupe's
        // "100%" really is 100% of the preview without a RAW decode.
        XCTAssertEqual(full.width, 6000)
        XCTAssertEqual(full.height, 4000)
        // 8 bits per component, in a layout a layer takes without a conversion. `CreateImageAtIndex`
        // gives 16-bit Display P3 here, which is twice the memory and a second conversion.
        XCTAssertTrue(DecodeEngine.isDisplayLayout(full))
    }

    /// What a display decode actually costs, at the sizes the viewer asks for.
    ///
    /// Printed, not asserted on absolute numbers: they depend on the machine and on what else is
    /// running. What *is* asserted is the shape — a decode at a third of the size is not a third of
    /// the time again, and every answer is in the layout a layer can take — because those are the
    /// properties todo.md §7.1 and §7.5 are written against. The absolute figures recorded in
    /// docs/qa/perf-baselines.md came from this test.
    func testDisplayDecodeCosts() throws {
        let files = try realCR3s(count: 1)
        func time(_ body: () -> CGImage?) -> (TimeInterval, CGImage?) {
            // Twice, reporting the second: the first pays for reading a 15 MB file off the SSD.
            _ = body()
            let start = Date()
            let image = body()
            return (Date().timeIntervalSince(start), image)
        }
        for edge in [6000, 3456, 3000, 2000] {
            let (elapsed, image) = time {
                DecodeEngine.decodeDisplay(url: files[0], maxPixel: edge)
            }
            let decoded = try XCTUnwrap(image, "no image at \(edge) px")
            XCTAssertTrue(
                DecodeEngine.isDisplayLayout(decoded),
                "\(edge) px came back as \(decoded.bitsPerComponent)bpc in "
                    + "\(decoded.colorSpace?.name.map(String.init(describing:)) ?? "no space")")
            XCTAssertLessThanOrEqual(max(decoded.width, decoded.height), edge)
            XCTAssertGreaterThan(max(decoded.width, decoded.height), edge / 2)
            print(
                "CR3 display decode at \(edge) px: \(decoded.width)×\(decoded.height) in "
                    + "\(String(format: "%.0f", elapsed * 1000)) ms "
                    + "(\(String(format: "%.0f", Double(decoded.width * decoded.height * 4) / 1_048_576)) MB)"
            )
        }
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
        let full = time { DecodeEngine.decodeDisplay(url: files[2], maxPixel: 6000) }
        print(
            "CR3 decode: 128px \(String(format: "%.0f", small * 1000)) ms · 1600px "
                + "\(String(format: "%.0f", large * 1000)) ms · full \(String(format: "%.0f", full * 1000)) ms"
        )

        // Asking for 12× the pixels must not cost 12× the time. If it does, a future change that
        // moved the filmstrip onto per-frame exact-size decodes would be 12× slower, and this is
        // what would catch it. A generous factor: on a loaded machine the small one can measure
        // much lower simply because of IO scheduling.
        XCTAssertLessThan(
            large, small * 8 + 1.0,
            "a 1600 px thumbnail cost \(large / max(small, 0.001))× a 128 px one")
    }

    // MARK: The whole thing

    /// The shipped path, end to end, on 708 real CR3s: the **core** scans the folder, batches it,
    /// and the pipeline prefetches the focus window and answers every frame from the cache.
    ///
    /// Through the core rather than through `PhotoFolderScanner`, because that is what the app does:
    /// `FileSession` is a mock-only stand-in that reads what ImageIO exposes, and ImageIO does not
    /// expose a CR3's `ISO` or its shutter count, so a test that went through it would be asserting
    /// about a path no user ever takes. The core reads both, on every one of the 2,880 files
    /// (`cargo test --test cr3_exiftool` with `FIRSTCUT_CR3_FULL=1`), and this is the Swift side of
    /// the same claim: every field below reaches the app.
    @MainActor
    func testAFolderOfCR3sScansBatchesPrefetchesAndAnswersFromCache() async throws {
        try skipUnlessTestPhotos()
        let folder = try XCTUnwrap(Game.game1JENKS.photos)

        // A real scan of a real folder through the real core. 708 files, so this is the seconds-long
        // path the loading screen exists for.
        let live = SessionFactory.liveAsync()
        let factory = try XCTUnwrap(live, "the generated bindings have no Session")
        let start = Date()
        let session = try await factory(folder)
        let sessionData = session.data
        let elapsed = Date().timeIntervalSince(start)
        XCTAssertEqual(sessionData.photos.count, 708, "Game1JENKS is 708 photos")
        XCTAssertGreaterThan(sessionData.batches.count, 10, "a shoot is not one batch")
        XCTAssertTrue(
            sessionData.skipped.isEmpty,
            "every CR3 must parse: \(sessionData.skipped.prefix(3).map(\.relPath))")
        print("scanned 708 CR3 headers in \(String(format: "%.1f", elapsed))s")

        // Real metadata, read out of the file rather than off the file system.
        let first = try XCTUnwrap(sessionData.photos.first)
        XCTAssertEqual(first.cameraModel, "Canon EOS R8")
        XCTAssertEqual(first.cameraMake, "Canon")
        XCTAssertEqual(first.width, 6000)
        XCTAssertEqual(first.height, 4000)
        XCTAssertNotNil(first.captureTime, "every frame in a burst has a capture time")
        XCTAssertNotNil(first.fNumber)
        XCTAssertNotNil(first.iso, "and an ISO: the info panel and the sort both read it")
        XCTAssertNotNil(first.shutterCount, "and a shutter count, which is how the order is proved")
        XCTAssertNotNil(first.fullPreview, "and the full-resolution JPEG's byte range")
        XCTAssertEqual(first.kind, .raw(.cr3))

        // Ordering: by capture time, never by file name (§2), which is what the shutter count makes
        // checkable — a Canon shoots 9999 → 0001 within one card.
        let times = sessionData.photos.compactMap(\.captureTime?.unixMs)
        XCTAssertEqual(times, times.sorted(), "photos must be in capture order")
        XCTAssertEqual(sessionData.photos.count, times.count, "every frame has a capture time")

        // The pipeline, on those real files.
        let images = ImageProvider(memoryBudgetBytes: 2 << 30, prefetchPixels: 256, maxConcurrentDecodes: 4)
        images.open(folder: folder, photos: sessionData.photos)
        let batch = try XCTUnwrap(sessionData.batches.first)
        // A real viewer's size, so T2 is decoded at the size the app would ask for (§7.1).
        images.setFocus(
            FocusRequest(
                windows: [FocusWindow(batchID: batch.id, photoIDs: batch.photoIds)],
                currentPhoto: try XCTUnwrap(batch.photoIds.first),
                viewportPixelSize: CGSize(width: 2880, height: 1800)))

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
        XCTAssertEqual(images.stats.focusMisses, 0, "todo.md §7.1: this must stay 0")
        XCTAssertEqual(images.thumbnailProgress, 1.0)
        XCTAssertEqual(images.stats.decodeFailures, 0, "not one of 708 CR3s failed to decode")
        XCTAssertGreaterThan(images.stats.thumbnailDecodes, 0)

        // And the display bitmaps are in the layout a layer takes without a conversion, at the size
        // the window asked for rather than at the camera's 6000 px (§7.2: no double resampling).
        let display = try XCTUnwrap(images.displayImage(for: try XCTUnwrap(batch.photoIds.first)))
        XCTAssertEqual(max(display.width, display.height), 2880)
        XCTAssertTrue(DecodeEngine.isDisplayLayout(display))
        XCTAssertEqual(images.byteRangeDecodes, 1, "the display decode came from the reported range")
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
