// Owner: ui.

import AppKit
import SwiftUI

struct Filmstrip: NSViewRepresentable {
    let frames: [FilmstripFrame]
    let selectedID: PhotoID?
    let ratingMode: RatingMode
    let thumbnailProvider: @MainActor (PhotoID, CGSize) -> CGImage?
    let onSelect: @MainActor (PhotoID) -> Void

    func makeNSView(context: Context) -> FilmstripScrollView {
        let view = FilmstripScrollView()
        view.thumbnailProvider = thumbnailProvider
        view.onSelect = onSelect
        view.apply(frames: frames, selectedID: selectedID)
        return view
    }

    func updateNSView(_ view: FilmstripScrollView, context: Context) {
        view.thumbnailProvider = thumbnailProvider
        view.onSelect = onSelect
        view.apply(frames: frames, selectedID: selectedID)
    }

    static func dismantleNSView(_ view: FilmstripScrollView, coordinator: Void) {
        view.thumbnailProvider = nil
        view.onSelect = nil
    }
}
