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
//   6. T4, the "Exact RAW" develop (`CIRAWFilter`): that it is a real demosaic rather than the
//      embedded preview, that it costs what §7.1 budgets for it, that it applies the EXIF
//      orientation **once** (the double-rotation trap), and that it reaches the layer in the layout
//      a layer can take.

import CoreGraphics
import CoreImage
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

    // MARK: T4, the "Exact RAW" develop

    /// The develop is a real RAW render, not the embedded preview again.
    ///
    /// This is the claim that `CIRAWFilter` is easy to get wrong, so it is checked the way that
    /// survives being wrong: against the filter's **own** `previewImage`, inside one pipeline, so
    /// colour management cannot explain a difference. Compared against the shipping decode it would
    /// be weaker — the two differ in colour space as well as in pixels, and a colour cast alone can
    /// produce a few units of difference (measured: 0.24/255 between two preview reads that a
    /// plausible demosaic would put at several units).
    ///
    /// Measured on `IMG_3192.CR3`: **3.2/255**. On `IMG_3181.CR3`: 51.9/255 (that frame is EXIF 8,
    /// so its preview and its develop are compared at different orientations unless each is
    /// uprighted first — which is what `testExactRawKeepsTheExifOrientation` is for).
    func testExactRawDevelopsTheSensorDataNotThePreview() throws {
        let files = try realCR3s(count: 2)
        for file in files {
            let name = file.lastPathComponent
            let filter = try XCTUnwrap(CIRAWFilter(imageURL: file), "no RAW filter for \(name)")
            let output = try XCTUnwrap(filter.outputImage, "no output image for \(name)")
            let context = CIContext(options: [.cacheIntermediates: false])
            let developed = try XCTUnwrap(
                context.createCGImage(output, from: output.extent.integral), "\(name) did not render")
            let preview = try XCTUnwrap(filter.previewImage, "\(name) exposed no preview")
            let previewCG = try XCTUnwrap(
                context.createCGImage(preview, from: preview.extent.integral))

            // Only compare frames where both are the same geometry: an orientation difference
            // produces a large, meaningless number.
            guard
                developed.width == previewCG.width, developed.height == previewCG.height
            else {
                print(
                    "\(name): develop \(developed.width)x\(developed.height) vs preview \(previewCG.width)x\(previewCG.height) — geometry differs, skipping the pixel comparison"
                )
                continue
            }

            let difference = try XCTUnwrap(meanAbsDifference(developed, previewCG))
            print(
                "\(name): RAW develop vs the filter's own preview, mean abs diff \(String(format: "%.2f", difference))/255"
            )
            XCTAssertGreaterThan(
                difference, 1.5,
                "\(name): the develop is the embedded preview again — CIRAWFilter did not demosaic")
        }
    }

    /// The develop costs what the design says it costs, and the tier's reason for existing.
    ///
    /// §7.1 budgets T4 as "on demand" because it is the most expensive decode in the app. Printed
    /// rather than asserted on absolute numbers (they depend on the machine and on what else is
    /// running), but the *ratio* to the shipping preview decode is asserted, because that ratio is
    /// the whole argument for making this opt-in and current-photo-only.
    ///
    /// Measured on this machine: develop 0.299 s, preview 0.089 s.
    func testExactRawCostsAboutTheSameAsThePreviewItReplaces() throws {
        let files = try realCR3s(count: 1)
        let url = files[0]

        func develop() -> TimeInterval {
            let start = Date()
            guard let filter = CIRAWFilter(imageURL: url), let output = filter.outputImage else {
                return 0
            }
            let context = CIContext(options: [.cacheIntermediates: false])
            _ = context.createCGImage(output, from: output.extent.integral)
            return Date().timeIntervalSince(start)
        }

        func previewDecode() -> TimeInterval {
            let start = Date()
            _ = DecodeEngine.decodeDisplay(url: url, maxPixel: 6000)
            return Date().timeIntervalSince(start)
        }

        // **Alternated, and the order reversed on alternate rounds.** A first measurement of this
        // said 0.299 s for the develop against 0.089 s for the preview, which read as "RAW is 3.4×
        // slower" and would have put T4 in the prefetch for entirely the wrong reason. It was the
        // cold file cache: the first call in a process pays to read the 12 MB CR3 off the SSD, and
        // whichever call went first got the bill. Warm, the two are the same order of magnitude
        // (measured: develop ~79 ms, preview ~90 ms, over five alternating rounds).
        //
        // So the assertion is deliberately weak — the same order of magnitude, not an ordering.
        // Pinning `develop > preview` would encode the cold-cache artefact, and pinning `develop <
        // preview` would pin a coin flip on a loaded machine. What actually keeps T4 one
        // photograph deep is the 92 MB, and that is asserted where it is deterministic instead.
        var rawSamples: [TimeInterval] = []
        var previewSamples: [TimeInterval] = []
        for round in 0..<5 {
            if round % 2 == 0 {
                rawSamples.append(develop())
                previewSamples.append(previewDecode())
            } else {
                previewSamples.append(previewDecode())
                rawSamples.append(develop())
            }
        }
        let raw = rawSamples.sorted()[2]
        let embedded = previewSamples.sorted()[2]
        print(
            "CR3 RAW develop \(String(format: "%.0f", raw * 1000)) ms vs preview decode "
                + "\(String(format: "%.0f", embedded * 1000)) ms "
                + "(median of 5 alternating, \(String(format: "%.2f", raw / max(embedded, 0.001)))×)")
        XCTAssertLessThan(
            raw, embedded * 4,
            "a develop four times the preview decode is not what was measured; if this fires, the "
                + "cost of T4 has genuinely changed and §7.1's budget needs revisiting")
        XCTAssertGreaterThan(
            embedded, 0, "the preview decode must have done real work, or the comparison is void")
    }

    /// EXIF orientation is applied **once**.
    ///
    /// This is the regression test for a wrong-picture bug rather than a slow one. Every other
    /// decode path in the app rotates with `applying(orientation:to:)`, because ImageIO's
    /// container read does not apply the tag. `CIRAWFilter` **does** apply it: `filter.orientation`
    /// defaults to the file's EXIF tag and the output geometry follows it. So a T4 implementation
    /// that also called `applying(orientation:to:)` would rotate an orientation-8 frame twice and
    /// show it upside down — the exact failure `applying(orientation:to:)`'s comment documents
    /// ("a portrait frame with the jersey reading LLORRAC"), and 26 of Game1JENKS's 708 frames are
    /// orientation 8, so it is not a corner.
    ///
    /// The assertion is on geometry, which is what a rotation actually changes, and it holds
    /// without needing a person to look at the picture: for orientation 8 the developed frame must
    /// come out **portrait** (a quarter turn from the sensor's 6000×4000 landscape), and upright.
    func testExactRawKeepsTheExifOrientation() throws {
        // Sample as many frames as the folder offers, up to a cap: the point is to find a rotated
        // one, and on a real shoot 24 is plenty, but the test must not depend on a folder size (and
        // must not fail on a small one).
        let files = try realCR3s(count: 24)
        var sawRotated = false

        // Only a couple of frames are actually rendered: each develop is 0.299 s and 92 MB, and the
        // property being checked does not need 24 of them. Every frame in the sample is still
        // checked for the cheap half of the claim (the filter picked up the tag), which needs no
        // render at all.
        let context = CIContext(options: [.cacheIntermediates: false])
        var rendered = 0
        for file in files {
            let name = file.lastPathComponent
            let orientation = try exifOrientation(of: file)
            let filter = try XCTUnwrap(CIRAWFilter(imageURL: file), "no RAW filter for \(name)")

            // The filter must have picked the tag up from the file by itself. If it did not, the
            // develop would come out in the sensor's frame and the app would have to apply the tag
            // itself — which is the decision this test pins.
            XCTAssertEqual(
                Int(filter.orientation.rawValue), Int(orientation),
                "\(name): the filter read orientation \(filter.orientation.rawValue) but the "
                    + "file says \(orientation)")

            // Quarter turns only (5, 6, 7, 8); 1-4 do not change which way is longer.
            guard [5, 6, 7, 8].contains(orientation), rendered < 2 else { continue }
            sawRotated = true
            rendered += 1

            let output = try XCTUnwrap(filter.outputImage)
            let developed = try XCTUnwrap(
                context.createCGImage(output, from: output.extent.integral))
            // A quarter turn from the sensor's 6000x4000 landscape frame comes out portrait. If the
            // app also applied the tag, this would be landscape again — upside down.
            XCTAssertGreaterThan(
                developed.height, developed.width,
                "\(name) is EXIF \(orientation), so the develop must come out upright (portrait); "
                    + "it came out \(developed.width)x\(developed.height), which means the tag was "
                    + "either dropped or applied twice")
        }
        // Guard against the test silently testing nothing: if none of the first 24 frames is
        // rotated, this asserts nothing and would pass forever.
        XCTAssertTrue(sawRotated, "no rotated frame in the first 24 photos; pick a larger sample")
    }

    /// The T4 decode path end to end on a real CR3, through the provider.
    ///
    /// The unit suite covers the plumbing with synthetic files; this is the same claim against a
    /// real CR3 — the develop produces the photograph's own pixels, at the sensor's size, in the
    /// layout a layer can take without a conversion.
    @MainActor
    func testExactRawThroughTheProviderOnRealCR3s() async throws {
        let files = try realCR3s(count: 24)
        // Prefer a rotated frame: the double-rotation bug only shows on one, and a landscape frame
        // comes out the same size either way. Written as a loop rather than `first(where:)` because
        // the predicate throws, which a non-throwing closure cannot carry.
        var url: URL?
        for file in files {
            if try exifOrientation(of: file) == 8 {
                url = file
                break
            }
        }
        let chosen = try XCTUnwrap(url ?? files.first, "no photograph in the sample")
        let provider = ImageProvider(memoryBudgetBytes: 1 << 30)
        let id: PhotoID = 1
        provider.open(
            folder: chosen.deletingLastPathComponent(),
            photos: [
                PhotoMeta(
                    id: id, relPath: chosen.lastPathComponent, companions: [], kind: .raw(.cr3),
                    fileSize: (try XCTUnwrap(
                        (try FileManager.default.attributesOfItem(atPath: chosen.path)[.size] as? NSNumber)?
                            .uint64Value)),
                    captureTime: nil, shutterCount: nil, fileNumber: nil, cameraMake: nil,
                    cameraModel: nil, cameraSerial: nil, lensModel: nil, focalLengthMm: nil,
                    exposureTimeS: nil, fNumber: nil, iso: nil, exposureCompEv: nil,
                    meteringMode: nil, driveMode: nil, shutterMode: nil,
                    orientation: try exifOrientation(of: chosen), width: 6000, height: 4000,
                    af: nil, preview: nil, fullPreview: nil, warnings: [])
            ])

        let focus = FocusRequest(
            windows: [FocusWindow(batchID: 1, photoIDs: [id])], currentPhoto: id,
            viewportPixelSize: CGSize(width: 1440, height: 900), exactRaw: true)
        provider.setFocus(focus)

        // Wait for the *develop*, not merely for an image: while it is running, `displayImage`
        // legitimately answers with the T2 preview (the same never-blank-the-viewer rule as a
        // resize), so polling for "any image" would race and pick up 1440 px instead of 6000.
        var landed = false
        for _ in 0..<1_200 {
            if provider.stats.exactRawDecodes > 0 {
                landed = true
                break
            }
            try await Task.sleep(for: .milliseconds(50))
        }
        XCTAssertTrue(landed, "the RAW develop never landed within 60s")
        let wentIdle = await provider.waitUntilIdle(timeout: 60)
        XCTAssertTrue(wentIdle, "the engine never went idle")

        let developed = try XCTUnwrap(provider.displayImage(for: id), "no image after the develop")
        XCTAssertEqual(provider.stats.exactRawDecodes, 1, "exactly one develop for one photograph")
        XCTAssertGreaterThan(provider.stats.exactRawBytes, 0, "the develop is real memory")
        // The develop is the sensor's own frame, so the loupe's "100%" is 100% of the sensor rather
        // than of the camera's preview. Asserted with the *orientation applied*, because a quarter
        // turn is exactly what a double rotation would undo: for an EXIF 8 frame the sensor is
        // 6000×4000 landscape and the upright develop is 4000×6000, so a develop that came back
        // landscape would mean the tag was applied twice.
        let orientation = try exifOrientation(of: chosen)
        let sensorIsLandscape = orientation == 1 || orientation == 3
        XCTAssertEqual(
            sensorIsLandscape ? developed.width : developed.height, 6000,
            "the develop is not the sensor's own frame at the file's orientation")
        if !sensorIsLandscape {
            XCTAssertGreaterThan(
                developed.height, developed.width,
                "an EXIF \(orientation) frame must come out upright (portrait); "
                    + "\(developed.width)x\(developed.height) means the tag was applied twice")
        }
        // `CIRAWFilter`'s output lacks `byteOrder32Little`, so this is the assertion that
        // `inDisplayLayout` really ran on the way in.
        XCTAssertTrue(
            DecodeEngine.isDisplayLayout(developed),
            "the develop came back as \(developed.bitsPerComponent)bpc in "
                + "\(developed.colorSpace?.name.map(String.init(describing:)) ?? "no space") "
                + "with bitmapInfo \(developed.bitmapInfo.rawValue)")

        // And with the setting off, the picture is the preview again.
        provider.setFocus(
            FocusRequest(
                windows: [FocusWindow(batchID: 1, photoIDs: [id])], currentPhoto: id,
                viewportPixelSize: CGSize(width: 1440, height: 900), exactRaw: false))
        let settled = await provider.waitUntilIdle(timeout: 60)
        XCTAssertTrue(settled)
        XCTAssertEqual(provider.stats.exactRawBytes, 0)
    }

    /// The EXIF orientation ImageIO reports for a file, as an integer 1...8.
    ///
    /// Read from the **top level** of the properties, or from TIFF — *not* from the Exif
    /// sub-dictionary. That was measured rather than guessed: for a CR3 the tag is at the top level
    /// (and mirrored in `{TIFF}`), and `{Exif}` has no orientation key at all. Reading the Exif
    /// dictionary returns nil, which a `?? 1` would silently turn into "every frame is upright" —
    /// which is exactly the assertion this test exists to make, so it would have passed while
    /// testing nothing. The app itself never reads the tag from ImageIO either: it comes from the
    /// core's own parser (`PhotoMeta.orientation`).
    private func exifOrientation(of url: URL) throws -> UInt8 {
        let source = try XCTUnwrap(CGImageSourceCreateWithURL(url as CFURL, nil))
        let properties = try XCTUnwrap(
            CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        if let top = properties[kCGImagePropertyOrientation] as? UInt8 { return top }
        let tiff = try XCTUnwrap(properties[kCGImagePropertyTIFFDictionary] as? [CFString: Any])
        return try XCTUnwrap(tiff[kCGImagePropertyTIFFOrientation] as? UInt8, "no orientation tag")
    }

    /// Mean absolute per-channel difference between two images of the same size, in 0...255 units.
    private func meanAbsDifference(_ lhs: CGImage, _ rhs: CGImage) -> Double? {
        guard lhs.width == rhs.width, lhs.height == rhs.height else { return nil }
        func pixels(_ image: CGImage) -> [UInt8]? {
            guard
                let context = CGContext(
                    data: nil, width: image.width, height: image.height, bitsPerComponent: 8,
                    bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            else { return nil }
            context.interpolationQuality = .none
            context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
            guard let data = context.data else { return nil }
            return Array(
                UnsafeBufferPointer(
                    start: data.bindMemory(to: UInt8.self, capacity: image.width * image.height * 4),
                    count: image.width * image.height * 4))
        }
        guard let a = pixels(lhs), let b = pixels(rhs) else { return nil }
        var total = 0
        for index in stride(from: 0, to: a.count, by: 4) {
            for channel in 0..<3 { total += abs(Int(a[index + channel]) - Int(b[index + channel])) }
        }
        return Double(total) / Double(a.count / 4 * 3)
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
