// Owner: qa.
//
// Contact sheets, so a boundary can be judged by looking at the photos (task.md §12) instead of by
// trusting a timestamp. Built on ImageIO, which reads the full-resolution JPEG preview embedded in
// each CR3 — the same path the app's own pipeline uses, and 100× cheaper than a RAW decode.
//
// Provenance of the types in this file, per REV-56: `Game`, `ExifToolRecord` and `Fixtures` in
// FirstcutTestSupport.swift are qa's own test helpers, not stand-ins for any contract type. They
// read `tests/fixtures/exiftool/<game>.json`, which is raw exiftool output, and are deleted with
// the harness. Nothing here imports `CoreTypes.swift`, so the UniFFI swap (REV-7) cannot break it.
//
// Output is NEVER written into the repo: contact sheets are derived from photos that must not be
// committed. `AuditOutput.directory` is `$FIRSTCUT_QA_AUDIT_DIR`, defaulting to a temp directory.

import CoreGraphics
import Foundation
import AppKit
import ImageIO
import UniformTypeIdentifiers

/// One candidate boundary to audit: the frames on either side of it, plus why qa picked it.
public struct CandidateBoundary: Sendable {
    public let game: Game
    /// Index of the last photo **before** the boundary in capture order.
    public let beforeIndex: Int
    /// Index of the first photo **after** the boundary.
    public let afterIndex: Int
    public let beforeName: String
    public let afterName: String
    public let gapSeconds: Double
    /// Why this boundary was sampled: "gap" (Δt in the ambiguous band), "orientation" (EXIF
    /// rotation change), "shutter" (ShutterCount jump > 1, i.e. frames deleted in camera), "tail"
    /// (inside the fast/ambiguous tail called out in task.md §3 and §5.4).
    public let reason: String

    public var title: String {
        "\(game.rawValue) #\(beforeIndex + 1)|\(afterIndex + 1) \(beforeName)→\(afterName) "
            + "Δt \(String(format: "%.3f", gapSeconds))s [\(reason)]"
    }
}

/// Where audit artefacts go. Never inside the repository.
public enum AuditOutput {
    public static let directoryEnvVar = "FIRSTCUT_QA_AUDIT_DIR"

    public static var directory: URL {
        if let raw = ProcessInfo.processInfo.environment[directoryEnvVar], !raw.isEmpty {
            return URL(fileURLWithPath: (NSString(string: raw).expandingTildeInPath))
        }
        return URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("firstcut-qa-audit", isDirectory: true)
    }

    @discardableResult
    public static func prepare() throws -> URL {
        try FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: true)
        return directory
    }
}

public enum ContactSheetError: Error, CustomStringConvertible {
    case unreadable(String)

    public var description: String {
        switch self {
        case .unreadable(let name): "ImageIO could not read \(name)"
        }
    }
}

public enum ContactSheet {
    /// One cell, with the index and file name drawn under it. Cells are big enough to judge a
    /// motion blur difference between adjacent 90 ms frames — that difference is what decides
    /// whether a fast burst is one play or several.
    public static let defaultCellSize = 320
    public static let defaultColumns = 4

    /// Renders `photos` (in the order given) into a single labelled JPEG.
    /// - Parameters:
    ///   - photoNames: file names to render, in capture order.
    ///   - labels: optional per-cell caption; defaults to the file name.
    ///   - caption: a header line, e.g. the boundary title.
    public static func render(
        game: Game,
        photoNames: [String],
        labels: [String]? = nil,
        caption: String,
        columns: Int = defaultColumns,
        cellSize: Int = defaultCellSize,
        outputName: String
    ) throws -> URL {
        guard let folder = game.photos else {
            throw ContactSheetError.unreadable("\(game.rawValue) folder is not available")
        }
        let columns = max(1, columns)
        let cell = max(64, cellSize)
        let labelHeight = 22
        let captionHeight = 34
        let rows = Int(ceil(Double(photoNames.count) / Double(columns)))
        let width = columns * cell
        let height = captionHeight + rows * (cell + labelHeight)

        guard
            let context = CGContext(
                data: nil,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.noneSkipFirst.rawValue
                    | CGBitmapInfo.byteOrder32Little.rawValue)
        else { throw ContactSheetError.unreadable("could not create a \(width)×\(height) context") }

        // Origin is bottom-left in CG; flip so the caption is at the top like a real contact sheet.
        context.setFillColor(CGColor(red: 0.10, green: 0.10, blue: 0.11, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: CGFloat(width), height: CGFloat(height)))
        context.translateBy(x: 0, y: CGFloat(height))
        context.scaleBy(x: 1, y: -1)

        Self.draw(
            caption,
            in: CGRect(x: 8, y: 8, width: CGFloat(width) - 16, height: CGFloat(captionHeight) - 10),
            font: .boldSystemFont(ofSize: 15),
            in: context
        )

        for (index, name) in photoNames.enumerated() {
            let column = index % columns
            let row = index / columns
            let originX = CGFloat(column * cell)
            let originY = CGFloat(captionHeight + row * (cell + labelHeight))
            let url = folder.appendingPathComponent(name)

            context.setFillColor(CGColor(red: 0.16, green: 0.16, blue: 0.17, alpha: 1))
            context.fill(
                CGRect(x: originX, y: originY, width: CGFloat(cell), height: CGFloat(cell)))

            if let image = downsample(url: url, to: cell) {
                context.draw(
                    image,
                    in: CGRect(x: originX, y: originY, width: CGFloat(cell), height: CGFloat(cell)))
            }

            let captionText = labels?[safe: index] ?? name
            context.setFillColor(CGColor(red: 0.10, green: 0.10, blue: 0.11, alpha: 1))
            context.fill(
                CGRect(x: originX, y: originY + CGFloat(cell), width: CGFloat(cell), height: CGFloat(labelHeight)))
            Self.draw(
                captionText,
                in: CGRect(x: originX + 4, y: originY + CGFloat(cell) + 3, width: CGFloat(cell) - 8, height: CGFloat(labelHeight) - 4),
                font: .monospacedSystemFont(ofSize: 11, weight: .regular),
                in: context
            )
        }

        guard let output = context.makeImage() else {
            throw ContactSheetError.unreadable("could not rasterise the sheet")
        }
        let destination = try AuditOutput.prepare()
            .appendingPathComponent("\(outputName).jpg")
        guard
            let dest = CGImageDestinationCreateWithURL(
                destination as CFURL, UTType.jpeg.identifier as CFString, 1, nil)
        else { throw ContactSheetError.unreadable(destination.path) }
        CGImageDestinationAddImage(
            dest, output,
            [kCGImageDestinationLossyCompressionQuality: 0.85] as CFDictionary)
        guard CGImageDestinationFinalize(dest) else {
            throw ContactSheetError.unreadable(destination.path)
        }
        return destination
    }

    /// Draws flipped text into a context whose coordinate system is y-down (see the flip above).
    private static func draw(_ string: String, in rect: CGRect, font: NSFont, in context: CGContext) {
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
        (string as NSString).draw(
            in: rect,
            withAttributes: [
                .font: font,
                .foregroundColor: NSColor.white,
            ])
        NSGraphicsContext.restoreGraphicsState()
    }

    /// Thumbnail-quality decode straight from the embedded preview. `kCGImageSourceThumbnailMaxPixelSize`
    /// lets ImageIO subsample during decode, so this reads a fraction of the 15 MB file — the same
    /// trick §7.3's thumbnail budget depends on.
    private static func downsample(url: URL, to pixelSize: Int) -> CGImage? {
        guard
            let source = CGImageSourceCreateWithURL(url as CFURL, [
                kCGImageSourceShouldCache: false
            ] as CFDictionary)
        else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceThumbnailMaxPixelSize: pixelSize,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }
}

extension Array {
    fileprivate subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
