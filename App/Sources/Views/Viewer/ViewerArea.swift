// Owner: ui.

import AppKit
import SwiftUI

struct ViewerArea: View {
  let state: any CullViewState

  var body: some View {
    ZStack {
      Appearance.viewerBackground(darkness: state.viewerBackgroundDarkness)
      if PhotoViewerHostView.layerViewFactory == nil {
        ViewerPlaceholder(photo: state.currentPhoto)
      }
      PhotoViewerHost(photoID: state.currentPhoto?.id)
        .opacity(state.currentPhoto == nil ? 0 : 1)
    }
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .background(PhaseOverlay(phase: state.phase))
  }
}

struct ViewerPlaceholder: View {
  let photo: CullPhoto?

  var body: some View {
    VStack(spacing: 8) {
      if let photo {
        Text(photo.fileName)
          .font(.system(size: 15, weight: .medium))
          .foregroundStyle(Appearance.secondaryLabel)
        Text(
          "\(photo.meta.width) × \(photo.meta.height) · \(photo.meta.cameraModel ?? "Unknown camera")"
        )
        .font(.system(size: 11))
        .foregroundStyle(Appearance.tertiaryLabel)
      }
      Text("Viewer layer pending from the pipeline agent")
        .font(.system(size: 11))
        .foregroundStyle(Appearance.tertiaryLabel)
    }
    .accessibilityElement(children: .combine)
    .accessibilityLabel(photo.map { "Viewer placeholder for \($0.fileName)" } ?? "Viewer")
  }
}

struct PhotoViewerHost: NSViewRepresentable {
  let photoID: PhotoID?

  func makeNSView(context: Context) -> PhotoViewerHostView {
    let view = PhotoViewerHostView()
    view.photoID = photoID
    return view
  }

  func updateNSView(_ view: PhotoViewerHostView, context: Context) {
    view.photoID = photoID
  }
}

/// Hosts pipeline's `PhotoViewerLayerView` without a compile-time dependency on it. pipeline
/// registers a factory (REQ-ui-2); until then the host stays empty and `ViewerArea` draws the
/// placeholder.
@MainActor
final class PhotoViewerHostView: NSView {
  typealias LayerViewFactory = @MainActor (PhotoID) -> NSView

  nonisolated(unsafe) private static var storedFactory: LayerViewFactory?

  static var layerViewFactory: LayerViewFactory? {
    get { storedFactory }
    set { storedFactory = newValue }
  }

  var photoID: PhotoID? {
    didSet {
      guard photoID != oldValue else { return }
      rebuild()
    }
  }

  private var current: NSView?

  override init(frame frameRect: NSRect) {
    super.init(frame: frameRect)
    wantsLayer = true
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is not used")
  }

  private func rebuild() {
    current?.removeFromSuperview()
    current = nil
    guard let id = photoID, let factory = Self.layerViewFactory else { return }
    let view = factory(id)
    view.frame = bounds
    view.autoresizingMask = [.width, .height]
    addSubview(view)
    current = view
    setAccessibilityLabel("Photo viewer")
  }
}
