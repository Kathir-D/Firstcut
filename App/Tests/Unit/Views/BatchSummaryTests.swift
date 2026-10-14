import Foundation
import Testing

@testable import Firstcut

@MainActor
@Suite("Batch summary")
struct BatchSummaryTests {
    @Test("An empty batch says zero photos and is not visited")
    func emptyBatch() {
        let summary = BatchSummary(number: 4, photos: [], isVisited: false)
        #expect(summary.detail == "0 photos · 0 rated")
        #expect(summary.status == "Not visited yet")
        #expect(summary.startTime() == nil)
    }

    @Test("Counts photos and rated photos, and reports a visited batch")
    func countsRated() {
        let photos = PreviewCullViewState().photosInCurrentBatch
        let summary = BatchSummary(number: 1, photos: photos, isVisited: true)
        #expect(summary.photoCount == photos.count)
        #expect(summary.ratedCount == photos.filter { !$0.rating.isNeutral }.count)
        #expect(summary.status == "Visited")
    }

    @Test("One photo reads as a singular noun")
    func singular() {
        let one = Array(PreviewCullViewState().photosInCurrentBatch.prefix(1))
        #expect(BatchSummary(number: 1, photos: one, isVisited: false).detail.contains("1 photo ·"))
    }
}
