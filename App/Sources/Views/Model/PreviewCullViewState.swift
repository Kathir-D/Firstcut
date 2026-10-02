// Owner: ui.
//
// Stand-in for app-logic's `AppModel` while the real model is being written. Deterministic: the same
// seed always produces the same shoot, so screenshots and UI tests are stable. Delete this file
// once `AppModel.preview(game:)` exists (REQ-ui-1).

import AppKit
import CoreGraphics
import Foundation

@Observable
@MainActor
final class PreviewCullViewState: CullViewState {
    private(set) var phase: CullPhase = .culling
    private(set) var folderName: String
    private(set) var batches: [CullBatch] = []
    private(set) var currentBatchIndex: Int = 0
    private(set) var currentPhotoIndex: Int = 0
    private(set) var photosInCurrentBatch: [CullPhoto] = []
    private(set) var currentPhoto: CullPhoto?
    var ratingMode: RatingMode = .stars {
        // Switching modes re-derives every photo's tier through the same mapping the finish summary
        // uses (REV-69), so the filmstrip, info panel and HUD cannot disagree with it.
        didSet {
            guard oldValue != ratingMode else { return }
            rederiveTiers()
        }
    }
    var viewMode: CullViewMode = .loupe
    var isInfoPanelVisible = false
    var isHUDVisible = true
    var showsAFOverlay = false
    var showsClippingOverlay = false
    var isZoomLocked = false
    var finishStage: FinishStage = .hidden
    var recentFolders: [RecentFolder] = []
    var visibleInfoFields: Set<InfoField> = InfoField.all
    var folderURL: URL? { nil }
    var autoAdvanceEnabled = false
    var viewerBackgroundDarkness: Double = 0.13
    private(set) var progress: CullProgress = CullProgress()
    let images: CullImageSource

    private var allPhotos: [PhotoID: CullPhoto] = [:]
    private var ratings: [PhotoID: Rating] = [:]
    private var visited: Set<BatchID> = []
    private let startedAt = Date()

    init(
        batchCount: Int = 148,
        seed: UInt64 = 0x5eed_f1c5,
        ratingMode: RatingMode = .stars,
        startPhase: CullPhase = .culling
    ) {
        self.folderName = "Game1JENKS"
        self.ratingMode = ratingMode
        phase = startPhase
        images = PreviewImageSource(seed: seed &* 31)
        build(batchCount: batchCount, seed: seed)
    }

    // MARK: - CullViewState

    var errorMessage: String? { nil }
    func dismissError() {}

    func openRecent(_ folder: RecentFolder) {}
    func forgetRecent(_ folder: RecentFolder) {}

    func finish(_ action: FinishAction) {
        if case .cancel = action { finishStage = .hidden }
    }

    func send(_ action: CullAction) {
        switch action {
        case .photoPrevious:
            if currentPhotoIndex > 0 {
                select(currentPhotoIndex - 1)
            } else if currentBatchIndex > 0 {
                go(toBatch: currentBatchIndex - 1, atEnd: true)
            }
        case .photoNext:
            if currentPhotoIndex + 1 < photosInCurrentBatch.count {
                select(currentPhotoIndex + 1)
            } else if currentBatchIndex + 1 < batches.count {
                go(toBatch: currentBatchIndex + 1, atEnd: false)
            }
        case .batchPrevious:
            if currentBatchIndex > 0 { go(toBatch: currentBatchIndex - 1, atEnd: true) }
        case .batchNext:
            if currentBatchIndex + 1 < batches.count { go(toBatch: currentBatchIndex + 1, atEnd: false) }
        case .selectPhoto(let index):
            select(index)
        case .setViewMode(let mode):
            viewMode = mode
        case .toggleInfoPanel:
            isInfoPanelVisible.toggle()
        case .toggleHUD:
            isHUDVisible.toggle()
        case .toggleAFOverlay:
            showsAFOverlay.toggle()
        case .toggleClippingOverlay:
            showsClippingOverlay.toggle()
        case .toggleZoomLock:
            isZoomLocked.toggle()
        case .toggleAutoAdvance:
            autoAdvanceEnabled.toggle()
        case .setRating(let stars):
            rateCurrent { $0.stars = stars }
        case .setFlag(let flag):
            rateCurrent { $0.flag = flag }
        case .setLabel(let label):
            rateCurrent { $0.label = label }
        case .setRatingMode(let mode):
            ratingMode = mode
        case .toggleKeep:
            rateCurrent { $0.keep.toggle() }
        case .setKeep:
            rateCurrent { $0.keep = true }
        case .setNotKeep:
            rateCurrent { $0.keep = false }
        case .undo, .redo:
            break
        case .openFolder, .finishCull:
            phase = .finishing
        }
    }

    // MARK: - Shoot generation

    private func build(batchCount: Int, seed: UInt64) {
        var rng = SeededGenerator(seed: seed)
        var nextID: PhotoID = 1
        var frameNumber = 3_181
        var batchID: BatchID = 1
        var start = Date(timeIntervalSince1970: 1_787_324_089)
        var built: [CullBatch] = []

        for batchIndex in 0..<batchCount {
            let isSingle = rng.next(upperBound: 100) < 8
            let count = isSingle ? 1 : 3 + rng.next(upperBound: 12)
            var ids: [PhotoID] = []
            for _ in 0..<count {
                let exposure = rng.pick([0.0005, 0.0004, 0.0008, 0.00025, 0.002])
                let meta = PhotoMeta(
                    id: nextID,
                    relPath: "IMG_\(frameNumber).CR3",
                    companions: [],
                    kind: .raw(.cr3),
                    fileSize: UInt64(11_000_000 + rng.next(upperBound: 3_000_000)),
                    captureTime: CaptureTime(
                        unixMs: Int64(start.timeIntervalSince1970 * 1000),
                        subsecResolutionMs: 10,
                        offsetMinutes: -360,
                        source: .exif
                    ),
                    shutterCount: UInt64(33_500 + frameNumber),
                    fileNumber: UInt32(frameNumber),
                    cameraMake: "Canon",
                    cameraModel: "Canon EOS R8",
                    cameraSerial: "122022006902",
                    lensModel: "EF70-200mm f/2.8L IS II USM",
                    focalLengthMm: 200,
                    exposureTimeS: Float(exposure),
                    fNumber: 2.8,
                    iso: UInt32(rng.pick([800, 1000, 1250, 1600, 2000, 400, 640])),
                    exposureCompEv: 0,
                    meteringMode: "Evaluative",
                    driveMode: "Continuous Shooting",
                    shutterMode: rng.pick(["Electronic", "Electronic First Curtain"]),
                    orientation: 8,
                    width: 6000,
                    height: 4000,
                    af: AfInfo(
                        areaMode: "AF Point Expansion (8 point)",
                        points: [
                            AfPoint(x: 0.52, y: 0.44, w: 0.05, h: 0.08, inFocus: true),
                            AfPoint(x: 0.47, y: 0.5, w: 0.05, h: 0.08, inFocus: true),
                            AfPoint(x: 0.58, y: 0.38, w: 0.05, h: 0.08, inFocus: true),
                        ]
                    ),
                    preview: EmbeddedPreview(range: ByteRange(offset: 0, len: 0), width: 1620, height: 1080),
                    fullPreview: EmbeddedPreview(
                        range: ByteRange(offset: 0, len: 0), width: 6000, height: 4000),
                    warnings: []
                )
                let photo = makePhoto(meta, rating: Rating())
                allPhotos[meta.id] = photo
                ratings[meta.id] = Rating()
                ids.append(meta.id)
                nextID += 1
                frameNumber += 1 + (rng.next(upperBound: 6) == 0 ? 1 : 0)
                start = start.addingTimeInterval(0.09 + Double(rng.next(upperBound: 40)) / 1000)
            }
            if rng.next(upperBound: 100) < 45 { visited.insert(batchID) }
            built.append(
                CullBatch(
                    id: batchID,
                    index: batchIndex,
                    photoIDs: ids,
                    isVisited: visited.contains(batchID),
                    isProvisional: batchIndex == batchCount - 1
                )
            )
            batchID += 1
        }

        batches = built
        go(toBatch: min(11, built.count - 1), atEnd: false)
    }

    private func rederiveTiers() {
        for (id, photo) in allPhotos {
            var updated = photo
            let rating = ratings[id] ?? Rating()
            updated.rating = rating
            updated.tier = RatingTiers.tier(for: rating, mode: ratingMode)
            updated.isKeep = RatingTiers.isKeep(rating)
            allPhotos[id] = updated
        }
        photosInCurrentBatch =
            batches.indices.contains(currentBatchIndex)
            ? batches[currentBatchIndex].photoIDs.compactMap { allPhotos[$0] }
            : []
        currentPhoto =
            photosInCurrentBatch.indices.contains(currentPhotoIndex)
            ? photosInCurrentBatch[currentPhotoIndex]
            : nil
        recomputeProgress()
    }

    private func makePhoto(_ meta: PhotoMeta, rating: Rating) -> CullPhoto {
        CullPhoto(
            id: meta.id,
            fileName: (meta.relPath as NSString).lastPathComponent,
            meta: meta,
            rating: rating,
            tier: RatingTiers.tier(for: rating, mode: ratingMode),
            isKeep: RatingTiers.isKeep(rating)
        )
    }

    private func select(_ index: Int) {
        guard photosInCurrentBatch.indices.contains(index) else { return }
        currentPhotoIndex = index
        currentPhoto = photosInCurrentBatch[index]
    }

    private func rateCurrent(_ mutate: (inout Rating) -> Void) {
        guard let photo = currentPhoto else { return }
        var rating = ratings[photo.id] ?? Rating()
        mutate(&rating)
        ratings[photo.id] = rating
        var updated = photo
        updated.rating = rating
        updated.tier = RatingTiers.tier(for: rating, mode: ratingMode)
        updated.isKeep = RatingTiers.isKeep(rating)
        allPhotos[photo.id] = updated
        photosInCurrentBatch[currentPhotoIndex] = updated
        currentPhoto = updated
        recomputeProgress()
    }

    private func go(toBatch index: Int, atEnd: Bool) {
        guard batches.indices.contains(index) else { return }
        currentBatchIndex = index
        let ids = batches[index].photoIDs
        photosInCurrentBatch = ids.compactMap { allPhotos[$0] }
        currentPhotoIndex = atEnd ? max(0, photosInCurrentBatch.count - 1) : 0
        currentPhoto =
            photosInCurrentBatch.indices.contains(currentPhotoIndex)
            ? photosInCurrentBatch[currentPhotoIndex] : nil
        visited.insert(batches[index].id)
        batches[index].isVisited = true
        recomputeProgress()
    }

    private func recomputeProgress() {
        // Counted over *every* photo, not over `ratings`: the shoot starts with no ratings at all, so
        // iterating the rated ones alone counted zero unrated photos and the HUD read "0 unrated left"
        // on a freshly opened folder.
        var counts: [Tier: Int] = [:]
        for photo in allPhotos.values {
            counts[photo.tier, default: 0] += 1
        }
        let total = allPhotos.count
        let rated = total - counts[.unrated, default: 0]
        progress = CullProgress(
            batchNumber: batches.isEmpty ? 0 : currentBatchIndex + 1,
            batchCount: batches.count,
            photoNumber: currentPhotoIndex + 1,
            photoCount: photosInCurrentBatch.count,
            photosLeftInBatch: max(0, photosInCurrentBatch.count - currentPhotoIndex - 1),
            batchesLeft: max(0, batches.count - currentBatchIndex - 1),
            unvisitedBatches: batches.count(where: { !$0.isVisited }),
            counts: counts,
            totalPhotos: total,
            ratedPhotos: rated
        )
    }
}

// MARK: - Preview images

@Observable
@MainActor
final class PreviewImageSource: CullImageSource {
    private var cache: [PhotoID: CGImage] = [:]
    let thumbnailProgress = 1.0
    private let seed: UInt64

    init(seed: UInt64) {
        self.seed = seed
    }

    func thumbnail(for id: PhotoID, size: CGSize) -> CGImage? {
        if let cached = cache[id] { return cached }
        guard let image = Self.makeThumbnail(seed: seed &+ id &* 2_654_435_761, size: size) else {
            return nil
        }
        cache[id] = image
        return image
    }

    /// The size is ignored: the synthetic shoot draws one picture per id and scaling it is free, so
    /// there is nothing here for a viewer's backing size to change.
    func displayImage(for id: PhotoID, minimumLongestEdge: Int) -> CGImage? {
        thumbnail(for: id, size: CGSize(width: 1200, height: 800))
    }

    func histogram(for id: PhotoID) -> CullHistogram? {
        var generator = SeededGenerator(seed: seed &+ id &* 40_503)
        let binCount = 64
        return CullHistogram(
            red: (0..<binCount).map { _ in generator.nextDouble() },
            green: (0..<binCount).map { _ in generator.nextDouble() },
            blue: (0..<binCount).map { _ in generator.nextDouble() },
            luminance: (0..<binCount).map { _ in generator.nextDouble() }
        )
    }

    private static func makeThumbnail(seed: UInt64, size: CGSize) -> CGImage? {
        let width = max(2, Int(size.width.rounded()))
        let height = max(2, Int(size.height.rounded()))
        guard
            let context = CGContext(
                data: nil,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue
            )
        else { return nil }

        var generator = SeededGenerator(seed: seed)
        let bounds = CGRect(x: 0, y: 0, width: width, height: height)
        let base = NSColor(
            hue: generator.nextDouble(),
            saturation: 0.3 + generator.nextDouble() * 0.2,
            brightness: 0.18 + generator.nextDouble() * 0.1,
            alpha: 1
        )
        context.setFillColor(base.cgColor)
        context.fill(bounds)

        context.setFillColor(NSColor.white.withAlphaComponent(0.05).cgColor)
        for _ in 0..<5 {
            let y = generator.nextDouble() * Double(height)
            let bandHeight = (6 + generator.nextDouble() * 26) * Double(height) / 200
            context.fill(CGRect(x: 0, y: y, width: Double(width), height: bandHeight))
        }

        context.setFillColor(NSColor.white.withAlphaComponent(0.03).cgColor)
        context.fillEllipse(in: bounds.insetBy(dx: -Double(width) * 0.2, dy: -Double(height) * 0.1))
        return context.makeImage()
    }
}

// MARK: - Deterministic generator

struct SeededGenerator: RandomNumberGenerator {
    private var state: UInt64

    init(seed: UInt64) {
        state = seed == 0 ? 0x9E37_79B9_7F4A_7C15 : seed
    }

    mutating func next() -> UInt64 {
        state ^= state << 13
        state ^= state >> 7
        state ^= state << 17
        return state
    }

    mutating func next(upperBound: Int) -> Int {
        guard upperBound > 0 else { return 0 }
        return Int(next() % UInt64(upperBound))
    }

    mutating func nextDouble() -> Double {
        Double(next() % 1_000_000) / 1_000_000
    }

    mutating func pick<T>(_ options: [T]) -> T {
        options[next(upperBound: options.count)]
    }
}
