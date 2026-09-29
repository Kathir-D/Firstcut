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

struct CullPhoto: Identifiable, Equatable {
  var id: PhotoID
  var fileName: String
  var meta: PhotoMeta
  var rating: Rating
  var tier: CullTier
  var isKeep: Bool

  init(meta: PhotoMeta, rating: Rating = Rating()) {
    id = meta.id
    fileName = (meta.relPath as NSString).lastPathComponent
    self.meta = meta
    self.rating = rating
    tier = CullTier(rating: rating)
    isKeep = rating.keep || rating.stars >= 4
  }
}

struct CullBatch: Identifiable, Equatable {
  var id: BatchID
  var index: Int
  var photoIDs: [PhotoID]
  var isVisited: Bool
  var isProvisional: Bool
}

enum CullTier: String, CaseIterable, Sendable {
  case keep
  case good
  case maybe
  case unrated
  case rejected

  init(rating: Rating) {
    if rating.flag == .reject {
      self = .rejected
    } else if rating.keep || rating.stars >= 4 {
      self = .keep
    } else if rating.stars == 3 {
      self = .good
    } else if rating.stars >= 1 {
      self = .maybe
    } else {
      self = .unrated
    }
  }

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
