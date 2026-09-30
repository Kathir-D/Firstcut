// Owner: pipeline.
//
// The real `ImageProvider`, against real files written to a temporary directory.
//
// A JPEGMaking fixture, not a mock of the decoder: the point of these tests is that ImageIO is
// really called, really off the main thread, and really cached. The CR3 case is
// `RealRawDecodeTests` in the integration target, which reads `~/Documents/testing`.

import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers

@testable import Firstcut

/// Writes a JPEG with known dimensions and a distinctive colour, so a decoded bitmap can be told
/// apart from a placeholder.
enum ImageFixtures {
    static func folder(_ name: String = "shots") -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("FirstcutImages-\(name)-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    @discardableResult
    static func jpeg(
        in folder: URL, named name: String, width: Int = 400, height: Int = 300,
        color: (Double, Double, Double) = (0.2, 0.5, 0.8)
    ) -> URL {
        let url = folder.appendingPathComponent(name)
        let context = CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue)!
        context.setFillColor(CGColor(red: color.0, green: color.1, blue: color.2, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        // A second block so a downsample cannot be confused with a solid fill.
        context.setFillColor(CGColor(red: 0.9, green: 0.3, blue: 0.1, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width / 2, height: height))
        let image = context.makeImage()!
        let destination = CGImageDestinationCreateWithURL(
            url as CFURL, UTType.jpeg.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(destination, image, nil)
        #expect(CGImageDestinationFinalize(destination))
        return url
    }

    /// N photos in one folder, ids 1...n, which is what `ImageProvider.open` wants.
    static func shoot(count: Int) -> (folder: URL, photos: [PhotoMeta]) {
        let folder = folder()
        var photos: [PhotoMeta] = []
        for index in 0..<count {
            let name = "IMG_\(String(format: "%04d", index)).jpg"
            jpeg(in: folder, named: name, color: (Double(index % 5) / 5, 0.5, 0.8))
            photos.append(
                PhotoMeta(
                    id: PhotoID(index + 1), relPath: name, companions: [], kind: .jpeg,
                    fileSize: 1024, captureTime: nil, shutterCount: nil, fileNumber: nil,
                    cameraMake: nil, cameraModel: nil, cameraSerial: nil, lensModel: nil,
                    focalLengthMm: nil, exposureTimeS: nil, fNumber: nil, iso: nil, exposureCompEv: nil,
                    meteringMode: nil, driveMode: nil, shutterMode: nil, orientation: 1,
                    width: 400, height: 300, af: nil, preview: nil, fullPreview: nil, warnings: []))
        }
        return (folder, photos)
    }
}

private func focus(_ ids: [PhotoID], current: PhotoID) -> FocusRequest {
    FocusRequest(
        windows: [FocusWindow(batchID: 1, photoIDs: ids)], currentPhoto: current)
}

@Suite("Real image decoding")
@MainActor
struct ImageProviderTests {
    @Test("A thumbnail is scheduled on the first ask and answered from the cache after")
    func thumbnailsArriveAsynchronously() async throws {
        let shoot = ImageFixtures.shoot(count: 1)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20, prefetchPixels: 128)
        provider.open(folder: shoot.folder, photos: shoot.photos)

        // The contract with the filmstrip: a cache miss returns nil rather than blocking `body`.
        let immediate = provider.thumbnail(for: 1, size: CGSize(width: 64, height: 48))
        #expect(immediate == nil)
        #expect(await provider.waitUntilIdle())

        // The provider answers with the *prefetch* bitmap, never with something smaller than was
        // asked for: re-decoding a CR3 per filmstrip frame to hit an exact pixel count would cost
        // ~300 ms and would make every frame a focus miss.
        let decoded = try! #require(provider.thumbnail(for: 1, size: CGSize(width: 64, height: 48)))
        #expect(max(decoded.width, decoded.height) >= 64)
        #expect(max(decoded.width, decoded.height) <= provider.prefetchPixels)
        #expect(provider.stats.thumbnailDecodes == 1)
        #expect(provider.stats.decodeFailures == 0)
    }

    @Test("A second ask is a cache hit and costs no decode")
    func repeatedAsksHitTheCache() async {
        let shoot = ImageFixtures.shoot(count: 1)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20, prefetchPixels: 128)
        provider.open(folder: shoot.folder, photos: shoot.photos)
        _ = provider.thumbnail(for: 1, size: CGSize(width: 96, height: 64))
        #expect(await provider.waitUntilIdle())

        // 96 px against a 128 px prefetch is within the 1.5× slack, so it is a hit: this is the
        // case that would otherwise make every filmstrip frame a focus miss.
        for _ in 0..<5 { _ = provider.thumbnail(for: 1, size: CGSize(width: 96, height: 64)) }
        #expect(provider.stats.thumbnailDecodes == 1)
        #expect(provider.stats.thumbnailCacheHits == 5)
        #expect(provider.stats.focusMisses == 0)
    }

    @Test("Nothing in the focus window is ever decoded on demand (pipeline-api.md §Guarantees)")
    func theFocusWindowIsNeverDecodedOnDemand() async {
        let shoot = ImageFixtures.shoot(count: 12)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20, prefetchPixels: 128)
        provider.open(folder: shoot.folder, photos: shoot.photos)

        let ids = shoot.photos.map(\.id)
        provider.setFocus(focus(ids, current: 6))
        #expect(await provider.waitUntilIdle())

        // The window is fully decoded, so every ask is a hit.
        let missesBefore = provider.stats.focusMisses
        let decodesBefore = provider.stats.thumbnailDecodes
        for id in ids {
            #expect(provider.thumbnail(for: id, size: CGSize(width: 74, height: 74)) != nil)
        }
        #expect(provider.stats.focusMisses == missesBefore)
        #expect(provider.stats.thumbnailDecodes == decodesBefore)
        #expect(provider.stats.focusMisses == 0)
        #expect(provider.thumbnailProgress == 1.0)
    }

    @Test("A photo outside the focus window misses the cache but is not a focus miss")
    func outsideTheWindowIsNotAFocusMiss() async {
        let shoot = ImageFixtures.shoot(count: 4)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20, prefetchPixels: 128)
        provider.open(folder: shoot.folder, photos: shoot.photos)
        provider.setFocus(focus([1, 2], current: 1))
        #expect(await provider.waitUntilIdle())

        #expect(provider.stats.focusMisses == 0)
        #expect(provider.thumbnail(for: 4, size: CGSize(width: 64, height: 64)) == nil)
        #expect(provider.stats.focusMisses == 0, "photo 4 is not in the focus window")
        #expect(provider.stats.thumbnailCacheMisses > 0)
    }

    @Test("The focus window is prefetched in the order the user can see it")
    func prefetchIsPriorityOrdered() async {
        let shoot = ImageFixtures.shoot(count: 6)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20, prefetchPixels: 128)
        provider.open(folder: shoot.folder, photos: shoot.photos)

        // The model always reports previous/current/next; a jump from one batch to a distant one
        // is what the priority order is for.
        provider.setFocus(focus([5, 6, 1, 2, 3, 4], current: 2))
        #expect(await provider.waitUntilIdle())
        for id in 1...6 {
            #expect(provider.thumbnail(for: PhotoID(id), size: CGSize(width: 74, height: 74)) != nil)
        }
        #expect(provider.stats.thumbnailDecodes == 6)
        #expect(provider.stats.focusMisses == 0)
    }

    @Test("The display image is the full-resolution one, not a thumbnail")
    func displayIsFullResolution() async {
        let shoot = ImageFixtures.shoot(count: 1)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20)
        provider.open(folder: shoot.folder, photos: shoot.photos)

        #expect(provider.displayImage(for: 1) == nil)
        #expect(await provider.waitUntilIdle())
        let display = provider.displayImage(for: 1)
        #expect(display?.width == 400)
        #expect(display?.height == 300)
        #expect(provider.stats.displayDecodes == 1)
    }

    @Test("Only the current photo and its near neighbours are prefetched at full resolution")
    func displayPrefetchIsBounded() async {
        let shoot = ImageFixtures.shoot(count: 12)
        let provider = ImageProvider(memoryBudgetBytes: 256 << 20)
        provider.open(folder: shoot.folder, photos: shoot.photos)
        provider.setFocus(focus(shoot.photos.map(\.id), current: 6))
        #expect(await provider.waitUntilIdle())
        // Photo 6, two behind and three ahead (the panes of 4-up Compare), out of twelve. Twelve
        // full decodes would be a whole batch at full resolution for a window the user sees a
        // few frames of.
        #expect(provider.stats.displayDecodes == 6)
        for id in 4...9 { #expect(provider.displayImage(for: PhotoID(id)) != nil) }
    }

    @Test("A histogram is 64 bins per channel, normalised, and cached")
    func histogram() async {
        let shoot = ImageFixtures.shoot(count: 1)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20)
        provider.open(folder: shoot.folder, photos: shoot.photos)

        // Nothing decoded yet: the histogram asks for the display image and says so by saying nil.
        #expect(provider.histogram(for: 1) == nil)
        #expect(await provider.waitUntilIdle())
        let histogram = try! #require(provider.histogram(for: 1))
        #expect(histogram.red.count == 64)
        #expect(histogram.luminance.count == 64)
        for channel in [histogram.red, histogram.green, histogram.blue, histogram.luminance] {
            let total = channel.reduce(0, +)
            #expect(abs(total - 1) < 0.001, "bins are fractions of the frame, not raw counts")
        }
        #expect(histogram.blue.max() ?? 0 > 0.05, "the fixture is blue in the right half")

        let computes = provider.stats.histogramComputes
        #expect(provider.histogram(for: 1) != nil)
        #expect(provider.stats.histogramComputes == computes, "cached, not recomputed")
    }

    @Test("An unreadable file is reported once, not retried forever")
    func unreadableFile() async {
        let shoot = ImageFixtures.shoot(count: 2)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20)
        provider.open(folder: shoot.folder, photos: shoot.photos)
        // Corrupt the second file behind the provider's back.
        try! Data("not a jpeg at all".utf8).write(to: shoot.folder.appendingPathComponent("IMG_0001.jpg"))

        for _ in 0..<5 { _ = provider.thumbnail(for: 2, size: CGSize(width: 64, height: 48)) }
        #expect(await provider.waitUntilIdle())
        #expect(provider.stats.decodeFailures == 1)
        #expect(provider.thumbnail(for: 2, size: CGSize(width: 64, height: 48)) == nil)
        #expect(provider.stats.decodeFailures == 1, "a failed decode is not retried")
    }

    @Test("The memory budget evicts the least recently used entry, never the focus window")
    func budgetEvicts() async {
        let shoot = ImageFixtures.shoot(count: 4)
        // Four 400×300 thumbnails at 4 B/px are 480 KB each; 1 MB holds two.
        let provider = ImageProvider(memoryBudgetBytes: 1 << 20, prefetchPixels: 400)
        provider.open(folder: shoot.folder, photos: shoot.photos)

        provider.setFocus(focus([3, 4], current: 3))
        #expect(await provider.waitUntilIdle())
        #expect(provider.stats.thumbnailBytes <= (1 << 20))

        // Walk well past the budget with photos that are not in the focus window.
        for id in 1...2 { _ = provider.thumbnail(for: PhotoID(id), size: CGSize(width: 400, height: 300)) }
        #expect(await provider.waitUntilIdle())
        #expect(provider.stats.thumbnailBytes <= (1 << 20))

        // The focus window survived the eviction, so the promise still holds.
        for id in [3, 4] {
            #expect(provider.thumbnail(for: PhotoID(id), size: CGSize(width: 200, height: 200)) != nil)
        }
    }

    @Test("Progress is 1 with no session and rises to 1 across a real prefetch")
    func progress() async {
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20)
        #expect(provider.thumbnailProgress == 1, "no session is not '0% of nothing'")
        let shoot = ImageFixtures.shoot(count: 8)
        provider.open(folder: shoot.folder, photos: shoot.photos)
        provider.setFocus(focus(shoot.photos.map(\.id), current: 1))
        #expect(await provider.waitUntilIdle())
        #expect(provider.thumbnailProgress == 1.0)
    }

    @Test("Opening a second folder drops the first one's cache")
    func reopeningResets() async {
        let first = ImageFixtures.shoot(count: 2)
        let second = ImageFixtures.shoot(count: 3)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20)
        provider.open(folder: first.folder, photos: first.photos)
        provider.setFocus(focus([1, 2], current: 1))
        #expect(await provider.waitUntilIdle())
        #expect(provider.stats.thumbnailDecodes == 2)

        // Ids are hashes of file names, so the new folder's ids overlap and a stale cache entry
        // would show the wrong photograph. This is the guard.
        provider.open(folder: second.folder, photos: second.photos)
        #expect(provider.stats.thumbnailDecodes == 0)
        #expect(provider.stats.focusSize == 0)
        #expect(provider.thumbnail(for: 1, size: CGSize(width: 64, height: 48)) == nil)
    }

    @Test("A photo added to the open folder keeps the pixels of the photos that are still there")
    func addingAPhotoKeepsTheRestOfTheCache() async {
        let shoot = ImageFixtures.shoot(count: 6)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20, prefetchPixels: 128)
        provider.open(folder: shoot.folder, photos: shoot.photos)
        provider.setFocus(focus(shoot.photos.map(\.id), current: 1))
        #expect(await provider.waitUntilIdle())
        let decodesBefore = provider.stats.thumbnailDecodes
        #expect(decodesBefore == 6)

        // One more file lands in the same folder (a second card, a file copied in). The shoot is
        // re-read and every photo that did not change must not be decoded a second time: a CR3
        // costs ~300 ms, so dropping the whole cache blanks the filmstrip and re-decodes the shoot
        // the user is in the middle of rating.
        var grown = shoot.photos
        ImageFixtures.jpeg(in: shoot.folder, named: "IMG_9999.jpg")
        grown.append(
            PhotoMeta(
                id: 9999, relPath: "IMG_9999.jpg", companions: [], kind: .jpeg, fileSize: 1024,
                captureTime: nil, shutterCount: nil, fileNumber: nil, cameraMake: nil,
                cameraModel: nil, cameraSerial: nil, lensModel: nil, focalLengthMm: nil,
                exposureTimeS: nil, fNumber: nil, iso: nil, exposureCompEv: nil, meteringMode: nil,
                driveMode: nil, shutterMode: nil, orientation: 1, width: 400, height: 300, af: nil,
                preview: nil, fullPreview: nil, warnings: []))
        provider.open(folder: shoot.folder, photos: grown)

        for id in shoot.photos.map(\.id) {
            #expect(
                provider.thumbnail(for: id, size: CGSize(width: 64, height: 64)) != nil,
                "photo \(id) is still in the folder and must not have been decoded again")
        }
        #expect(provider.stats.thumbnailDecodes == decodesBefore, "no re-decode of the survivors")
    }

    @Test("A photo that vanished is dropped from the cache and cannot be served")
    func aVanishedPhotoIsEvicted() async {
        let shoot = ImageFixtures.shoot(count: 4)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20, prefetchPixels: 128)
        provider.open(folder: shoot.folder, photos: shoot.photos)
        provider.setFocus(focus(shoot.photos.map(\.id), current: 1))
        #expect(await provider.waitUntilIdle())

        // The last file is deleted in Finder, the core re-reads the folder, and the model hands the
        // provider the shorter list.
        provider.open(folder: shoot.folder, photos: Array(shoot.photos.prefix(3)))
        #expect(
            provider.thumbnail(for: 4, size: CGSize(width: 64, height: 64)) == nil,
            "a deleted photo has no file, so it must not be answered from the cache")
        for id in 1...3 {
            #expect(provider.thumbnail(for: PhotoID(id), size: CGSize(width: 64, height: 64)) != nil)
        }
    }

    @Test("Moving on cancels queued work for photos the user left")
    func movingOnCancelsStaleWork() async {
        let shoot = ImageFixtures.shoot(count: 30)
        // One decode at a time, so what is still queued when the focus moves is deterministic.
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20, prefetchPixels: 128, maxConcurrentDecodes: 1)
        provider.open(folder: shoot.folder, photos: shoot.photos)
        provider.setFocus(focus(shoot.photos.map(\.id), current: 1))
        provider.setFocus(focus([29, 30], current: 30))
        #expect(await provider.waitUntilIdle())
        // The two photos now in focus, plus whatever was already decoding when the focus moved.
        #expect(provider.stats.thumbnailDecodes <= 4)
        #expect(provider.thumbnail(for: 29, size: CGSize(width: 64, height: 64)) != nil)
        #expect(provider.thumbnail(for: 30, size: CGSize(width: 64, height: 64)) != nil)
    }

    @Test("Batches are prefetched current first, then next, then previous")
    func windowsNearestFirst() {
        let windows = (0..<5).map { FocusWindow(batchID: BatchID($0), photoIDs: [PhotoID($0 * 10 + 1)]) }
        let order = ImageProvider.nearestFirst(FocusRequest(windows: windows, currentPhoto: 21))
        #expect(order.map(\.batchID) == [2, 3, 1, 4, 0])
    }

    @Test("Memory pressure drops everything outside the focus window, and nothing in it")
    func memoryPressureSheds() async {
        let shoot = ImageFixtures.shoot(count: 4)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20, prefetchPixels: 128)
        provider.open(folder: shoot.folder, photos: shoot.photos)
        provider.setFocus(focus([1, 2], current: 1))
        for id in [3, 4] { _ = provider.thumbnail(for: PhotoID(id), size: CGSize(width: 64, height: 64)) }
        #expect(await provider.waitUntilIdle())
        let decodes = provider.stats.thumbnailDecodes

        provider.shedForMemoryPressure()
        #expect(provider.stats.pressureSheds == 1)
        for id in [1, 2] {
            #expect(provider.thumbnail(for: PhotoID(id), size: CGSize(width: 64, height: 64)) != nil)
        }
        #expect(provider.thumbnail(for: 3, size: CGSize(width: 64, height: 64)) == nil)
        #expect(provider.stats.thumbnailDecodes == decodes)
    }

    @Test("A decode that lands after another folder was opened is dropped")
    func lateLandingFromThePreviousFolderIsDropped() async throws {
        let first = ImageFixtures.shoot(count: 1)
        let second = ImageFixtures.shoot(count: 1)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20, prefetchPixels: 128)
        provider.open(folder: first.folder, photos: first.photos)
        _ = provider.thumbnail(for: 1, size: CGSize(width: 64, height: 64))
        provider.open(folder: second.folder, photos: second.photos)
        // The first folder's decode may still be running; give it time to land.
        try await Task.sleep(for: .milliseconds(300))
        #expect(provider.stats.thumbnailDecodes == 0, "the first folder's IMG_0000 is not the second's")
    }

    @Test("The decoder calls are the ones the contract names")
    func decoderOptions() throws {
        // Not a performance test — a naming test. `CreateThumbnailAtIndex` is what reads the
        // embedded preview, and the two option keys are what make it subsample during decode.
        // ContactSheet.swift's `downsample` is the recipe; if a future change swaps in a RAW
        // decoder this still compiles, but the integration target's CR3 test will fail.
        let shoot = ImageFixtures.shoot(count: 1)
        let thumb = try #require(
            DecodeEngine.decodeThumbnail(url: shoot.folder.appendingPathComponent("IMG_0000.jpg"), maxPixel: 64))
        #expect(thumb.width <= 64)
        #expect(thumb.height <= 64)
        let full = try #require(
            DecodeEngine.decodeFull(url: shoot.folder.appendingPathComponent("IMG_0000.jpg")))
        #expect(full.width == 400)
        #expect(full.height == 300)
    }

    /// The byte-range read must produce the *same pixels* as reading the container.
    ///
    /// This is the whole of the todo.md §7.5 optimisation: hand ImageIO the JPEG's own bytes so it
    /// never parses the CR3. If the two paths disagreed by a pixel, a user would see one photograph
    /// in the viewer and a subtly different one after a cache eviction — which is exactly the kind
    /// of bug that only shows up under load and is impossible to report. So the range is compared
    /// against the container, pixel for pixel, on a real encoded JPEG.
    @Test("Decoding from a byte range gives the same image as decoding the container")
    func byteRangeDecodeMatchesTheContainerDecode() throws {
        let shoot = ImageFixtures.shoot(count: 1)
        let url = shoot.folder.appendingPathComponent("IMG_0000.jpg")
        let fromContainer = try #require(DecodeEngine.decodeFull(url: url))
        let length = UInt64(
            try #require(
                FileManager.default.attributesOfItem(atPath: url.path)[.size] as? NSNumber)
                .intValue)
        #expect(length > 100, "the fixture should be a real JPEG, not a stub")

        // The whole file is a range that covers exactly one JPEG, so it must give the same picture.
        let whole = ByteRange(offset: 0, len: length)
        let fromRange = try #require(DecodeEngine.decodeFull(byteRange: whole, in: url))
        #expect(fromRange.width == fromContainer.width)
        #expect(fromRange.height == fromContainer.height)
        #expect(
            identical(fromRange, fromContainer),
            "the byte-range path and the container path must be the same pixels")

        // Every range that is not a whole JPEG must be nil, and nil is what lets the caller fall
        // back. A torn or blank image here would be worse than no image.
        for range in [
            ByteRange(offset: 0, len: 2),  // the SOI and nothing else
            ByteRange(offset: length / 2, len: 2),  // two bytes from the middle
            ByteRange(offset: length, len: 10),  // starts past the end of the file
            ByteRange(offset: length + 1_000_000, len: 10),  // far past the end
            ByteRange(offset: 0, len: 0),  // empty
        ] {
            #expect(
                DecodeEngine.decodeFull(byteRange: range, in: url) == nil,
                "a partial or out-of-bounds range must not produce an image: \(range)")
        }

        // A file that does not exist at all, rather than a file with no JPEG in it.
        let missing = shoot.folder.appendingPathComponent("IMG_9999.jpg")
        #expect(DecodeEngine.decodeFull(byteRange: whole, in: missing) == nil)
    }

    /// A display decode is served from the reported byte range when there is one.
    @Test("A reported full-preview range is used, and the fallback still works without one")
    func displayPrefersTheReportedRange() async throws {
        let shoot = ImageFixtures.shoot(count: 1)
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20)
        var photos = shoot.photos
        // The fixture is a bare JPEG, so its "full preview" is the whole file.
        let size = (try FileManager.default.attributesOfItem(
            atPath: shoot.folder.appendingPathComponent("IMG_0000.jpg").path)[.size] as? NSNumber)
        photos[0].fullPreview = EmbeddedPreview(
            range: ByteRange(offset: 0, len: try #require(size).uint64Value),
            width: 400, height: 300)
        provider.open(folder: shoot.folder, photos: photos)

        #expect(provider.displayImage(for: photos[0].id) == nil, "a miss returns nil, never blocks")
        #expect(await provider.waitUntilIdle())
        #expect(try #require(provider.displayImage(for: photos[0].id)).width == 400)
        #expect(
            provider.byteRangeDecodes == 1,
            "the reported range should have served the decode, not the container")
    }
}

/// Two images are identical when every pixel of every row matches.
private func identical(_ lhs: CGImage, _ rhs: CGImage) -> Bool {
    guard lhs.width == rhs.width, lhs.height == rhs.height else { return false }
    func pixels(_ image: CGImage) -> [UInt8]? {
        guard
            let context = CGContext(
                data: nil, width: image.width, height: image.height, bitsPerComponent: 8,
                bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        guard let data = context.data else { return nil }
        let bytes = data.bindMemory(to: UInt8.self, capacity: image.width * image.height * 4)
        return Array(UnsafeBufferPointer(start: bytes, count: image.width * image.height * 4))
    }
    guard let a = pixels(lhs), let b = pixels(rhs) else { return false }
    return a == b
}
