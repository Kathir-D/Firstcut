// Owner: ui + app-logic.
//
// `AppModel` → `CullViewState`: the projection the views bind to.
//
// `AppModel` is app-logic's brain and is heavily tested with no window and no pixels. `CullViewState`
// is ui's read-only shape, which every view, the toolbar, the menus and the key router read. Until
// now the app ran `PreviewCullViewState` — 148 synthetic batches from a seeded PRNG — because
// nothing bridged the two. This file is that bridge, and deleting `PreviewCullViewState.swift` is
// then a deletion rather than a rewrite.
//
// Two rules this file exists to keep (both are in the protocol's own doc comment):
//
// * **Nothing is derived here.** `tier` and `isKeep` are read off `PhotoVM`, which the model filled
//   in through the one rating-mode mapping. A view that recomputed them from `stars`/`keep` would
//   disagree with the finish summary the moment the mapping changed (REV-69).
// * **Nothing is stored here.** Every accessor is a projection of `AppModel`; this type holds no
//   state of its own beyond the model reference, so it cannot drift out of sync with it.

import CoreGraphics
import Foundation
import Observation

@MainActor
@Observable
final class ModelCullViewState: CullViewState {
  private let model: AppModel
  let images: any CullImageSource

  init(model: AppModel, images: any CullImageSource) {
    self.model = model
    self.images = images
  }

  // MARK: - CullViewState

  var phase: CullPhase {
    // Exhaustive on purpose: a new `Phase` case is a compile error here rather than a phase that
    // silently renders as the previous one.
    switch model.phase {
    case .welcome: .welcome
    case .loading(let progress): .loading(progress: progress.fraction)
    case .culling: .culling
    case .finishing: .finishing
    }
  }

  var folderName: String { model.folderDisplayName }

  /// The model this state wraps (`CullViewState.activeModel`). Overridden rather than defaulted
  /// so a viewer pane created after a `use(_:)` swap reports to the model that is on screen.
  var activeModel: AppModel? { model }

  var batches: [CullBatch] {
    model.batches.map {
      CullBatch(
        id: $0.id, index: $0.index, photoIDs: $0.photoIDs, isVisited: $0.visited,
        isProvisional: $0.provisional)
    }
  }

  var currentBatchIndex: Int { model.currentBatchIndex }

  var currentPhotoIndex: Int { model.currentPhotoIndex }

  var photosInCurrentBatch: [CullPhoto] {
    model.photos(inBatch: model.currentBatchIndex).map(\.cullPhoto)
  }

  var currentPhoto: CullPhoto? { model.currentPhoto?.cullPhoto }

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
  var isDebugHUDVisible: Bool { model.debugHUDVisible }

  /// The pipeline's counters, for the debug HUD. Nil for a mock provider, which has no engine to
  /// report on — the view then draws nothing rather than zeros, which would read as "all quiet".
  var pipelineStats: PipelineStats? { (images as? ImageProvider)?.stats }
  var thumbnailProgress: Double { (images as? ImageProvider)?.thumbnailProgress ?? 1 }
  var byteRangeDecodes: Int { (images as? ImageProvider)?.byteRangeDecodes ?? 0 }
  var containerDecodes: Int { (images as? ImageProvider)?.containerDecodes ?? 0 }
  var memoryBudgetBytes: Int { model.settings.memoryBudgetBytes }
  /// The viewer's backing size, which is what T2 is decoded at — worth seeing in the HUD, because a
  /// stale zero here is why the viewer looks soft.
  var viewportPixelSize: CGSize { model.viewportPixelSize }
  /// The measured key-to-frame, straight off the model: the intervals are opened by the command
  /// handler and closed by the viewer's frame commit, so no view owns the number.
  var lastFrameLatencyMs: Double? { model.lastFrameLatencyMs }
  var worstFrameLatencyMs: Double? { model.worstFrameLatencyMs }
  var standInFramesPresented: Int { model.standInFramesPresented }
  var showsAFOverlay: Bool { model.afOverlay }
  var showsClippingOverlay: Bool { model.clippingOverlay }
  var isZoomLocked: Bool { model.viewer.zoomLock }
  var finishStage: FinishStage { model.finish }
  var recentFolders: [RecentFolder] { model.recents }
  var visibleInfoFields: Set<InfoField> { model.settings.viewer.infoFields }
  var folderURL: URL? { model.folderURL }
  var autoAdvanceEnabled: Bool { model.autoAdvance }
  var viewerBackgroundDarkness: Double { model.viewer.backgroundGray }
  var progress: CullProgress { model.progress }

  var errorMessage: String? { model.lastError }
  func dismissError() { model.dismissError() }

  func openRecent(_ folder: RecentFolder) { model.openRecent(folder) }
  func forgetRecent(_ folder: RecentFolder) { model.forgetRecent(folder) }

  func finish(_ action: FinishAction) {
    switch action {
    case .showOptions: model.showFinishOptions()
    case .setUnkept(let unkept): model.updateFinishSettings { $0.unkept = unkept }
    case .setKept(let kept): model.updateFinishSettings { $0.kept = kept }
    case .preview: model.runFinishDryRun()
    case .back: model.backFinish()
    case .execute: model.executeFinish()
    case .undo: model.undoFinish()
    case .cancel: model.cancelFinish()
    }
  }

  func send(_ action: CullAction) {
    // The filmstrip selects by position *inside the current batch*, which is a navigation, not a
    // `Command` — the command list has no "jump to index" because every other entry is a
    // photographer's intent rather than a UI gesture.
    if case .selectPhoto(let index) = action {
      model.goToPhoto(index)
      return
    }
    guard let command = action.command else { return }
    model.perform(command)
  }
}

// MARK: - Action mapping

extension CullAction {
  /// Every ui action is a `Command`; this is the whole translation table, in one place, so a
  /// shortcut and a menu item cannot mean different things. `openFolder` and `toggleFullScreen`
  /// are the model's to raise as callbacks — they need a panel and a window, not a state change.
  ///
  /// `send` handles `.selectPhoto` before it gets here — it is the one action with no `Command`,
  /// and returns nil for it.
  var command: Command? {
    switch self {
    case .photoPrevious: .photoPrevious
    case .photoNext: .photoNext
    case .batchPrevious: .batchPrevious
    case .batchNext: .batchNext
    case .setViewMode(let mode):
      switch mode {
      case .loupe: .showLoupe
      case .grid: .showGrid
      case .compare(let count): .showCompare(count)
      }
    case .toggleInfoPanel: .toggleInfoPanel
    case .toggleHUD: .toggleHUD
    case .toggleAFOverlay: .toggleAFOverlay
    case .toggleClippingOverlay: .toggleClippingOverlay
    case .toggleZoomLock: .toggleZoomLock
    case .toggleAutoAdvance: .toggleAutoAdvance
    case .setRating(let stars): .setStars(Int(stars))
    case .setFlag(.pick): .togglePickFlag
    case .setFlag(.none): .unflag
    case .setFlag(.reject): .rejectFlag
    case .setLabel(let label): .setLabel(label)
    case .toggleKeep: .toggleKeep
    case .undo: .undo
    case .redo: .redo
    case .openFolder: .openFolder
    case .finishCull: .finishCull
    case .selectPhoto: nil
    }
  }
}

extension PhotoVM {
  /// The same photo as `CullPhoto`. Field-for-field, and in particular `tier` and `isKeep` come
  /// straight across rather than being recomputed.
  var cullPhoto: CullPhoto {
    CullPhoto(
      id: id, fileName: fileName, meta: meta, rating: rating, tier: tier, isKeep: isKeep)
  }
}
