// EmbeddedPreviewFallbackTests: a RAW ImageIO cannot open is drawn from the JPEG the core found
// inside it, and a file ImageIO *can* open never is.

import CoreGraphics
import Foundation
import Testing

@testable import Firstcut

@Suite("Embedded preview fallback")
struct EmbeddedPreviewFallbackTests {
    /// A file ImageIO does not recognise, with a real JPEG at a known offset: what a Leaf MOS or a
    /// Phase One IIQ looks like to macOS.
    private static func undecodableRaw(
        in folder: URL, named name: String
    ) throws -> (url: URL, range: ByteRange) {
        let jpeg = try Data(
            contentsOf: ImageFixtures.jpeg(in: folder, named: "\(name).jpg", width: 320, height: 240))
        var bytes = Data(repeating: 0x5a, count: 4096)
        let offset = bytes.count
        bytes.append(jpeg)
        bytes.append(Data(repeating: 0x5a, count: 512))
        let url = folder.appendingPathComponent(name)
        try bytes.write(to: url)
        return (url, ByteRange(offset: UInt64(offset), len: UInt64(jpeg.count)))
    }

    private static func meta(_ name: String, preview range: ByteRange?, orientation: UInt8 = 1)
        -> PhotoMeta
    {
        PhotoMeta(
            id: 1, relPath: name, companions: [], kind: .raw(.iiq),
            fileSize: 1024, captureTime: nil, shutterCount: nil, fileNumber: nil,
            cameraMake: nil, cameraModel: nil, cameraSerial: nil, lensModel: nil,
            focalLengthMm: nil, exposureTimeS: nil, fNumber: nil, iso: nil, exposureCompEv: nil,
            meteringMode: nil, driveMode: nil, shutterMode: nil, orientation: orientation,
            width: 7000, height: 5000, af: nil,
            preview: range.map { EmbeddedPreview(range: $0, width: 320, height: 240) },
            fullPreview: nil, warnings: [])
    }

    @Test("A RAW ImageIO cannot open is drawn from its embedded JPEG, the right way up")
    func anUndecodableRawUsesItsPreview() throws {
        let folder = ImageFixtures.folder("fallback")
        let raw = try Self.undecodableRaw(in: folder, named: "CF000001.IIQ")
        #expect(DecodeEngine.decodeThumbnail(url: raw.url, maxPixel: 128) == nil)

        EmbeddedPreviewFallback.shared.replace(
            folder: folder, photos: [Self.meta("CF000001.IIQ", preview: raw.range, orientation: 6)])
        defer { EmbeddedPreviewFallback.shared.remove(folder: folder) }

        let thumbnail = try #require(DecodeEngine.decodeThumbnail(url: raw.url, maxPixel: 128))
        // Orientation 6 is a quarter turn: the 320×240 JPEG comes out portrait.
        #expect(thumbnail.width < thumbnail.height)
        #expect(max(thumbnail.width, thumbnail.height) == 128)

        let display = try #require(
            DecodeEngine.decodeDisplay(url: raw.url, orientation: 1, maxPixel: 4096))
        #expect((display.width, display.height) == (320, 240))
    }

    @Test("A file ImageIO opens is never drawn from the preview instead")
    func aDecodableFileIgnoresThePreview() throws {
        let folder = ImageFixtures.folder("fallback-decodable")
        let url = ImageFixtures.jpeg(in: folder, named: "IMG_0001.JPG", width: 400, height: 300)
        // A preview range that is not a JPEG at all: using it would fail, so success proves it was
        // not used.
        EmbeddedPreviewFallback.shared.replace(
            folder: folder,
            photos: [Self.meta("IMG_0001.JPG", preview: ByteRange(offset: 0, len: 4))])
        defer { EmbeddedPreviewFallback.shared.remove(folder: folder) }

        let display = try #require(DecodeEngine.decodeDisplay(url: url, maxPixel: 4096))
        #expect((display.width, display.height) == (400, 300))
    }

    @Test("Opening another folder drops only the previous one")
    func foldersDoNotWipeEachOther() throws {
        let first = ImageFixtures.folder("fallback-a")
        let second = ImageFixtures.folder("fallback-b")
        let other = ImageFixtures.folder("fallback-c")
        let range = ByteRange(offset: 0, len: 4)
        let table = EmbeddedPreviewFallback()
        table.replace(folder: first, photos: [Self.meta("A.IIQ", preview: range)])
        table.replace(folder: other, photos: [Self.meta("C.IIQ", preview: range)])
        table.replace(
            folder: second, photos: [Self.meta("B.IIQ", preview: range)], previous: first)

        #expect(table.entry(for: first.appendingPathComponent("A.IIQ")) == nil)
        #expect(table.entry(for: second.appendingPathComponent("B.IIQ")) != nil)
        #expect(table.entry(for: other.appendingPathComponent("C.IIQ")) != nil)
        table.remove(folder: other)
        #expect(table.entry(for: other.appendingPathComponent("C.IIQ")) == nil)
    }
}
