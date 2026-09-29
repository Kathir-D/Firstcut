// Owner: app-logic.
//
// A minimal mirror of the parts of [pipeline-api.md] that app-logic actually calls, so the model can
// be built and tested before pipeline's decoder exists.
//
// This is **not** a second opinion on pipeline's API — it's a stub with the smallest possible
// surface: the model only ever says "here is what the user can reach right now", and reads nothing
// back. When pipeline lands, infra replaces this file with the real types; the call site
// (`AppModel.updatePipelineFocus()`) is the only thing that changes (REQ-app-logic-1).
//
// Deliberately *not* mirrored: `displayImage`, `fullImage`, `histogram`, `clippingMask`, `stats`,
// `thumbnailProgress` — all of those are ui's and qa's to consume, straight from pipeline.

import CoreGraphics
import Foundation

/// The photo IDs of one batch, in capture order, as the scheduler needs them.
public struct FocusWindow: Hashable, Sendable {
    public var batchID: BatchID
    public var photoIDs: [PhotoID]

    public init(batchID: BatchID, photoIDs: [PhotoID]) {
        self.batchID = batchID
        self.photoIDs = photoIDs
    }
}

/// "What the user can reach next", recomputed on **every** navigation (pipeline-api.md).
public struct FocusRequest: Hashable, Sendable {
    public var windows: [FocusWindow]  // previous, current, next, then as far as the budget allows
    public var currentPhoto: PhotoID
    /// Backing pixels of the viewer area, Retina aware. Zero until the window reports a size.
    public var viewportPixelSize: CGSize
    public var zoomed: Bool
    public var zoomLock: Bool
    public var exactRaw: Bool

    public init(
        windows: [FocusWindow],
        currentPhoto: PhotoID,
        viewportPixelSize: CGSize = .zero,
        zoomed: Bool = false,
        zoomLock: Bool = false,
        exactRaw: Bool = false
    ) {
        self.windows = windows
        self.currentPhoto = currentPhoto
        self.viewportPixelSize = viewportPixelSize
        self.zoomed = zoomed
        self.zoomLock = zoomLock
        self.exactRaw = exactRaw
    }

    /// The previous/current/next window, which is the set that must never miss the cache (§7.1).
    public var guaranteedWindows: [FocusWindow] { Array(windows.prefix(3)) }
}

@MainActor public protocol ImageProviding: AnyObject {
    func setFocus(_ focus: FocusRequest)
}

/// Deterministic stand-in used by `AppModel.preview(game:)` and every test: it records what the model
/// asked for and paints a stable colour per photo so the filmstrip isn't a wall of grey.
@MainActor public final class MockImageProvider: ImageProviding {
    public private(set) var focusHistory: [FocusRequest] = []
    /// How many batches around the current one the model asked to keep ready.
    public var lookAheadBatches: Int

    public init(lookAheadBatches: Int = 2) {
        self.lookAheadBatches = lookAheadBatches
    }

    public var lastFocus: FocusRequest? { focusHistory.last }
    public var focusCount: Int { focusHistory.count }

    public func setFocus(_ focus: FocusRequest) {
        focusHistory.append(focus)
        if focusHistory.count > 64 { focusHistory.removeFirst() }
    }

    public func reset() {
        focusHistory.removeAll()
    }

    /// A stable colour per photo, so `photo.next` is visible in a screenshot.
    public nonisolated static func color(for id: PhotoID) -> CGColor {
        var hash = id &* 0x9E37_79B9_7F4A_7C15
        hash ^= hash >> 29
        let r = Double((hash >> 3) & 0xFF) / 255.0
        let g = Double((hash >> 11) & 0xFF) / 255.0
        let b = Double((hash >> 19) & 0xFF) / 255.0
        return CGColor(srgbRed: 0.25 + r * 0.6, green: 0.25 + g * 0.6, blue: 0.25 + b * 0.6, alpha: 1)
    }

    public nonisolated static func thumbnail(for id: PhotoID, size: CGSize = CGSize(width: 96, height: 64)) -> CGImage? {
        let width = Int(size.width), height = Int(size.height)
        guard width > 0, height > 0 else { return nil }
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        let color = Self.color(for: id)
        let r = UInt8(color.components![0] * 255)
        let g = UInt8(color.components![1] * 255)
        let b = UInt8(color.components![2] * 255)
        for y in 0..<height {
            for x in 0..<(width / 4) {  // a bar on the left encodes the id, so frames differ visibly
                let offset = (y * width + x) * 4
                pixels[offset] = r
                pixels[offset + 1] = g
                pixels[offset + 2] = b
                pixels[offset + 3] = 255
            }
            for x in (width / 4)..<width {
                let offset = (y * width + x) * 4
                pixels[offset] = UInt8(min(255, Int(r) / 2))
                pixels[offset + 1] = UInt8(min(255, Int(g) / 2))
                pixels[offset + 2] = UInt8(min(255, Int(b) / 2))
                pixels[offset + 3] = 255
            }
        }
        guard let context = CGContext(
            data: &pixels, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        return context.makeImage()
    }
}
