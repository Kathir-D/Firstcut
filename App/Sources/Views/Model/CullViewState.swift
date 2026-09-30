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
  var isZoomLocked: Bool { get }
  var finishStage: FinishStage { get }
  var recentFolders: [RecentFolder] { get }
  var visibleInfoFields: Set<InfoField> { get }
  var folderURL: URL? { get }
  var autoAdvanceEnabled: Bool { get }
  var viewerBackgroundDarkness: Double { get }
  var progress: CullProgress { get }
  var images: CullImageSource { get }
  /// Something that went wrong that the user has to be told about (a folder that would not open,
  /// sidecars that cannot be written). The window shows it as an alert.
  var errorMessage: String? { get }
  func dismissError()
  func send(_ action: CullAction)
  func finish(_ action: FinishAction)
  func openRecent(_ folder: RecentFolder)
  func forgetRecent(_ folder: RecentFolder)
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
  var tier: Tier
  var isKeep: Bool
}

struct CullBatch: Identifiable, Equatable {
  var id: BatchID
  var index: Int
  var photoIDs: [PhotoID]
  var isVisited: Bool
  var isProvisional: Bool
}

// A photo's tier is `Tier` (App/Sources/Session/SessionTypes.swift) — the same five cases, the
// same titles, one type. `Tier` used to be declared here as well (REV-56: parallel vocabularies
// for one concept), and the duplicate is exactly why the HUD's `keeps/good/maybe` and core-store's
// `[Tier: Int]` counts could not be the same value.

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

// `CullProgress` used to be redeclared here, which is REV-56 — app-logic and ui each invented one
// and the merged tree would not compile ('invalid redeclaration'). There is one type now, in
// App/Sources/Session/SessionTypes.swift, because it is part of the state the model owns: it carries
// the per-tier counts that the HUD and the finish summary both read, and the values a view would
// otherwise recompute from the photos (REV-53).

@MainActor
protocol CullImageSource: AnyObject {
  var thumbnailProgress: Double { get }
  func thumbnail(for id: PhotoID, size: CGSize) -> CGImage?
  func displayImage(for id: PhotoID) -> CGImage?
  func histogram(for id: PhotoID) -> CullHistogram?
}

/// Sendable because the pipeline computes it on a decode thread and hands it to the info panel;
/// every field is a value type, so nothing about it is actor-bound.
struct CullHistogram: Equatable, Sendable {
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
  case toggleZoomLock
  case toggleAutoAdvance
  case setRating(stars: UInt8)
  case setFlag(Flag)
  case setLabel(ColorLabel?)
  case toggleKeep
  case undo
  case redo
}

/// What the Finish sheet can ask for. Every step maps to one `AppModel` method; the sheet never
/// changes the stage itself, so the model's state machine is the only thing that decides what is
/// on screen.
enum FinishAction {
  case showOptions
  case setUnkept(UnkeptAction)
  case setKept(KeptAction)
  case preview
  case back
  case execute
  case undo
  case cancel
}
