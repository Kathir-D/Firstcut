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
    /// Settings → Performance → Debug HUD, and the pipeline counters it reads. All defaulted, because
    /// the preview state has no engine behind it and reporting zeros there would read as "all quiet"
    /// rather than "not measuring".
    var isDebugHUDVisible: Bool { get }
    var pipelineStats: PipelineStats? { get }
    var thumbnailProgress: Double { get }
    var byteRangeDecodes: Int { get }
    var containerDecodes: Int { get }
    var memoryBudgetBytes: Int { get }
    var viewportPixelSize: CGSize { get }
    /// The measured key-to-frame, in milliseconds: the last one and the worst since the folder opened.
    /// These are the only numbers in the app that come from a stopwatch rather than a counter, and the
    /// row they decide is todo.md §7.3's "arrow key → sharp photo ≤ 8 ms".
    var lastFrameLatencyMs: Double? { get }
    var worstFrameLatencyMs: Double? { get }
    /// Frames that reached the user as the 256 px stand-in. Must stay 0; anything else is §7.1 broken.
    var standInFramesPresented: Int { get }
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

extension CullViewState {
    /// The model behind the state, when there is one. The viewer reports presented frames and its
    /// viewport size to it, so a pane must resolve this from the *active* state rather than from
    /// whatever the environment was built with. nil for the stand-ins, which have no pipeline to
    /// report to.
    var activeModel: AppModel? { nil }
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
    /// The display bitmap, at least `minimumLongestEdge` pixels on its longest side. Zero asks for
    /// whatever T2 is, which is the right answer for a view that fits the photograph to the window and
    /// has not been laid out yet; a view showing it at 100% asks for the photograph's own pixels.
    func displayImage(for id: PhotoID, minimumLongestEdge: Int) -> CGImage?
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
    /// The answer to Settings → General → "Confirm before Finish": yes, go ahead.
    case confirm
    case showOptions
    case setUnkept(UnkeptAction)
    case setKept(KeptAction)
    case preview
    case back
    case execute
    case undo
    case cancel
}

// MARK: - Defaults for a state with no pipeline behind it

/// The preview/mock shoot has no decoder, so the debug HUD would show a wall of zeroes that reads as
/// "nothing is wrong" rather than "nothing is being measured". `pipelineStats` being nil is what the
/// view checks, and these keep every conformer from having to spell the rest out.
extension CullViewState {
    public var isDebugHUDVisible: Bool { false }
    public var pipelineStats: PipelineStats? { nil }
    public var thumbnailProgress: Double { 1 }
    public var byteRangeDecodes: Int { 0 }
    public var containerDecodes: Int { 0 }
    public var memoryBudgetBytes: Int { 0 }
    public var viewportPixelSize: CGSize { .zero }
    public var lastFrameLatencyMs: Double? { nil }
    public var worstFrameLatencyMs: Double? { nil }
    public var standInFramesPresented: Int { 0 }

    /// The clipping overlay's clip points, resolved from Settings by the real state. The preview
    /// shoot has no settings, so it gets the mask's own defaults — the numbers it has always used.
    public var clippingThresholds: (highlight: UInt8, shadow: UInt8) {
        (ClippingMask.defaultHighlight, ClippingMask.defaultShadow)
    }
}
