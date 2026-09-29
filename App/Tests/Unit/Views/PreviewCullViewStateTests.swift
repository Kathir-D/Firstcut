// Owner: ui.

import CoreGraphics
import Testing

@testable import Firstcut

@Suite("Preview culling state")
@MainActor
struct PreviewCullViewStateTests {
  @Test("The same seed always produces the same shoot")
  func deterministic() {
    let a = PreviewCullViewState(batchCount: 6, seed: 42)
    let b = PreviewCullViewState(batchCount: 6, seed: 42)
    #expect(a.batches.map(\.photoIDs) == b.batches.map(\.photoIDs))
    #expect(a.photosInCurrentBatch.map(\.fileName) == b.photosInCurrentBatch.map(\.fileName))
  }

  @Test("Batch navigation stops at both ends")
  func batchNavigation() {
    let state = PreviewCullViewState(batchCount: 5, seed: 7)
    for _ in 0..<40 { state.send(.batchPrevious) }
    #expect(state.currentBatchIndex == 0)
    for _ in 0..<40 { state.send(.batchNext) }
    #expect(state.currentBatchIndex == 4)
  }

  @Test("Moving to a batch selects its first photo")
  func enteringBatch() {
    let state = PreviewCullViewState(batchCount: 5, seed: 7)
    state.send(.batchNext)
    #expect(state.currentPhotoIndex == 0)
    #expect(state.currentPhoto?.id == state.photosInCurrentBatch.first?.id)
  }

  @Test("Arrowing past the end of a batch rolls into the next one")
  func photoNextWraps() {
    let state = PreviewCullViewState(batchCount: 20, seed: 7)
    #expect(state.currentBatchIndex == 11)
    let count = state.photosInCurrentBatch.count
    for _ in 0..<count { state.send(.photoNext) }
    #expect(state.currentBatchIndex == 12)
    #expect(state.currentPhotoIndex == 0)
  }

  @Test("Selecting a photo by index")
  func selectPhoto() {
    let state = PreviewCullViewState(batchCount: 20, seed: 7)
    state.send(.selectPhoto(1))
    #expect(state.currentPhotoIndex == 1)
    #expect(state.currentPhoto?.id == state.photosInCurrentBatch[1].id)
  }

  @Test("Rating the current photo updates the tier and the progress counts")
  func rating() {
    let state = PreviewCullViewState(batchCount: 3, seed: 7)
    let before = state.progress.keeps
    state.send(.setRating(stars: 5))
    #expect(state.currentPhoto?.tier == .keep)
    #expect(state.currentPhoto?.isKeep == true)
    #expect(state.progress.keeps == before + 1)
    #expect(state.currentPhoto?.rating.stars == 5)
  }

  @Test("A flag is independent of the star count")
  func flags() {
    let state = PreviewCullViewState(batchCount: 3, seed: 7)
    state.send(.setRating(stars: 3))
    state.send(.setFlag(.pick))
    #expect(state.currentPhoto?.rating.flag == .pick)
    #expect(state.currentPhoto?.tier == .good)
    state.send(.setFlag(.reject))
    #expect(state.currentPhoto?.tier == .rejected)
  }

  @Test("Keep mode toggles keep on the current photo")
  func keepMode() {
    let state = PreviewCullViewState(batchCount: 3, seed: 7, ratingMode: .keep)
    state.send(.toggleKeep)
    #expect(state.currentPhoto?.rating.keep == true)
    #expect(state.currentPhoto?.tier == .keep)
    state.send(.toggleKeep)
    #expect(state.currentPhoto?.rating.keep == false)
  }

  @Test("Toggles flip")
  func toggles() {
    let state = PreviewCullViewState(batchCount: 3, seed: 7)
    #expect(state.isInfoPanelVisible == false)
    state.send(.toggleInfoPanel)
    #expect(state.isInfoPanelVisible == true)
    state.send(.toggleHUD)
    #expect(state.isHUDVisible == false)
    state.send(.setViewMode(.grid))
    #expect(state.viewMode == .grid)
    state.send(.toggleAFOverlay)
    #expect(state.showsAFOverlay == true)
    state.send(.toggleClippingOverlay)
    #expect(state.showsClippingOverlay == true)
  }

  @Test("Only the last batch is provisional")
  func provisional() {
    let state = PreviewCullViewState(batchCount: 8, seed: 7)
    #expect(state.batches.filter(\.isProvisional).count == 1)
    #expect(state.batches.last?.isProvisional == true)
  }

  @Test("Thumbnails are generated for every photo in the batch")
  func thumbnails() {
    let state = PreviewCullViewState(batchCount: 3, seed: 7)
    for photo in state.photosInCurrentBatch {
      let image = state.images.thumbnail(for: photo.id, size: CGSize(width: 120, height: 80))
      #expect(image != nil)
      #expect(image?.width == 120)
      #expect(image?.height == 80)
    }
    #expect(state.images.thumbnailProgress == 1.0)
  }

  @Test("Progress counts every photo in the shoot")
  func progressTotals() {
    let state = PreviewCullViewState(batchCount: 4, seed: 7)
    let total = state.batches.reduce(0) { $0 + $1.photoIDs.count }
    #expect(state.progress.batchCount == 4)
    #expect(state.progress.photosRemaining == total)
  }
}
