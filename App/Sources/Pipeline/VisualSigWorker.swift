// Owner: pipeline.
//
// Phase two of batching (task.md §5.4): the shoot is first split from metadata alone, instantly, and
// then the boundaries that timing cannot decide are refined by how the frames *look*. This computes
// that "look" for every photo in the background and hands it to the core in chunks, so unvisited
// batches sharpen while the user is already culling.
//
// The signature itself is computed by the Rust reference implementation (REV-64). This file only
// does what Rust cannot: decode a 256 px thumbnail through ImageIO and hand its pixels across.
//
// It runs at `.utility` priority with a small concurrency cap, so it never competes with the
// decodes the user is waiting on, and it starts at the photo the user is on and works outward so
// the batches they are about to reach are refined first.

import CoreGraphics
import Foundation

@MainActor
final class VisualSigWorker {
    /// Photos per hand-off. Each hand-off may re-batch, so it is neither per photo (a re-batch per
    /// photo is wasted work) nor the whole shoot (nothing would refine until the very end).
    static let chunkSize = 96
    /// Decodes in flight at once. Well under the core count: this is background work.
    static let maxConcurrent = 3

    private var task: Task<Void, Never>?

    /// Starts (or restarts) the background pass. `submit` is called on the main actor with each
    /// finished chunk.
    func start(
        photos: [PhotoMeta], folder: URL, startingAt index: Int,
        submit: @escaping @MainActor ([(PhotoID, VisualSig)]) -> Void
    ) {
        cancel()
        guard !photos.isEmpty else { return }
        let from = min(max(0, index), photos.count - 1)
        let ordered = Array(photos[from...]) + Array(photos[..<from])
        let work = ordered.map { (id: $0.id, url: folder.appendingPathComponent($0.relPath)) }

        task = Task.detached(priority: .utility) {
            var offset = 0
            while offset < work.count {
                if Task.isCancelled { return }
                let slice = Array(work[offset..<min(offset + Self.chunkSize, work.count)])
                offset += slice.count
                let sigs = await Self.signatures(for: slice)
                if Task.isCancelled { return }
                if !sigs.isEmpty {
                    await MainActor.run { submit(sigs) }
                }
            }
        }
    }

    func cancel() {
        task?.cancel()
        task = nil
    }

    // MARK: - Off the main actor

    /// One chunk, decoded with at most `maxConcurrent` in flight.
    private nonisolated static func signatures(
        for slice: [(id: PhotoID, url: URL)]
    ) async -> [(PhotoID, VisualSig)] {
        var results: [(PhotoID, VisualSig)] = []
        await withTaskGroup(of: (PhotoID, VisualSig)?.self) { group in
            var next = 0
            for _ in 0..<min(maxConcurrent, slice.count) {
                let item = slice[next]
                next += 1
                group.addTask { signature(for: item.id, at: item.url) }
            }
            while let finished = await group.next() {
                if let finished { results.append(finished) }
                if next < slice.count, !Task.isCancelled {
                    let item = slice[next]
                    next += 1
                    group.addTask { signature(for: item.id, at: item.url) }
                }
            }
        }
        return results
    }

    /// nil for a file that cannot be decoded: that photo simply keeps its metadata-only boundaries,
    /// which is what the core does for any photo without a signature.
    nonisolated static func signature(for id: PhotoID, at url: URL) -> (PhotoID, VisualSig)? {
        guard let image = DecodeEngine.decodeThumbnail(url: url, maxPixel: 256),
            let pixels = rgbaPixels(of: image),
            let sig = FirstcutCoreBridge.visualSig(
                rgba: pixels.bytes, width: pixels.width, height: pixels.height)
        else { return nil }
        return (id, sig)
    }

    /// The image as 8-bit sRGB RGBA, the layout the Rust reference expects.
    nonisolated static func rgbaPixels(of image: CGImage) -> (bytes: [UInt8], width: Int, height: Int)? {
        let width = image.width
        let height = image.height
        guard width > 0, height > 0, let space = CGColorSpace(name: CGColorSpace.sRGB) else {
            return nil
        }
        var bytes = [UInt8](repeating: 0, count: width * height * 4)
        let drew = bytes.withUnsafeMutableBytes { buffer -> Bool in
            guard
                let context = CGContext(
                    data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8,
                    bytesPerRow: width * 4, space: space,
                    bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)
            else { return false }
            context.interpolationQuality = .medium
            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        return drew ? (bytes, width, height) : nil
    }
}
