// Owner: ui.
//
// Compare (C, task.md §9.6): 2-, 3- or 4-up of consecutive frames from the current batch, at the same
// zoom and the same spot. Click a frame to jump every pane to 100% on that detail, and the others
// follow, which is the whole point: is the shutter frame sharper than the one after it?
//
// The panes are the ordinary viewer hosts joined by a `ViewerSyncGroup`, so pinch, click, drag and
// two-finger scroll behave exactly as they do in the loupe. The arrow keys move the candidate:
// the window slides along the batch, and the current frame keeps its highlight. Zoom stays put as
// frames change, otherwise every arrow press would throw the comparison away.

import SwiftUI

struct CompareView: View {
  let state: any CullViewState
  let count: Int

  @State private var group = ViewerSyncGroup()

  struct Pane: Identifiable {
    var index: Int
    var photo: CullPhoto
    var id: PhotoID { photo.id }
  }

  /// The frames on screen: `count` in a row starting at the current one, slid back so the window
  /// is always full near the end of a batch.
  private var window: [Pane] {
    let photos = state.photosInCurrentBatch
    guard !photos.isEmpty else { return [] }
    let size = min(count, photos.count)
    let start = min(max(0, state.currentPhotoIndex), photos.count - size)
    return (start..<start + size).map { Pane(index: $0, photo: photos[$0]) }
  }

  var body: some View {
    let panes = window
    ZStack {
      Appearance.viewerBackground(darkness: state.viewerBackgroundDarkness)
      if panes.count == 4 {
        VStack(spacing: 6) {
          HStack(spacing: 6) { pane(panes[0]); pane(panes[1]) }
          HStack(spacing: 6) { pane(panes[2]); pane(panes[3]) }
        }
      } else {
        HStack(spacing: 6) {
          ForEach(panes) { entry in pane(entry) }
        }
      }
    }
    .padding(6)
    .accessibilityLabel("Compare, \(panes.count) frames")
  }

  private func pane(_ entry: Pane) -> some View {
    let photo = entry.photo
    let isCurrent = entry.index == state.currentPhotoIndex
    var presentation = ViewerPresentation.fit
    // Zoom is locked across frames here: sliding the window must not reset the comparison.
    presentation.isZoomLocked = true
    presentation.showsClipping = state.showsClippingOverlay
    if state.showsAFOverlay {
      presentation.afRects = ViewerPresentation.afRects(
        from: photo.meta.af, orientation: photo.meta.orientation)
    }
    return ZStack(alignment: .bottomLeading) {
      PhotoViewerHostLayer(
        photoID: photo.id, aspectRatio: aspectRatio(of: photo.meta), presentation: presentation,
        syncGroup: group)
      caption(photo)
        .padding(8)
        .allowsHitTesting(false)
    }
    .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
    .overlay {
      RoundedRectangle(cornerRadius: 6, style: .continuous)
        .strokeBorder(isCurrent ? Color.accentColor : Color.white.opacity(0.12), lineWidth: isCurrent ? 2 : 1)
        .allowsHitTesting(false)
    }
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(GridCell.accessibilityText(for: photo, mode: state.ratingMode))
  }

  private func caption(_ photo: CullPhoto) -> some View {
    HStack(spacing: 6) {
      Text(photo.fileName).lineLimit(1)
      if state.ratingMode == .stars {
        let stars = RatingVisuals.starCount(for: photo.rating, mode: state.ratingMode)
        if stars > 0 { Text(String(repeating: "★", count: stars)).foregroundStyle(Color.yellow) }
      } else {
        Text(RatingVisuals.isKeep(photo.rating, mode: state.ratingMode) ? "Keep" : "Not keep")
          .foregroundStyle(
            RatingVisuals.isKeep(photo.rating, mode: state.ratingMode)
              ? Appearance.keepGreen : Appearance.rejectRed)
      }
    }
    .font(.caption.weight(.medium))
    .padding(.horizontal, 8)
    .padding(.vertical, 4)
    .background(.black.opacity(0.55), in: Capsule())
    .foregroundStyle(.white)
  }

  private func aspectRatio(of meta: PhotoMeta) -> Double {
    guard meta.width > 0, meta.height > 0 else { return 1.5 }
    let turned = meta.orientation >= 5 && meta.orientation <= 8
    let width = Double(turned ? meta.height : meta.width)
    let height = Double(turned ? meta.width : meta.height)
    return max(0.2, min(5, width / height))
  }
}
