// Owner: app-logic.
//
// The real session, projected into the shape the views bind to.
//
// `AppEnvironment` used to build `PreviewCullViewState`, a stand-in that invents 148 batches from
// a seed. Every view was therefore rendering photographs that did not exist, and the real
// `AppModel` -- with a real folder, real batches, real ratings and a real Finish -- was never
// instantiated. This is the one line `AppEnvironment.init` was documented as needing, and it is
// this file.
//
// ## Translation, not logic
//
// The mapping is mechanical and the decisions stay in `AppModel` and in Rust:
//
// * `tier` and `isKeep` are **read off the model**, never recomputed. REV-53/REV-69 exist because
//   a view that derived "kept" from `stars` and `keep` separately would disagree with the Finish
//   summary the moment a rating-mode mapping changed -- which is how this project shipped a Keep
//   ring on a photo Finish trashes.
// * `phase` is derived from whether a session is open and what Finish is doing, because those are
//   facts rather than states somebody has to set.
//
// One thing is decided here, and it is a UI decision: the window is told about the *real* image
// source, so `ViewerArea` draws photographs rather than the placeholder. That is the entire reason
// this file exists.

import CoreGraphics
import Foundation

/// Projects `AppModel` (app-logic's owner of the truth) into `CullViewState` (ui's read-only view
/// of it).
@MainActor
final class LiveCullViewState: CullViewState {
  let model: AppModel
  /// What the views bind to. The existential, because that is the protocol's shape and ui must not
  /// learn this type's name.
  let images: any CullImageSource
  /// The same object, concretely, for the things only the pipeline can do: preloading a window of
  /// frames and telling the decoder what size the window is.
  private let previewImages: EmbeddedPreviewSource
  /// Called after any action that can change what should be on screen, so the window can redraw and
  /// the viewer can re-request the current frame.
  var onChange: (() -> Void)?

  private(set) var phase: CullPhase = .welcome

  init(model: AppModel, images: EmbeddedPreviewSource) {
    self.model = model
    self.images = images
    self.previewImages = images
  }


// MARK: - CullViewState

  var folderName: String { model.folderDisplayName }

  var batches: [CullBatch] {
    model.batches.map {
      CullBatch(
        id: $0.id, index: $0.index, photoIDs: $0.photoIDs, isVisited: $0.visited,
        isProvisional: $0.provisional)
    }
  }

  var currentBatchIndex: Int { model.currentBatch?.index ?? 0 }

  var currentPhotoIndex: Int { model.currentPhotoIndex }

  var photosInCurrentBatch: [CullPhoto] {
    model.photos(inBatch: currentBatchIndex).map(Self.cullPhoto)
  }

  var currentPhoto: CullPhoto? { model.currentPhoto.map(Self.cullPhoto) }

  var ratingMode: RatingMode { model.ratingMode }

  var viewMode: CullViewMode {
    switch model.viewMode {
    case .loupe: .loupe
    case .grid: .grid
    case .compare(let count): .compare(count: count)
    }
  }

  var isInfoPanelVisible: Bool { model.infoPanelVisible }
  var isHUDVisible: Bool { model.hudVisible }
  var showsAFOverlay: Bool { model.afOverlay }
  var showsClippingOverlay: Bool { model.clippingOverlay }
  var autoAdvanceEnabled: Bool { model.autoAdvance }
  var viewerBackgroundDarkness: Double { model.settings.viewer.backgroundGray }
  var progress: CullProgress { model.progress }

  /// Every `CullAction` routes through `AppModel.perform(_:)`, which is app-logic's own routing
  /// table. Going via `perform` rather than calling methods one by one is deliberate: the keyboard
  /// path and the view path then run the *same* code, so a shortcut and a button cannot diverge.
  func send(_ action: CullAction) {
    switch action {
    case .openFolder:
      model.onRequestOpenFolder?()
    case .photoNext:
      model.movePhoto(by: 1)
    case .photoPrevious:
      model.movePhoto(by: -1)
    case .batchNext:
      _ = model.moveBatch(by: 1)
    case .batchPrevious:
      _ = model.moveBatch(by: -1)
    case .selectPhoto(let index):
      model.goToPhoto(index)
    case .setViewMode(let mode):
      model.setViewMode(mode.viewMode)
    case .setRating(let stars):
      model.rateCurrent { $0.stars = stars }
    case .setFlag(let flag):
      model.rateCurrent { $0.flag = flag }
    case .setLabel(let label):
      model.rateCurrent { $0.label = label }
    case .toggleKeep:
      model.rateCurrent { $0.keep.toggle() }
    case .undo:
      model.undo()
    case .redo:
      model.redo()
    case .finishCull:
      model.startFinish()
    case .toggleInfoPanel:
      model.perform(.toggleInfoPanel)
    case .toggleHUD:
      model.perform(.toggleHUD)
    case .toggleAFOverlay:
      model.perform(.toggleAFOverlay)
    case .toggleClippingOverlay:
      model.perform(.toggleClippingOverlay)
    case .toggleAutoAdvance:
      model.perform(.toggleAutoAdvance)
    }
    refresh()
  }

  /// Re-derives what the window shows, and tells the viewer the current frame changed so it can ask
  /// for real pixels.
  func refresh() {
    phase = Self.phase(for: model)
    if phase != .welcome, let id = model.currentPhoto?.id {
      // Keep the frames the arrow keys will reach decoded. Cheap when they already are, and the
      // one thing that stops ← and → from waiting on a JPEG.
      previewImages.prefetchAround(id, maxPixel: previewImages.maxPixel)
    }
    onChange?()
  }

  /// The real images for whatever is on screen, and the folder they live in.
  ///
  /// Called when a session opens, because a new folder means a new set of files and a cache full of
  /// the previous shoot's photographs would be worse than no cache.
  func attach(folder: URL, photos: [PhotoMeta], order: [PhotoID]) {
    previewImages.configure(folder: folder, photos: photos, inOrder: order)
    refresh()
  }

  /// The window's pixel size, so the viewer can decode at the size it will be drawn at rather than
  /// a fixed one that would be wrong on every display (task.md §7.1, T2).
  func setViewport(_ size: CGSize) {
    previewImages.maxPixel = Int(max(size.width, size.height)).clamped(to: 256...8192)
    model.setViewportPixelSize(size)
  }

  /// `AppModel`'s VM types are already shaped like ui's; this is the one place that says so, so
  /// the two cannot drift apart by being converted in three different ways.
  private static func cullPhoto(_ vm: PhotoVM) -> CullPhoto {
    CullPhoto(
      id: vm.id, fileName: vm.fileName, meta: vm.meta, rating: vm.rating, tier: vm.tier,
      isKeep: vm.isKeep)
  }

  /// Which screen the window is on. Derived, because "is there a session" and "is Finish running"
  /// are facts about the model, not states a second object has to keep in step.
  private static func phase(for model: AppModel) -> CullPhase {
    guard model.currentBatch != nil || !model.batches.isEmpty else { return .welcome }
    switch model.finish {
    case .hidden:
      return .culling
    case .executing:
      return .loading(progress: model.progress.batchNumber > 0
        ? Double(model.progress.photoNumber) / Double(max(1, model.progress.photoCount))
        : 0)
    case .summary, .options, .dryRun, .report, .failed:
      return .finishing
    }
  }
}

// MARK: - Mode conversion

/// `CullViewMode` is ui's; `ViewMode` is app-logic's, with the same three cases. The conversion is
/// here so neither side has to know about the other's spelling.
extension CullViewMode {
  var viewMode: ViewMode {
    switch self {
    case .loupe: .loupe
    case .grid: .grid
    case .compare(let count): .compare(count)
    }
  }
}
