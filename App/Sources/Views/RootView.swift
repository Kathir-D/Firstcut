// Owner: ui.
//
// Finder gallery layout (task.md §9): large viewer, filmstrip of the current batch underneath,
// floating HUD, right-hand info inspector. The window chrome lives in App/ToolbarController.swift.

import SwiftUI

struct RootView: View {
  let state: any CullViewState

  var body: some View {
    HStack(spacing: 0) {
      ZStack(alignment: .bottom) {
        ViewerArea(state: state)

        if state.isHUDVisible {
          VStack {
            Spacer(minLength: 0)
            ProgressHUD(progress: state.progress)
              .padding(.bottom, Appearance.filmstripHeight + 16)
          }
          .allowsHitTesting(false)
        }

        filmstrip
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity)

      if state.isInfoPanelVisible {
        InfoPanel(state: state)
          .transition(.move(edge: .trailing).combined(with: .opacity))
      }
    }
    .background(Appearance.windowBackground)
    .animation(.easeInOut(duration: 0.18), value: state.isInfoPanelVisible)
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
