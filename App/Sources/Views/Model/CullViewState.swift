// Owner: ui.
//
// Read-only projection of the culling state that the views bind to, plus the value types they draw.
// app-logic's `AppModel` is the real owner of this state (docs/contracts/app-model.md v0.1); until it
// lands, `PreviewCullViewState` implements this so every screen can be built and run today. The
// shapes below mirror the contract 1:1, so the swap is a deletion of this file plus the adapter.
// See REQ-ui-1 in docs/agents/ui.md.

import CoreGraphics
import Foundation

@MainActor
protocol CullViewState: AnyObject {
  var phase: CullPhase { get }
  var folderName: String { get }
  var batches: [CullBatch] { get }
  var currentBatchIndex: Int { get }
  var currentPhotoIndex: Int { get }
  var photosInCurrentBatch: [CullPhoto] { get }
  var currentPhoto: CullPhoto? { get }
  var ratingMode: RatingMode { get }
  var viewMode: CullViewMode { get }
  var isInfoPanelVisible: Bool { get }
  var isHUDVisible: Bool { get }
  var showsAFOverlay: Bool { get }
  var showsClippingOverlay: Bool { get }
  var autoAdvanceEnabled: Bool { get }
  var viewerBackgroundDarkness: Double { get }
  var progress: CullProgress { get }
  var images: CullImageSource { get }
  func send(_ action: CullAction)
}

/// A photo as the views consume it. REV-53 and REV-69: `tier` and `isKeep` are **values the model
/// supplies**, never derived here. The rating-mode mapping is core-store's single pure function
/// (session-api.md, REV-69) and app-logic's job to call; a view that recomputed it from
/// `stars`/`keep` would disagree with the finish summary the moment a mode mapping changed.
struct CullPhoto: Identifiable, Equatable {
  var id: PhotoID
  var fileName: String
  var meta: PhotoMeta
  var rating: Rating
  var tier: CullTier
  var isKeep: Bool
}

struct CullBatch: Identifiable, Equatable {
  var id: BatchID
  var index: Int
  var photoIDs: [PhotoID]
  var isVisited: Bool
  var isProvisional: Bool
}

/// The tier a photo is in, in the current rating mode. The **model** produces this; see `CullPhoto`.
enum CullTier: String, CaseIterable, Sendable {
  case keep
  case good
  case maybe
  case unrated
  case rejected

  var displayName: String {
    switch self {
    case .keep: "Keep"
    case .good: "Good"
    case .maybe: "Maybe"
    case .unrated: "Unrated"
    case .rejected: "Rejected"
    }
  }
}

enum CullPhase: Equatable {
  case welcome
  case loading(progress: Double)
  case culling
  case finishing
}

enum CullViewMode: Hashable, Sendable, CaseIterable, Identifiable {
  case loupe
  case grid
  case compare(count: Int)

  static var allCases: [CullViewMode] { [.loupe, .grid, .compare(count: 2)] }

  var id: String {
    switch self {
    case .loupe: "loupe"
    case .grid: "grid"
    case .compare: "compare"
    }
  }

  var displayName: String {
    switch self {
    case .loupe: "Loupe"
    case .grid: "Grid"
    case .compare: "Compare"
    }
  }
}

enum CullViewBackground: Equatable, Sendable {
  case loupe
  case grid
  case compare
}

struct CullProgress: Equatable {
  var batchIndex: Int
  var batchCount: Int
  var photosRemaining: Int
  var keeps: Int
  var good: Int
  var maybe: Int
  var unvisitedBatchCount: Int
  var elapsed: TimeInterval

  static let empty = CullProgress(
    batchIndex: 0, batchCount: 0, photosRemaining: 0,
    keeps: 0, good: 0, maybe: 0, unvisitedBatchCount: 0, elapsed: 0
  )
}

@MainActor
protocol CullImageSource: AnyObject {
  var thumbnailProgress: Double { get }
  func thumbnail(for id: PhotoID, size: CGSize) -> CGImage?
  func displayImage(for id: PhotoID) -> CGImage?
  func histogram(for id: PhotoID) -> CullHistogram?
}

struct CullHistogram: Equatable {
  var red: [Double]
  var green: [Double]
  var blue: [Double]
  var luminance: [Double]
}

enum CullAction {
  case openFolder
  case finishCull
  case photoPrevious
  case photoNext
  case batchPrevious
  case batchNext
  case selectPhoto(Int)
  case setViewMode(CullViewMode)
  case toggleInfoPanel
  case toggleHUD
  case toggleAFOverlay
  case toggleClippingOverlay
  case toggleAutoAdvance
  case setRating(stars: UInt8)
  case setFlag(Flag)
  case setLabel(ColorLabel?)
  case toggleKeep
  case undo
  case redo
}
