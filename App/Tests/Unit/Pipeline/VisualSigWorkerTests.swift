import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers

@testable import Firstcut

@Suite("Visual signatures (todo.md §5.4 phase two)")
struct VisualSigWorkerTests {
    /// Writes a JPEG of a horizontal gradient (dark left to bright right) to a temp file.
    private func gradientJPEG(width: Int = 64, height: Int = 48, invert: Bool = false) throws -> URL {
        let space = CGColorSpace(name: CGColorSpace.sRGB)!
        let context = CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
            space: space, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
        for x in 0..<width {
            let level = CGFloat(invert ? width - 1 - x : x) / CGFloat(width - 1)
            context.setFillColor(red: level, green: level, blue: level, alpha: 1)
            context.fill(CGRect(x: x, y: 0, width: 1, height: height))
        }
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("sig-\(UUID().uuidString).jpg")
        let destination = CGImageDestinationCreateWithURL(
            url as CFURL, UTType.jpeg.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(destination, context.makeImage()!, nil)
        #expect(CGImageDestinationFinalize(destination))
        return url
    }

    @Test("A decoded thumbnail becomes an sRGB RGBA buffer of exactly the size the core expects")
    func rgbaLayout() throws {
        let url = try gradientJPEG(width: 40, height: 30)
        defer { try? FileManager.default.removeItem(at: url) }
        let image = try #require(DecodeEngine.decodeThumbnail(url: url, maxPixel: 256))
        let pixels = try #require(VisualSigWorker.rgbaPixels(of: image))
        #expect(pixels.bytes.count == pixels.width * pixels.height * 4)
        #expect(pixels.width > 0 && pixels.height > 0)
    }

    @Test("The signature comes from the Rust reference, and two different images differ")
    func signaturesComeFromTheCore() throws {
        let bright = try gradientJPEG()
        let flipped = try gradientJPEG(invert: true)
        defer {
            try? FileManager.default.removeItem(at: bright)
            try? FileManager.default.removeItem(at: flipped)
        }
        let a = try #require(VisualSigWorker.signature(for: 1, at: bright))
        let b = try #require(VisualSigWorker.signature(for: 2, at: flipped))
        #expect(a.0 == 1 && b.0 == 2)
        #expect(a.1.hist.count == 48)
        // A gradient one way and the same gradient the other way are not the same picture: the dHash
        // compares each pixel with its right-hand neighbour, so the two hashes are complementary.
        #expect(a.1.dhash != b.1.dhash)
        // The same file twice gives the same signature: deterministic, which resume relies on.
        let again = try #require(VisualSigWorker.signature(for: 1, at: bright))
        #expect(again.1 == a.1)
    }

    @Test("A file that cannot be decoded has no signature and costs nothing else")
    func undecodable() {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("nope-\(UUID()).cr3")
        #expect(VisualSigWorker.signature(for: 3, at: url) == nil)
    }

    @Test("The bridge refuses a buffer that does not match its size instead of crashing")
    func badBuffer() {
        #expect(FirstcutCoreBridge.visualSig(rgba: [0, 0, 0], width: 2, height: 2) == nil)
        #expect(FirstcutCoreBridge.visualSig(rgba: [], width: 0, height: 0) == nil)
    }
}

// MARK: - One decode, two consumers
//
// The signature pass used to decode every photograph in the shoot from the file URL, on top of the
// pipeline decoding the focus window. A CR3 costs ~300 ms (todo.md §3), so a 2,880-photo shoot was
// read and decoded twice, fighting the decodes the user is waiting for. These tests pin that the
// cache is consulted first and that a bitmap this pass decodes is published for the other one.

/// A stand-in for the pipeline's cache that counts what it was asked for and what it was given.
private final class CountingThumbnails: ThumbnailSource, @unchecked Sendable {
    private let lock = NSLock()
    private var cached: [PhotoID: CGImage] = [:]
    private var offered: [(id: PhotoID, size: UInt64)] = []

    func cachedThumbnail(for id: PhotoID) -> CGImage? {
        lock.lock()
        defer { lock.unlock() }
        return cached[id]
    }

    func offerThumbnail(_ id: PhotoID, image: CGImage, fileSize: UInt64) {
        lock.lock()
        defer { lock.unlock() }
        offered.append((id, fileSize))
        cached[id] = image
    }

    func prefill(_ image: CGImage, for id: PhotoID) {
        lock.lock()
        defer { lock.unlock() }
        cached[id] = image
    }

    var offeredSizes: [(id: PhotoID, size: UInt64)] {
        lock.lock()
        defer { lock.unlock() }
        return offered
    }
}

extension VisualSigWorkerTests {
    @Test("A cached thumbnail is used instead of decoding the file again")
    func cachedThumbnailIsReused() throws {
        let url = try gradientJPEG(width: 64, height: 48)
        defer { try? FileManager.default.removeItem(at: url) }
        let source = CountingThumbnails()
        // The pipeline has already decoded this one.
        let image = try #require(DecodeEngine.decodeThumbnail(url: url, maxPixel: 256))
        source.prefill(image, for: 7)

        let result = try #require(
            VisualSigWorker.signature(for: (id: 7, url: url, fileSize: 1024), thumbnails: source))
        #expect(result.0 == 7)
        #expect(
            source.offeredSizes.isEmpty,
            "a cached bitmap must not be published back; that would be a second copy for nothing")
    }

    @Test("A thumbnail this pass decodes is published so the filmstrip need not decode it")
    func decodedThumbnailIsPublished() throws {
        let url = try gradientJPEG(width: 64, height: 48)
        defer { try? FileManager.default.removeItem(at: url) }
        let source = CountingThumbnails()

        let result = try #require(
            VisualSigWorker.signature(for: (id: 9, url: url, fileSize: 4242), thumbnails: source))
        #expect(result.0 == 9)
        #expect(source.offeredSizes.count == 1, "the decode should be handed to the cache")
        #expect(source.offeredSizes.first?.id == 9)
        #expect(
            source.offeredSizes.first?.size == 4242,
            "the file size has to travel with it, or the cache cannot tell a replaced file from this one")
    }

    @Test("Both directions agree on the signature for the same photograph")
    func sharedDecodeAgreesWithAFreshOne() throws {
        let url = try gradientJPEG(width: 64, height: 48)
        defer { try? FileManager.default.removeItem(at: url) }
        let fresh = try #require(
            VisualSigWorker.signature(for: (id: 1, url: url, fileSize: 1), thumbnails: nil))

        // Now the same photograph, but its bitmap comes from the cache instead.
        let source = CountingThumbnails()
        let image = try #require(DecodeEngine.decodeThumbnail(url: url, maxPixel: 256))
        source.prefill(image, for: 1)
        let shared = try #require(
            VisualSigWorker.signature(for: (id: 1, url: url, fileSize: 1), thumbnails: source))

        // A different dHash would mean the shared decode is a different image, which would quietly
        // change where the batcher puts a boundary depending on cache timing.
        #expect(shared.1.dhash == fresh.1.dhash)
        #expect(shared.1.hist == fresh.1.hist)
    }

    @Test("A file that cannot be decoded yields no signature and publishes nothing")
    func anUndecodableFilePublishesNothing() throws {
        let source = CountingThumbnails()
        let missing = URL(fileURLWithPath: "/tmp/firstcut-no-such-file-\(UUID().uuidString).jpg")
        #expect(VisualSigWorker.signature(for: (id: 3, url: missing, fileSize: 1), thumbnails: source) == nil)
        #expect(source.offeredSizes.isEmpty)
    }
}
