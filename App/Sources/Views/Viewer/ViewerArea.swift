// Owner: ui.

import AppKit
import SwiftUI

struct ViewerArea: View {
  let state: any CullViewState

  private var aspectRatio: Double {
    guard let meta = state.currentPhoto?.meta, meta.width > 0, meta.height > 0 else { return 1.5 }
    let turned = meta.orientation >= 5 && meta.orientation <= 8
    let width = Double(turned ? meta.height : meta.width)
    let height = Double(turned ? meta.width : meta.height)
    return max(0.2, min(5, width / height))
  }

  var body: some View {
    ZStack {
      Appearance.viewerBackground(darkness: state.viewerBackgroundDarkness)
      if !PhotoViewerHostView.isRegistered {
        ViewerPlaceholder(photo: state.currentPhoto)
      }
      PhotoViewerHostLayer(photoID: state.currentPhoto?.id, aspectRatio: aspectRatio)
        .opacity(state.currentPhoto == nil ? 0 : 1)
    }
    .frame(maxWidth: .infinity, maxHeight: .infinity)
  }
}

/// REV-77: every string here is at least `secondaryLabel` on the near-black viewer background.
/// The pipeline note is a temporary line, but it is the first text a reviewer reads, and dim greys
/// are exactly what survives to release.
struct ViewerPlaceholder: View {
  let photo: CullPhoto?

  var body: some View {
    VStack(spacing: 8) {
      if let photo {
        Text(photo.fileName)
          .font(.system(size: 15, weight: .medium))
          .foregroundStyle(Appearance.primaryLabel)
        Text(
          "\(photo.meta.width) × \(photo.meta.height) · \(photo.meta.cameraModel ?? "Unknown camera")"
        )
        .font(.system(size: 11))
        .foregroundStyle(Appearance.secondaryLabel)
      }
      Text("Viewer layer pending from the pipeline agent")
        .font(.system(size: 11))
        .foregroundStyle(Appearance.secondaryLabel)
    }
    .accessibilityElement(children: .combine)
    .accessibilityLabel(photo.map { "Viewer placeholder for \($0.fileName)" } ?? "Viewer")
  }
}

/// SwiftUI wrapper around the AppKit host. Named `…Layer` so it does not collide with the
/// `PhotoViewerHostView` class it wraps.
struct PhotoViewerHostLayer: NSViewRepresentable {
  let photoID: PhotoID?
  let aspectRatio: Double

  func makeNSView(context: Context) -> PhotoViewerHostView {
    let view = PhotoViewerHostView(frame: .zero)
    view.aspectRatio = aspectRatio
    view.photoID = photoID
    return view
  }

  func updateNSView(_ view: PhotoViewerHostView, context: Context) {
    view.aspectRatio = aspectRatio
    view.photoID = photoID
  }
}
