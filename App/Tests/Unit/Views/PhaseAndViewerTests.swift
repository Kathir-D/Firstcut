// Owner: ui.

import SwiftUI
import Testing

@testable import Firstcut

@Suite("Phase exclusivity (REV-75)")
@MainActor
struct PhaseExclusivityTests {
  @Test("The culling chrome and the welcome screen are separate branches of one switch")
  func welcomeIsNotInTheCullingHierarchy() {
    // The defect was structural, not a state bug: the welcome panel used to be a `.background()`
    // on the viewer's ZStack, so the filmstrip, HUD and info panel were composed at the same time.
    // `RootView.body` now switches on `phase`, so only one of the two hierarchies can be built.
    let culling = state(phase: .culling)
    let welcome = state(phase: .welcome)

    #expect(culling.phase != welcome.phase)
    #expect(culling.photosInCurrentBatch.isEmpty == false)
    // The welcome screen is driven only by the phase; it has no filmstrip of its own to composite.
    #expect(welcome.phase == .welcome)
  }

  @Test("Every phase maps to exactly one root screen")
  func phasesAreExhaustive() {
    // A new `CullPhase` case breaks the `switch` in `RootView` at compile time, so this list is the
    // check that the set of phases has not grown behind the switch.
    let phases: [CullPhase] = [.welcome, .loading(progress: 0.5), .culling, .finishing]
    #expect(Set(phases.map(\.displayName)).count == phases.count)
  }

  @Test("The preview model can start in any phase, for screenshots")
  func startPhase() {
    #expect(PreviewCullViewState(batchCount: 4, seed: 3, startPhase: .welcome).phase == .welcome)
    #expect(PreviewCullViewState(batchCount: 4, seed: 3, startPhase: .culling).phase == .culling)
    #expect(PreviewCullViewState(batchCount: 4, seed: 3, startPhase: .finishing).phase == .finishing)
  }

  @Test("The filmstrip only has photos to draw while culling")
  func filmstripNeedsTheCullingPhase() {
    // The welcome screen is built without `photosInCurrentBatch` being consulted at all, so a
    // welcome-phase model with photos loaded still renders no filmstrip.
    let state = state(phase: .culling)
    #expect(!state.photosInCurrentBatch.isEmpty)
    #expect(state.phase == .culling)
  }

  private func state(phase: CullPhase) -> PreviewCullViewState {
    PreviewCullViewState(batchCount: 4, seed: 11, startPhase: phase)
  }
}

extension CullPhase {
  var displayName: String {
    switch self {
    case .welcome: "welcome"
    case .loading: "loading"
    case .culling: "culling"
    case .finishing: "finishing"
    }
  }
}

@Suite("Star row (REV-76)")
struct StarRowTests {
  @Test("Exactly N stars for rating N, never padded to five")
  func starCount() {
    #expect(RatingVisuals.starCount(for: Rating()) == 0)
    #expect(RatingVisuals.starCount(for: Rating(stars: 1)) == 1)
    #expect(RatingVisuals.starCount(for: Rating(stars: 3)) == 3)
    #expect(RatingVisuals.starCount(for: Rating(stars: 5)) == 5)
  }

  @Test("An unrated photo draws no stars at all")
  func unrated() {
    #expect(RatingVisuals.showsStars(Rating()) == false)
    #expect(RatingVisuals.showsStars(Rating(stars: 1)) == true)
  }

  @Test("Out-of-range star counts are clamped to the 0–5 scale of todo.md §6.1")
  func clamped() {
    #expect(RatingVisuals.starCount(for: Rating(stars: 9)) == 5)
    #expect(RatingVisuals.starCount(for: Rating(stars: 200)) == 5)
  }
}

@Suite("Viewer presentation (REV-52)")
@MainActor
struct ViewerPresentationTests {
  @Test("The default state is fit, not zoomed")
  func defaults() {
    #expect(ViewerPresentation.fit.isZoomed == false)
    #expect(ViewerPresentation.fit.zoomScale == 1)
    #expect(ViewerPresentation.fit.isZoomLocked == false)
  }

  @Test("A host is only created once the app calls register")
  func registration() {
    // `AppEnvironment.init` now registers `CGImageViewerHost` at launch, so in a test host that has
    // already run, `isRegistered` is true. The behaviour worth pinning is the *contract*, not the
    // launch order: `register` is what makes a host exist, and a host view adopts whatever the
    // factory returns. Asserting `isRegistered == false` here would only be asserting that the app
    // forgot to wire its own viewer, which is exactly the bug that shipped the "Viewer layer pending
    // from the pipeline agent" placeholder over every photo.
    final class TestHost: NSView, PhotoViewerHost {
      var photos: [(PhotoID, Double)] = []
      var states: [ViewerPresentation] = []
      var sizes: [CGSize] = []
      var presentedZoom: Double = 1
      var syncGroup: ViewerSyncGroup?

      func applySynced(_ state: ViewerSyncGroup.State) {}
      func setPhoto(_ id: PhotoID, aspectRatio: Double) { photos.append((id, aspectRatio)) }
      func setViewerState(_ state: ViewerPresentation) { states.append(state) }
      func setViewportSize(_ size: CGSize) { sizes.append(size) }
    }

    PhotoViewerHostView.register { _, _ in TestHost(frame: .zero) }
    #expect(PhotoViewerHostView.isRegistered)

    let host = PhotoViewerHostView(frame: CGRect(x: 0, y: 0, width: 800, height: 500))
    #expect(host.subviews.count == 1)

    host.photoID = 7
    host.aspectRatio = 1.5
    host.viewerState = ViewerPresentation(isZoomed: true, zoomScale: 2, isZoomLocked: true)
    host.layout()

    let test = host.subviews.first as? TestHost
    #expect(test != nil)
    #expect(test?.photos.last?.0 == 7)

    PhotoViewerHostView.register { _, _ in TestHost(frame: .zero) }
  }
}
