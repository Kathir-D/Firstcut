// Owner: ui.
//
// Finder gallery layout (todo.md §9): large viewer, filmstrip of the current batch underneath,
// floating HUD, right-hand info inspector. The window chrome lives in App/Toolbar.
//
// REV-75: the phases are mutually exclusive *by construction*. The culling chrome used to be a
// ZStack with the welcome panel as a `.background()`, so the filmstrip, HUD and info panel stayed
// in the hierarchy while the welcome panel was also on screen and four cells were unreachable. The
// switch below is exhaustive on `phase`, so adding a case to `CullPhase` is a compile error until
// this view decides where that phase renders.

import SwiftUI

struct RootView: View {
  let state: any CullViewState

  @Environment(\.accessibilityReduceMotion) private var reduceMotion

  var body: some View {
    Group {
      switch state.phase {
      case .welcome:
        WelcomeScreen(state: state)
      case .loading(let progress):
        LoadingScreen(progress: progress)
      case .culling, .finishing:
        cullingLayout
      }
    }
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .background(Appearance.windowBackground)
    // Finish Cull (todo.md §9.7). The stage is the model's; dismissing the sheet any other way
    // than a button (Escape) is a cancel, so what is on screen never disagrees with the model.
    .sheet(
      isPresented: Binding(
        get: { state.finishStage.isVisible },
        set: { if !$0 { state.finish(.cancel) } })
    ) {
      FinishSheet(state: state)
    }
    .alert(
      "Something went wrong",
      isPresented: Binding(
        get: { state.errorMessage != nil },
        set: { if !$0 { state.dismissError() } })
    ) {
      Button("OK") { state.dismissError() }
    } message: {
      Text(state.errorMessage ?? "")
    }
  }

  /// Only reachable in `.culling` and `.finishing`, so nothing here can composite over the welcome
  /// panel: the two hierarchies are separate branches of the switch above.
  private var cullingLayout: some View {
    HStack(spacing: 0) {
      ZStack(alignment: .bottom) {
        mainArea

        if state.isHUDVisible {
          VStack {
            Spacer(minLength: 0)
            ProgressHUD(progress: state.progress, photo: state.currentPhoto, mode: state.ratingMode)
              .padding(.bottom, Appearance.filmstripHeight + 16)
          }
          .allowsHitTesting(false)
        }

        // The grid *is* the batch, so a filmstrip under it would show everything twice.
        if state.viewMode != .grid { filmstrip }
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity)

      if state.isInfoPanelVisible {
        InfoPanel(state: state)
          .transition(.move(edge: .trailing).combined(with: .opacity))
      }
    }
    .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: state.isInfoPanelVisible)
  }

  /// The photograph area, by view mode (G / E / C).
  @ViewBuilder
  private var mainArea: some View {
    switch state.viewMode {
    case .loupe:
      ViewerArea(state: state)
    case .grid:
      GridView(state: state)
    case .compare(let count):
      CompareView(state: state, count: count)
    }
  }

  @ViewBuilder
  private var filmstrip: some View {
    if !state.photosInCurrentBatch.isEmpty {
      Filmstrip(
        frames: state.photosInCurrentBatch.map {
          FilmstripFrame(
            id: $0.id,
            rating: $0.rating,
            aspectRatio: aspectRatio(of: $0.meta)
          )
        },
        selectedID: state.currentPhoto?.id,
        ratingMode: state.ratingMode,
        thumbnailProvider: { [images = state.images] id, size in
          images.thumbnail(for: id, size: size)
        },
        onSelect: { id in
          if let index = state.photosInCurrentBatch.firstIndex(where: { $0.id == id }) {
            state.send(.selectPhoto(index))
          }
        }
      )
      .frame(height: Appearance.filmstripHeight)
      .background(Appearance.barBackground)
      .overlay(alignment: .top) {
        Rectangle().fill(Appearance.separator).frame(height: 0.5)
      }
      .accessibilityLabel("Filmstrip, \(state.photosInCurrentBatch.count) photos")
    }
  }

  private func aspectRatio(of meta: PhotoMeta) -> Double {
    guard meta.width > 0, meta.height > 0 else { return 1.5 }
    let isQuarterTurned = meta.orientation >= 5 && meta.orientation <= 8
    let width = Double(isQuarterTurned ? meta.height : meta.width)
    let height = Double(isQuarterTurned ? meta.width : meta.height)
    return max(0.2, min(5, width / height))
  }
}
