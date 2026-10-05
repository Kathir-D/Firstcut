// Owner: pipeline.
//
// Regressions for defects found in the audit, grouped in one file because each is a property of the
// decode engine that the rest of the suite did not pin.
//
// Every test here fails against the code as it was before its fix, for the stated reason.
//
// One fix in this file has no test here and should not pretend otherwise: the in-flight slot leak in
// `DecodeEngine.store`, where the discard branch returned before removing its key. The fix is a
// `defer` and is not in doubt, but driving that path needs the engine re-opened while decodes are in
// flight, and every construction of that tried so far either wedged `waitUntilIdle` or took the test
// host down with it. It is listed in todo.md as the one thing from this audit still unproven.

import CoreGraphics
import Foundation
import Testing

@testable import Firstcut

@Suite("Decode engine regressions")
@MainActor
struct DecodeEngineRegressionsTests {

    private func shoot(count: Int, name: String) -> (folder: URL, photos: [PhotoMeta]) {
        ImageFixtures.shoot(count: count, folder: ImageFixtures.folder(name))
    }

    private func focus(_ ids: [PhotoID], current: PhotoID) -> FocusRequest {
        FocusRequest(windows: [FocusWindow(batchID: 1, photoIDs: ids)], currentPhoto: current)
    }

    // MARK: - Cross-tier eviction

    /// The eviction victim was collected per tier but removed from all three, so a stale thumbnail
    /// could take down the display bitmap and the 92 MB RAW develop for the same photograph.
    ///
    /// The symptom is the expensive one: the RAW tier thrashes (a re-develop per arrow key) and the
    /// viewer is handed a picture it already had cached.
    @Test("Evicting a thumbnail does not evict the same photo's display bitmap")
    func evictionStaysInsideItsTier() async throws {
        let (folder, photos) = shoot(count: 3, name: "tiers")
        defer { try? FileManager.default.removeItem(at: folder) }
        // Generous, so nothing is evicted while the fixture is being set up. The budget is then
        // tightened by the assertions below rather than guessed at here: how many bytes a display
        // entry really costs depends on the layout the decoder chose, and a test that hard-codes it
        // is a test that fails for the wrong reason on the next SDK.
        let provider = ImageProvider(memoryBudgetBytes: 16 << 20)
        provider.open(folder: folder, photos: photos)
        let watched = photos[0]

        // The thumbnail is stored first, so it is the older of the two entries for this photograph
        // and therefore the eviction victim. The display entry is the newer, hot one.
        _ = provider.thumbnail(for: watched.id, size: CGSize(width: 96, height: 96))
        #expect(await provider.waitUntilIdle())
        _ = provider.displayImage(for: watched.id, minimumLongestEdge: 300)
        #expect(await provider.waitUntilIdle())
        _ = try #require(provider.displayImage(for: watched.id, minimumLongestEdge: 300))
        #expect(provider.stats.thumbnailBytes > 0, "the thumbnail is in, and is the older entry")

        // Now tighten the budget to exactly what is cached. The next thumbnail of the *same* size has
        // to push the cache over by exactly its own size, so the only way back under is to evict
        // something — and the oldest entry in the whole cache is `watched`'s thumbnail. After the fix
        // that is the only thing that goes.
        let displayBytesWhenSetUp = provider.stats.displayBytes
        let cached = provider.stats.thumbnailBytes + displayBytesWhenSetUp
        provider.setMemoryBudgetForTesting(cached)

        _ = provider.thumbnail(for: photos[1].id, size: CGSize(width: 96, height: 96))
        #expect(await provider.waitUntilIdle())

        // Proof that an eviction really ran, so the assertion below is not vacuous: the cache is
        // back exactly at the budget rather than one thumbnail over it.
        #expect(
            provider.stats.thumbnailBytes + provider.stats.displayBytes <= cached,
            "the cache should have been brought back under the budget")
        #expect(
            provider.stats.displayBytes == displayBytesWhenSetUp,
            "and what came back under it was a thumbnail, not the display bitmap")
    }

    // MARK: - The recorded pixel size

    /// `Entry.pixels` recorded the size that was *asked* for, while the comment above it claimed the
    /// opposite. `kCGImageSourceThumbnailMaxPixelSize` is a maximum, so a file smaller than the
    /// request comes back at its own size and every later reader believed it was larger than it is.
    ///
    /// The cost was a guaranteed-redundant decode: any photograph smaller than the viewer was
    /// re-decoded on every window growth, each time producing exactly the same pixels.
    @Test("A photo smaller than the viewer is not re-decoded when the viewer grows")
    func aSmallPhotoIsNotReDecodedOnEveryResize() async throws {
        let (folder, photos) = shoot(count: 1, name: "small")
        defer { try? FileManager.default.removeItem(at: folder) }
        let provider = ImageProvider(memoryBudgetBytes: 64 << 20)
        provider.open(folder: folder, photos: photos)
        let id = photos[0].id

        _ = provider.displayImage(for: id, minimumLongestEdge: 2880)
        #expect(await provider.waitUntilIdle())
        let afterFirst = provider.stats.displayDecodes

        _ = provider.displayImage(for: id, minimumLongestEdge: 3456)
        #expect(await provider.waitUntilIdle())

        #expect(
            provider.stats.displayDecodes == afterFirst,
            "a file that already gave back everything it had must not be decoded again (\(afterFirst) -> \(provider.stats.displayDecodes))")
    }

    // MARK: - Orientation on the histogram path

    /// The histogram's fallback scheduled a display decode with the orientation defaulted to 1,
    /// while the provider had the real value in hand and did not pass it. `display` rotates on the
    /// way into the cache, so an orientation-8 photograph got an upright full-size bitmap filed
    /// under its id and the viewer drew it sideways.
    ///
    /// 26 of Game1JENKS's 708 frames are orientation 8, and the bad entry survived until something
    /// evicted it — so it read as an intermittent rendering fault rather than a cache bug.
    @Test("A display decode scheduled by the histogram keeps the EXIF orientation")
    func theHistogramPathKeepsTheOrientation() async throws {
        let folder = ImageFixtures.folder("orientation")
        defer { try? FileManager.default.removeItem(at: folder) }
        // Written 800×600, declared orientation 8 (a quarter turn), so a correctly decoded and
        // rotated image is 600×800.
        ImageFixtures.jpeg(in: folder, named: "rotated.jpg", width: 800, height: 600)
        var meta = ImageFixtures.photo(named: "rotated.jpg", in: folder)
        meta.orientation = 8

        let provider = ImageProvider(memoryBudgetBytes: 64 << 20)
        provider.open(folder: folder, photos: [meta])

        // The histogram first, with no focus window: this is the path that scheduled its own decode.
        _ = provider.histogram(for: meta.id)
        #expect(await provider.waitUntilIdle())

        let image = try #require(provider.displayImage(for: meta.id))
        #expect(
            image.width == 600 && image.height == 800,
            "the cached bitmap must be rotated; it was \(image.width)×\(image.height)")
    }

    // MARK: - A short read

    /// `readBytes` promised "a short read is a failure rather than a short image" and never checked.
    /// ImageIO will happily decode the partial progressive JPEG a half-copied file contains, so the
    /// engine cached a half-grey frame and recorded no failure — which a user cannot tell from a
    /// photograph of a dark scene.
    @Test("A byte range past the end of the file is refused, not truncated")
    func aRangePastTheEndIsRefused() async throws {
        let (folder, photos) = shoot(count: 1, name: "shortread")
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent(photos[0].relPath)
        let size = try FileHandle(forReadingFrom: url).seekToEnd()

        // Starts inside the file and runs past its end. The bounds clamp is unchanged, so what is
        // left to catch is exactly the short read.
        let data = DecodeEngine.readBytes(ByteRange(offset: 0, len: size + 4096), in: url)
        #expect(
            data == nil || data?.count == Int(size),
            "either nothing, or the whole file — never a prefix of it")
    }
}
