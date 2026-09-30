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
