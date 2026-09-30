// Owner: pipeline.
//
// Phase two of batching (todo.md §5.4): the shoot is first split from metadata alone, instantly, and
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

/// Where the worker gets its 256 px bitmaps, so the filmstrip and the signatures do not each decode
/// the same photograph.
///
/// `Sendable` because the worker runs it from a detached task and a task group at `.utility`. Both
/// methods are `nonisolated` and non-blocking on purpose: the worker must be able to ask for all
/// 2,880 photos without hopping to the main actor, which would both stall the window and get "not
/// cached yet" for everything.
///
/// `ImageProvider` satisfies it because every one of these calls is a single `NSLock`-guarded
/// dictionary operation on a `final` class that holds no mutable state of its own — which is why the
/// conformance is declared on the type rather than papered over at the call site.
protocol ThumbnailSource: AnyObject, Sendable {
    /// A bitmap that is already decoded, or nil. Never schedules work.
    func cachedThumbnail(for id: PhotoID) -> CGImage?
    /// Take a bitmap decoded elsewhere so it is not decoded again.
    func offerThumbnail(_ id: PhotoID, image: CGImage, fileSize: UInt64)
}

@MainActor
final class VisualSigWorker {
    /// Photos per hand-off. Each hand-off may re-batch, so it is neither per photo (a re-batch per
    /// photo is wasted work) nor the whole shoot (nothing would refine until the very end).
    nonisolated static let chunkSize = 96
    /// Decodes in flight at once. Well under the core count: this is background work.
    nonisolated static let maxConcurrent = 3

    private var task: Task<Void, Never>?

    /// Starts (or restarts) the background pass. `submit` is called on the main actor with each
    /// finished chunk.
    ///
    /// `thumbnails` is the pipeline's cache. Passing it is what makes this pass cheap: a photograph
    /// the filmstrip has already decoded is not decoded again, and one this pass decodes is published
    /// so the filmstrip gets it for free. Without it this decodes the entire shoot from the file
    /// URLs, on top of the pipeline decoding the focus window — a CR3 is ~300 ms, so a 2,880-photo
    /// shoot was being read and decoded twice, competing with the decodes the user is waiting for.
    func start(
        photos: [PhotoMeta], folder: URL, startingAt index: Int,
        thumbnails: (any ThumbnailSource)? = nil,
        submit: @escaping @MainActor ([(PhotoID, VisualSig)]) -> Void
    ) {
        cancel()
        guard !photos.isEmpty else { return }
        let from = min(max(0, index), photos.count - 1)
        let ordered = Array(photos[from...]) + Array(photos[..<from])
        let work = ordered.map {
            (id: $0.id, url: folder.appendingPathComponent($0.relPath), fileSize: $0.fileSize)
        }

        task = Task.detached(priority: .utility) {
            var offset = 0
            while offset < work.count {
                if Task.isCancelled { return }
                let slice = Array(work[offset..<min(offset + Self.chunkSize, work.count)])
                offset += slice.count
                let sigs = await Self.signatures(for: slice, thumbnails: thumbnails)
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
        for slice: [(id: PhotoID, url: URL, fileSize: UInt64)],
        thumbnails: (any ThumbnailSource)?
    ) async -> [(PhotoID, VisualSig)] {
        var results: [(PhotoID, VisualSig)] = []
        await withTaskGroup(of: (PhotoID, VisualSig)?.self) { group in
            var next = 0
            for _ in 0..<min(maxConcurrent, slice.count) {
                let item = slice[next]
                next += 1
                group.addTask { signature(for: item, thumbnails: thumbnails) }
            }
            while let finished = await group.next() {
                if let finished { results.append(finished) }
                if next < slice.count, !Task.isCancelled {
                    let item = slice[next]
                    next += 1
                    group.addTask { signature(for: item, thumbnails: thumbnails) }
                }
            }
        }
        return results
    }

    /// The signature of one photograph, reusing a cached bitmap when there is one.
    ///
    /// The cache is asked first, then the file, and whatever comes back is offered to the cache. So
    /// each photograph is decoded **once** no matter which of the two passes gets there first, and
    /// the filmstrip gets a free thumbnail for every photo this pass touched.
    ///
    /// nil for a file that cannot be decoded: that photo simply keeps its metadata-only boundaries,
    /// which is what the core does for any photo without a signature.
    nonisolated static func signature(
        for id: PhotoID, at url: URL
    ) -> (PhotoID, VisualSig)? {
        signature(for: (id: id, url: url, fileSize: 0), thumbnails: nil)
    }

    nonisolated static func signature(
        for item: (id: PhotoID, url: URL, fileSize: UInt64),
        thumbnails: (any ThumbnailSource)? = nil
    ) -> (PhotoID, VisualSig)? {
        let image: CGImage?
        if let cached = thumbnails?.cachedThumbnail(for: item.id) {
            image = cached
        } else {
            image = DecodeEngine.decodeThumbnail(url: item.url, maxPixel: 256)
            if let image {
                thumbnails?.offerThumbnail(item.id, image: image, fileSize: item.fileSize)
            }
        }
        guard let image,
            let pixels = rgbaPixels(of: image),
            let sig = FirstcutCoreBridge.visualSig(
                rgba: pixels.bytes, width: pixels.width, height: pixels.height)
        else { return nil }
        return (item.id, sig)
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
