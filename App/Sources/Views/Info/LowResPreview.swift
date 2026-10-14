// Owner: ui.
//
// Whether the photo shown in the viewer came from an embedded JPEG much smaller than the
// sensor, and the label used for that state (loupe badge and info row).

import Foundation

enum LowResPreview {
    /// True when the best embedded preview is at most half the sensor's largest edge, so the
    /// image on screen is a down-scaled JPEG rather than full-size.
    static func isLowRes(_ meta: PhotoMeta) -> Bool {
        let sensorEdge = max(meta.width, meta.height)
        guard sensorEdge != 0 else { return false }
        let best = meta.fullPreview ?? meta.preview
        guard let best else { return false }
        let previewEdge = max(best.width, best.height)
        return previewEdge * 2 <= sensorEdge
    }

    static let label = "Low-res preview"
}
