// Owner: ui.
//
// Grid (G, task.md §9.6): the whole current batch as a grid of thumbnails, with ratings and keep
// rings visible. Clicking a cell selects it; double-clicking (or Return / E, the Loupe key) opens it
// in the loupe. Only the current batch is shown: rating is scoped to it (task.md §2), and a grid of
// the whole shoot would be 1,500 cells of decoding nobody asked for.

import SwiftUI

struct GridView: View {
  let state: any CullViewState

  private let columns = [GridItem(.adaptive(minimum: 160, maximum: 240), spacing: 12)]

  var body: some View {
    let photos = state.photosInCurrentBatch
    ScrollViewReader { proxy in
      ScrollView {
        LazyVGrid(columns: columns, spacing: 12) {
          ForEach(Array(photos.enumerated()), id: \.element.id) { index, photo in
            GridCell(
              photo: photo,
              mode: state.ratingMode,
              isSelected: index == state.currentPhotoIndex,
              thumbnail: state.images.thumbnail(for: photo.id, size: CGSize(width: 320, height: 320))
            )
            .id(photo.id)
            // Double-click first: SwiftUI resolves the gesture that asks for more taps ahead of the
            // single tap, so a double-click does not also fire the select.
            .onTapGesture(count: 2) {
              state.send(.selectPhoto(index))
              state.send(.setViewMode(.loupe))
            }
            .onTapGesture { state.send(.selectPhoto(index)) }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(GridCell.accessibilityText(for: photo, mode: state.ratingMode))
            .accessibilityAddTraits(.isButton)
            .accessibilityAddTraits(index == state.currentPhotoIndex ? .isSelected : [])
          }
        }
        .padding(16)
      }
      .onChange(of: state.currentPhotoIndex) { _, index in
        guard photos.indices.contains(index) else { return }
        withAnimation(.easeInOut(duration: 0.15)) { proxy.scrollTo(photos[index].id) }
      }
    }
    .background(Appearance.viewerBackground(darkness: state.viewerBackgroundDarkness))
    .accessibilityLabel("Grid, \(photos.count) photos in this batch")
  }
}

struct GridCell: View {
  let photo: CullPhoto
  let mode: RatingMode
  let isSelected: Bool
  let thumbnail: CGImage?

  private var isKeep: Bool { RatingVisuals.isKeep(photo.rating, mode: mode) }

  var body: some View {
    VStack(spacing: 4) {
      ZStack {
        RoundedRectangle(cornerRadius: 6, style: .continuous).fill(Color.black.opacity(0.35))
        if let thumbnail {
          Image(decorative: thumbnail, scale: 1)
            .resizable()
            .scaledToFit()
            .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
            .padding(2)
        } else {
          ProgressView().controlSize(.small)
        }
      }
      .aspectRatio(3 / 2, contentMode: .fit)
      .overlay {
        // Keep / Not keep mode: a green ring for keeps, a red one for everything else (§6.2).
        if mode == .keep {
          RoundedRectangle(cornerRadius: 6, style: .continuous)
            .strokeBorder(isKeep ? Appearance.keepGreen : Appearance.rejectRed, lineWidth: 2)
        }
      }
      .overlay(alignment: .topTrailing) { flagBadge.padding(5) }
      .padding(4)
      .background(
        RoundedRectangle(cornerRadius: 10, style: .continuous)
          .fill(isSelected ? Appearance.plateFill : .clear))

      HStack(spacing: 6) {
        Text(photo.fileName)
          .font(.caption)
          .lineLimit(1)
          .truncationMode(.middle)
          .foregroundStyle(Appearance.secondaryLabel)
        Spacer(minLength: 0)
        if mode == .stars, RatingVisuals.starCount(for: photo.rating, mode: mode) > 0 {
          Text(String(repeating: "★", count: RatingVisuals.starCount(for: photo.rating, mode: mode)))
            .font(.caption2)
            .foregroundStyle(Color.yellow)
        }
        if let label = photo.rating.label {
          Circle().fill(RatingColor.color(for: label)).frame(width: 8, height: 8)
        }
      }
      .padding(.horizontal, 6)
    }
  }

  @ViewBuilder
  private var flagBadge: some View {
    switch photo.rating.flag {
    case .pick:
      Image(systemName: "flag.fill").foregroundStyle(.white).shadow(radius: 2)
    case .reject:
      Image(systemName: "xmark.circle.fill").foregroundStyle(Appearance.rejectRed).shadow(radius: 2)
    case .none:
      EmptyView()
    }
  }

  static func accessibilityText(for photo: CullPhoto, mode: RatingMode) -> String {
    var parts = [photo.fileName]
    switch mode {
    case .stars:
      let stars = RatingVisuals.starCount(for: photo.rating, mode: mode)
      parts.append(stars == 0 ? "unrated" : "\(stars) star\(stars == 1 ? "" : "s")")
    case .keep:
      parts.append(RatingVisuals.isKeep(photo.rating, mode: mode) ? "kept" : "not kept")
    }
    if photo.rating.flag == .pick { parts.append("picked") }
    if photo.rating.flag == .reject { parts.append("rejected") }
    return parts.joined(separator: ", ")
  }
}
