// Contact-sheet renderer, ported from qa's `ContactSheet.swift` (the one senior-dev verified by
// eye: real composed sheets, right way up, labels legible). It is standalone rather than a test
// helper because `firstcut contact-sheet` drives it.
//
// Two details in here were hard-won and are load-bearing:
//
//   * The per-image y-flip. The context is flipped once, at the top, so the caption can be at the
//     top like a real contact sheet. `CGContext.draw` then puts every frame in *that* space
//     upside down. Undoing the flip per cell is what makes the sheet readable, and a test cannot
//     tell you a sheet is rotated 180° -- a human rules boundaries off these, so a sheet that reads
//     backwards produces wrong ground truth rather than a failure.
//
//   * Core Text, not `NSString.draw(in:withAttributes:)`. The AppKit one needs
//     `NSGraphicsContext.current`, which is main-thread-only state, and this runs off the main
//     thread. Core Text draws into the CGContext directly and is safe from any thread.
//
// Compiled standalone by `scripts/build-contact-sheet.sh`; `firstcut contact-sheet` invokes it.
// Kept out of the Xcode app target (see project.yml) for the same reason DecodeSpike is: top-level
// code is a syntax error in any Swift file that is not main.swift.

import AppKit
import CoreGraphics
import CoreText
import Foundation
import ImageIO
import UniformTypeIdentifiers

// MARK: - Input

/// One boundary to render, as produced by the Rust CLI. A flat struct so the boundary list can come
/// straight off stdin as JSON without this file knowing anything about the batcher.
struct Boundary: Decodable {
    let index: Int
    let game: String
    let photos: String
    let beforeName: String
    let afterName: String
    let gapMs: Int64
    let reason: String
    let frames: [String]
    /// Index **into `frames`** of the first frame after the boundary. -1 when unknown.
    let boundaryFrame: Int?
}

struct Request: Decodable {
    let outDir: String
    let cellSize: Int?
    let columns: Int?
    let boundaries: [Boundary]
}

enum SheetError: Error, CustomStringConvertible {
    case badInput(String)
    case unreadable(String)

    var description: String {
        switch self {
        case .badInput(let m): "contact-sheet: \(m)"
        case .unreadable(let m): "contact-sheet: could not render \(m)"
        }
    }
}

// MARK: - Render

enum ContactSheet {
    /// Big enough to judge the motion-blur difference between adjacent 90 ms frames, which is the
    /// difference that decides whether a fast burst is one play or several.
    static let defaultCellSize = 320
    static let defaultColumns = 4

    static func run() throws {
        let raw = FileHandle.standardInput.readDataToEndOfFile()
        guard !raw.isEmpty else {
            throw SheetError.badInput(
                "expected a JSON request on stdin (see `firstcut contact-sheet --help`)")
        }
        let request = try JSONDecoder().decode(Request.self, from: raw)

        let cell = max(64, request.cellSize ?? defaultCellSize)
        let columns = max(1, request.columns ?? defaultColumns)
        let outDir = URL(fileURLWithPath: (request.outDir as NSString).expandingTildeInPath)
        try FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)

        var rendered = 0
        var failed: [String] = []
        for boundary in request.boundaries {
            do {
                let url = try render(
                    boundary, outDir: outDir, cell: cell, columns: columns)
                rendered += 1
                print(url.path)
            } catch {
                failed.append("\(boundary.game) #\(boundary.index): \(error)")
            }
        }

        // Always write the plan, even when every sheet failed: it is the hand-off list, and a human
        // needs to know which boundaries still need ruling on.
        let plan: [String: Any] = [
            "schema": "contact-sheet/1",
            "rendered": rendered,
            "failed": failed.count,
            "outDir": outDir.path,
            "note": """
                Every sheet above is one boundary the batcher could not decide on metadata alone. \
                A human rules on each: same play or not. Write the answer to \
                tests/fixtures/ground-truth/<game>.json and score it with `firstcut eval`. \
                Nothing in this file is ground truth on its own.
                """,
            "boundaries": request.boundaries.map { boundary in
                [
                    "index": boundary.index,
                    "game": boundary.game,
                    "before": boundary.beforeName,
                    "after": boundary.afterName,
                    "gapMs": boundary.gapMs,
                    "reason": boundary.reason,
                    "frames": boundary.frames,
                ]
            },
        ]
        let planURL = outDir.appendingPathComponent("plan.json")
        let data = try JSONSerialization.data(
            withJSONObject: plan, options: [.prettyPrinted, .sortedKeys])
        try (data + Data("\n".utf8)).write(to: planURL)
        FileHandle.standardError.write(
            Data("rendered \(rendered)/\(request.boundaries.count) sheets, plan: \(planURL.path)\n"
                .utf8))

        if rendered == 0, let first = failed.first {
            throw SheetError.unreadable("\(request.boundaries.count) sheets failed; first: \(first)")
        }
    }

    static func render(
        _ boundary: Boundary, outDir: URL, cell: Int, columns: Int
    ) throws -> URL {
        let folder = URL(fileURLWithPath: (boundary.photos as NSString).expandingTildeInPath)
        guard FileManager.default.fileExists(atPath: folder.path) else {
            throw SheetError.unreadable("\(boundary.photos) is not a folder")
        }

        let names = boundary.frames
        let boundaryIndex = max(0, boundary.boundaryFrame ?? names.count / 2)
        let labelHeight = 22
        let captionHeight = 34
        let rows = Int(ceil(Double(names.count) / Double(columns)))
        let width = columns * cell
        let height = captionHeight + rows * (cell + labelHeight)

        guard
            let context = CGContext(
                data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.noneSkipFirst.rawValue
                    | CGBitmapInfo.byteOrder32Little.rawValue)
        else { throw SheetError.unreadable("a \(width)×\(height) context") }

        // Origin is bottom-left in CG; flip once so the caption is at the top.
        context.setFillColor(CGColor(red: 0.10, green: 0.10, blue: 0.11, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: CGFloat(width), height: CGFloat(height)))
        context.translateBy(x: 0, y: CGFloat(height))
        context.scaleBy(x: 1, y: -1)

        draw(
            "\(boundary.game)  #\(boundary.index)  \(boundary.beforeName) → \(boundary.afterName)"
                + "   Δt \(boundary.gapMs) ms   [\(boundary.reason)]",
            in: CGRect(x: 8, y: 8, width: CGFloat(width) - 16, height: CGFloat(captionHeight) - 10),
            font: .boldSystemFont(ofSize: 15), in: context)

        for (index, name) in names.enumerated() {
            let column = index % columns
            let row = index / columns
            // The boundary frame is the hinge of the sheet: it is the last frame of the previous
            // burst and the first of the next. Mark it, because with 7 frames across 4 columns it
            // lands mid-grid and is otherwise indistinguishable from its neighbours -- which is
            // exactly the judgement the human is being asked to make.
            let originX = CGFloat(column * cell)
            let originY = CGFloat(captionHeight + row * (cell + labelHeight))

            context.setFillColor(CGColor(red: 0.16, green: 0.16, blue: 0.17, alpha: 1))
            context.fill(
                CGRect(x: originX, y: originY, width: CGFloat(cell), height: CGFloat(cell)))

            if let image = downsample(url: folder.appendingPathComponent(name), to: cell) {
                // Undo the global flip for the image only, around the cell's own origin. Without
                // this every frame renders rotated 180° and the sheet reads backwards.
                context.saveGState()
                context.translateBy(x: originX, y: originY + CGFloat(cell))
                context.scaleBy(x: 1, y: -1)
                context.draw(
                    image,
                    in: CGRect(x: 0, y: 0, width: CGFloat(cell), height: CGFloat(cell)))
                context.restoreGState()
            }

            // A bright rule under the boundary frame, so "which two frames is this?" is answered by
            // the sheet rather than by counting cells.
            if index == boundaryIndex {
                context.setFillColor(CGColor(red: 0.20, green: 0.85, blue: 0.55, alpha: 1))
                context.fill(
                    CGRect(
                        x: originX, y: originY + CGFloat(cell) - 4, width: CGFloat(cell), height: 4))
            }

            context.setFillColor(CGColor(red: 0.10, green: 0.10, blue: 0.11, alpha: 1))
            context.fill(
                CGRect(
                    x: originX, y: originY + CGFloat(cell), width: CGFloat(cell),
                    height: CGFloat(labelHeight)))
            draw(
                index == boundaryIndex ? "-> \(name)" : name,
                in: CGRect(
                    x: originX + 4, y: originY + CGFloat(cell) + 3, width: CGFloat(cell) - 8,
                    height: CGFloat(labelHeight) - 4),
                font: .monospacedSystemFont(ofSize: 11, weight: .regular), in: context)
        }

        guard let image = context.makeImage() else {
            throw SheetError.unreadable("\(boundary.game) #\(boundary.index) raster")
        }
        let destination = outDir.appendingPathComponent(
            "boundary-\(boundary.game)-\(String(format: "%05d", boundary.index)).jpg")
        guard
            let dest = CGImageDestinationCreateWithURL(
                destination as CFURL, UTType.jpeg.identifier as CFString, 1, nil)
        else { throw SheetError.unreadable(destination.path) }
        CGImageDestinationAddImage(
            dest, image, [kCGImageDestinationLossyCompressionQuality: 0.85] as CFDictionary)
        guard CGImageDestinationFinalize(dest) else {
            throw SheetError.unreadable(destination.path)
        }
        return destination
    }

    /// Flipped text into a y-down context. Core Text, so it is safe off the main thread.
    private static func draw(_ string: String, in rect: CGRect, font: NSFont, in context: CGContext)
    {
        let attributed = NSAttributedString(
            string: string, attributes: [.font: font, .foregroundColor: NSColor.white])
        let line = CTLineCreateWithAttributedString(attributed)
        context.saveGState()
        context.textMatrix = .identity
        context.scaleBy(x: 1, y: -1)
        context.textPosition = CGPoint(x: rect.minX, y: -rect.maxY)
        CTLineDraw(line, context)
        context.restoreGState()
    }

    /// Subsampled decode straight from the CR3's embedded preview, so this reads a fraction of the
    /// 15 MB file rather than decoding RAW.
    private static func downsample(url: URL, to pixelSize: Int) -> CGImage? {
        guard
            let source = CGImageSourceCreateWithURL(
                url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary)
        else { return nil }
        return CGImageSourceCreateThumbnailAtIndex(
            source, 0,
            [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceThumbnailMaxPixelSize: pixelSize,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceShouldCacheImmediately: true,
            ] as CFDictionary)
    }
}

try ContactSheet.run()
