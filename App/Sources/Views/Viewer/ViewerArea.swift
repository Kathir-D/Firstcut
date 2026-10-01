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

    /// What the host is told besides the photo: zoom lock and which overlays to draw.
    private var presentation: ViewerPresentation {
        var value = ViewerPresentation.fit
        value.isZoomLocked = state.isZoomLocked
        value.showsClipping = state.showsClippingOverlay
        (value.clippingHighlight, value.clippingShadow) = state.clippingThresholds
        if let meta = state.currentPhoto?.meta {
            // So "100%" is 100% of the photograph and not of whatever bitmap the cache happened to hold.
            value.pixelSize = CGSize(width: CGFloat(meta.width), height: CGFloat(meta.height))
            if state.showsAFOverlay {
                value.afRects = ViewerPresentation.afRects(from: meta.af, orientation: meta.orientation)
            }
        }
        return value
    }

    var body: some View {
        ZStack {
            Appearance.viewerBackground(darkness: state.viewerBackgroundDarkness)
            if !PhotoViewerHostView.isRegistered {
                ViewerPlaceholder(photo: state.currentPhoto)
            }
            PhotoViewerHostLayer(
                photoID: state.currentPhoto?.id, aspectRatio: aspectRatio, presentation: presentation
            )
            .opacity(state.currentPhoto == nil ? 0 : 1)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Drawn only when no viewer layer is registered (previews). REV-77: every string here is at least
/// `secondaryLabel` on the near-black viewer background.
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
    let presentation: ViewerPresentation
    var syncGroup: ViewerSyncGroup?

    func makeNSView(context: Context) -> PhotoViewerHostView {
        let view = PhotoViewerHostView(frame: .zero)
        view.aspectRatio = aspectRatio
        view.photoID = photoID
        view.viewerState = presentation
        view.syncGroup = syncGroup
        return view
    }

    func updateNSView(_ view: PhotoViewerHostView, context: Context) {
        view.aspectRatio = aspectRatio
        view.photoID = photoID
        view.viewerState = presentation
    }
}
