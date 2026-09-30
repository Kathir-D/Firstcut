# Contract: Pipeline API (images, prefetch, display)

- **Owner:** pipeline
- **Consumers:** app-logic (tells it where the user is), ui (displays what it provides), qa
- **Version:** v0.1 (draft; frozen as v1.0 at the end of wave 1)

## Swift API

```swift
/// What the user can reach next. app-logic calls this on every navigation.
struct PipelineFocus: Sendable {
    var batches: [BatchWindow]        // previous, current, next (+ more if the budget allows); app-logic supplies the photo IDs in order
    var currentPhoto: PhotoID
    var viewportPixelSize: CGSize     // backing pixels of the viewer area (Retina aware)
    var zoomed: Bool                  // at 100% right now
    var zoomLocked: Bool              // prefetch 1:1 neighbours
    var exactRaw: Bool                // T4 decode on
}

@MainActor protocol ImageProvider: AnyObject {
    func setFocus(_ focus: PipelineFocus)

    /// Always non-nil once thumbnails finish (the whole shoot is loaded on open). Returns a placeholder before that.
    func thumbnail(_ id: PhotoID) -> CGImage

    /// Fit-to-viewport image already on the GPU. Non-nil for everything in the focus window,
    /// otherwise it's a bug. Display = assign `surface` to a CALayer's contents.
    func displayImage(_ id: PhotoID) -> DisplayImage?

    /// Full resolution for 100% zoom. Instant when prefetched (zoom lock), otherwise < 150 ms.
    func fullImage(_ id: PhotoID) async -> DisplayImage

    /// Overlays are computed from the display image.
    func histogram(_ id: PhotoID) -> Histogram?
    func clippingMask(_ id: PhotoID) -> DisplayImage?

    var stats: PipelineStats { get }   // cache hits/misses, queue depth, memory per tier (debug HUD + qa)
    var thumbnailProgress: AsyncStream<Double> { get }
}

struct DisplayImage { let image: CGImage;  // was `surface: IOSurfaceRef`; see the v0.2 changelog entry
                       let pixelSize: CGSize; let orientationApplied: Bool; let colorSpace: CGColorSpace }
```

## Viewer view

pipeline also owns `PhotoViewerLayerView` (AppKit, in `Render/`): the zoomable, pannable layer that
shows a `DisplayImage`, handles pinch and click-to-100% gestures, and draws the AF and clipping
overlays. **ui** embeds it and styles the space around it; **app-logic** tells it the zoom state through
`ViewerState` (defined in [app-model.md](app-model.md)).

## Outputs to other areas

- **VisualSig** for every photo, computed from the thumbnail with the algorithm in
  [batching.md](batching.md), delivered through `Session.submit_visual_sigs` in chunks as thumbnails finish.

## Guarantees

- Nothing in the focus window is ever decoded on demand. `stats.focusMisses` must stay 0 (qa checks it).
- Never re-encodes images; caches decoded pixels or the camera's original compressed bytes only.
- Honors the memory budget from settings and sheds tiers under memory pressure, never the current batch.

## Proposed changes

(none)

## Changelog

- v0.1: initial draft.
- v0.2 (2026-09-30): **REV-38 corrected.** `CALayer.contents` cannot take an `IOSurfaceRef`, so the
  viewer holds a `CGImage` (`CGImageViewerHost`) and display is a layer-contents swap. The IOSurface
  path would need a `CAMetalLayer` and buys nothing for v0.1: the cost in task.md §7.1 is the decode,
  which `ImageProvider` does ahead of time. `DisplayImage.image` replaces `DisplayImage.surface`. The
  host owns zoom (pinch, click to 100%, pan, zoom lock across photos, synced groups for Compare) and
  the AF and clipping overlays.

