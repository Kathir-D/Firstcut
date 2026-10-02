# Contract: Pipeline API (images, prefetch, display)

- **Owner:** pipeline
- **Consumers:** app-logic (tells it where the user is), ui (displays what it provides), qa
- **Version:** v0.4 (draft; frozen as v1.0 at the end of wave 1)

## Swift API

Two protocols, because app-logic and ui need different things from the same cache, and one concrete
type implements both (`App/Sources/Pipeline/ImageProvider.swift:105`).

```swift
/// What the user can reach next. app-logic recomputes it on **every** navigation.
/// (`App/Sources/Session/PipelineMirror.swift:29`)
struct FocusRequest: Hashable, Sendable {
    var windows: [FocusWindow]       // previous, current, next (+ more if the budget allows);
                                     // app-logic supplies the photo IDs in capture order
    var currentPhoto: PhotoID
    var viewportPixelSize: CGSize    // backing pixels of the viewer area (Retina aware); .zero
                                     // until the window reports a size
    var zoomed: Bool                 // at 100% right now
    var zoomLock: Bool               // prefetch 1:1 neighbours
    var exactRaw: Bool               // Settings → Viewer → Exact RAW (T4); read by ImageProvider

    /// The previous/current/next window: the set that must never miss the cache (§7.1).
    var guaranteedWindows: [FocusWindow] { Array(windows.prefix(3)) }
}

struct FocusWindow { var batchID: BatchID; var photoIDs: [PhotoID] }   // PipelineMirror.swift:18

/// app-logic's whole surface (`PipelineMirror.swift:58`).
@MainActor protocol ImageProviding: AnyObject { func setFocus(_ focus: FocusRequest) }

/// ui's surface (`App/Sources/Views/Model/CullViewState.swift:141`). Everything returns from the
/// cache or schedules a decode and returns nil — never blocks, never throws, never re-encodes.
protocol CullImageSource: AnyObject {
    var thumbnailProgress: Double { get }     // fraction of the focus window with a thumbnail
    func thumbnail(for id: PhotoID, size: CGSize) -> CGImage?
    func displayImage(for id: PhotoID, minimumLongestEdge: Int) -> CGImage?
    func histogram(for id: PhotoID) -> CullHistogram?   // 64 bins each for R, G, B and Rec. 601
                                                        // luma, as fractions of the total
}
```

`CullHistogram` is defined by ui (`App/Sources/Views/Model/CullViewState.swift:153`); the bins are
computed by pipeline off a 256-pixel reduction of the cached display image, cached for the life of
the session (`ImageProvider.swift:1362-1404`). A histogram needs the display image, so asking for one
schedules that decode and returns nil until it lands.

Two things the draft said and the code does not:

- **`DisplayImage` does not exist.** The viewer's `layer.contents` takes a `CGImage`, so the
  provider hands out a bare `CGImage` and the pixel size / orientation / colour space travel with
  the focus request and `ViewerPresentation.pixelSize` instead
  (`App/Sources/Views/Viewer/PhotoViewerHost.swift:122`).
- **`fullImage(_:)` is gone, and `clippingMask(_:)` is not on the provider.** A viewer at 100%
  asks `displayImage(for:minimumLongestEdge:)` for the photograph's own pixels
  (`CGImageViewerHost.swift:166-172`); the clipping mask is computed by pipeline from the display
  bitmap it already holds (`ClippingMask.make`, `CGImageViewerHost.swift:484`).
- `exactRaw` is carried from settings into every `FocusRequest` (`AppModel.swift:867`) and is now
  read: `ImageProvider.updateExactRaw` points the T4 tier at the current photograph and
  `DecodeEngine.decodeExactRaw` develops the sensor data through `CIRAWFilter`. See **Cache tiers**
  and **Guarantees** below for what it costs and the one trap in it.

## Cache tiers and the memory budget

`DecodeEngine` (`ImageProvider.swift:464`) is a lock-guarded `final class` — not an `actor`, because
a `CGImageSource` must be created and destroyed on one thread and an actor hop per job would cost
more than the ~300 ms decode it hides (`ImageProvider.swift:39-41`).

| Tier | What the engine actually caches | Where |
| --- | --- | --- |
| T0 | 256 px thumbnails (`prefetchPixels`, default 256, `SettingsModel.swift:161`) | `thumbnails: [PhotoID: Entry]` (`ImageProvider.swift:529`) |
| T1 | **not implemented** — the compressed preview bytes are read from the file per display decode rather than cached | `readBytes` (`ImageProvider.swift:1285`) |
| T2 | Display bitmaps decoded at the viewer's longest backing edge | `displays:` (`ImageProvider.swift:530`); sized by `displayEdges` (`:327`) |
| T3 | The same cache asked for the photograph's own longest edge while zoomed | `displayEdges` full set (`:336-342`) |
| T4 | A true RAW develop of the sensor data (`CIRAWFilter`), while "Exact RAW" is on | The current photograph only — **92 MB** a picture at roughly the preview decode's cost, so one at a time, dropped on move (`exactRaws`). Memory, not latency, is the constraint |

- **T2 is the viewer's backing size, not a guess** (`displayEdges`, `ImageProvider.swift:327-349`),
  floored at `minimumDisplayEdge = 64` and, before the window has reported a size, at
  `fallbackDisplayEdge = 2048` (`:352-355`). T3 is the photo's own edge, asked for the current
  frame while zoomed plus — with zoom lock — the frame the user is about to arrow to (`:336-342`).
- **Budget**: 40% of physical RAM by default, from Settings → Performance
  (`ImageProvider.swift:203`, `SettingsModel.swift:159`), LRU eviction that never touches the focus
  window (`evictLocked`, `ImageProvider.swift:1032`), and a `DispatchSource` memory-pressure
  handler that drops everything outside the focus at once (`shedToFocus`, `:578`).
- **Focus window and priority order**: current photo first, then the rest of its batch, then the
  neighbouring batches ordered nearest-first with **next before previous** at equal distance
  (`nearestFirst`, `ImageProvider.swift:359`). Display (T2/T3) decodes are additionally limited to
  the current batch and to two frames behind / three ahead of the current one
  (`displayPrefetchIDs`, `:372`); display work outside that set is cancelled on every move and the
  rest is re-ranked (`:631-636`).
- **T4 is a `Kind`, not a size** (`DecodeEngine.Kind.exactRaw`), so the cache separation is free
  from `Key`'s `id + kind`. It has to be: a `display` entry is the camera's embedded JPEG and an
  `exactRaw` entry is the sensor data, and they differ by 3–52/255 per pixel. Sharing a kind would
  let a T2 request be answered with RAW pixels — a wrong picture, not a slow one. The one
  photograph-deep rule is `setExactRaw`, which drops the queued, in-flight and cached develop for
  the photograph just left; an in-flight one is marked `cancelled` so `store` discards it rather
  than re-inserting 92 MB (`cancelled`, not `failed`, so asking again retries).
- Decode concurrency: "auto" is `min(4, performance-core count)`, not the core count
  (`resolvedDecodeThreads`, `:184`; `performanceCoreCount` reads `hw.perflevel0.logicalcpu`, `:193`).
- Thumbnails may satisfy a request up to `thumbnailSlack = 1.5×` the prefetch size
  (`ImageProvider.swift:165`, `satisfies`, `:874`), and a cached display bitmap smaller than the
  viewer needs is returned anyway with a bigger decode scheduled (`display`, `:801-820`), counted by
  `stats.displayResizes` — deliberately **not** a focus miss.

## `PipelineStats`

The counters qa and the debug HUD read (`ImageProvider.swift:62-88`; `DebugHUD.swift:43-76`):

`focusMisses`, `thumbnailCacheHits` / `thumbnailCacheMisses` / `thumbnailDecodes`,
`displayCacheHits` / `displayDecodes`, `histogramCacheHits` / `histogramComputes`,
`decodeFailures`, `decodesInProgress`, `queuedDecodes`, `thumbnailBytes`, `displayBytes`,
`focusSize`, `pressureSheds`, `displayResizes`, and for T4 `exactRawDecodes`, `exactRawBytes` and
`exactRawInFlight`.

`decodesInProgress`, `queuedDecodes`, `thumbnailBytes` and `displayBytes` are computed per read
rather than counted (`:601-610`). `ImageProvider.byteRangeDecodes` / `containerDecodes`
(`:144-145`) are the pair that says whether display decodes are coming from the reported full-preview
byte range; 0 of each would mean the optimisation is simply not wired up.

## Signposts

`App/Sources/Pipeline/Signpost.swift` names intervals after the todo.md §7.3 / §7.5 rows rather than
after the functions that contain them (`:69-94`). One `OSSignposter` **per interval**: the engine runs
up to `maxConcurrent` drains at once, so a shared signposter would fuse four real decodes into one
misleading span (`Signpost.swift:18-24`).

| Interval | Emitted by |
| --- | --- |
| `openToFirstPhoto` | `AppModel.open(folder:)`, accepting `.anyFrame` (`AppModel.swift:225`) |
| `keyToFrame` | `photo.previous` / `photo.next` (`AppModel.swift:674`) |
| `batchToFrame` | `batch.previous` / `batch.next` (`AppModel.swift:676`) |
| `zoomToSharp` | click-to-100% and pinch (`AppModel.swift:710`) |
| `decodeThumbnail`, `decodeDisplay`, `decodeFromBytes` | `DecodeEngine.drain` (`ImageProvider.swift:908, 929, 951`) |
| `decodeExactRaw` | `DecodeEngine.run`, kept apart from `decodeDisplay` so a trace can attribute a slow frame to one tier or the other rather than averaging them |
| `setFocus`, `evictions` | the focus update and the LRU pass (`ImageProvider.swift:621, 1034`) |

The four `§7.3` rows that need a *presented* frame are closed by the viewer, not by the model:
`FrameSpan.Accepts.displayOnly` for `keyToFrame` / `batchToFrame` / `zoomToSharp`, `.anyFrame` for
`openToFirstPhoto` (`Signpost.swift:102-113`).

## Viewer view

pipeline owns `CGImageViewerHost` (`App/Sources/Pipeline/CGImageViewerHost.swift:37`): an `NSView`
with `wantsLayer`, whose `imageLayer` has `contentsGravity = .resize` and whose `contents` is the
decoded `CGImage` (`:79-86`). It owns zoom (pinch anchored at the pinch point, click to 100% centred
on the spot clicked, drag / two-finger scroll to pan, zoom lock across photos, `ViewerSyncGroup` for
Compare) and the AF and clipping overlays, drawn as **sublayers** of an `overlayLayer` child of
`imageLayer`: one `CALayer` holding `ClippingMask.make`'s `CGImage` plus one `CAShapeLayer` per AF
rect, green for in-focus and white at 60% for a point that missed (`:303-331`). One geometry
function (`imageRect`, `:249`) derives pinch, click, pan and lock from three numbers, so they cannot
disagree about where the photograph is.

There is no `PhotoViewerLayerView`, and `App/Sources/Render/` — where the draft put it — is empty.
ui declares the protocol `PhotoViewerHost` and embeds whatever conforms
(`PhotoViewerHost.swift:25`); the concrete host is
installed by the one `PhotoViewerHostView.register` call in `AppEnvironment`
(`App/Sources/App/AppEnvironment.swift:99-115`), which resolves the image source and the model from
the **active** `CullViewState` (`state.images`, `state.activeModel`) rather than from the
environment's own provider — that is what lets `-FirstcutMockShoot` swap in a synthetic source
without the viewer rendering black.

`onFramePresented` is the closing end of the measured "arrow key → sharp photo" interval
(`CGImageViewerHost.swift:54`): `present()` wraps the `layer.contents` swap in a `CATransaction`
whose completion block fires after that run-loop turn's layer tree is committed, and reports
`PresentedFrame.display` or `.standIn` (`:203-214`). A stand-in must not close a span that is waiting
for the display decode. `onViewportPixelSize` is the opening end of the same story: the viewer is
the only thing that knows how many pixels it covers, and T2 is decoded at exactly that many
(`:152-160`; `AppModel.setViewportPixelSize`, `AppModel.swift:872`).

**ui** embeds the host and styles the space around it; **app-logic** tells it the zoom state through
`ViewerState` (defined in [app-model.md](app-model.md)), which arrives as `ViewerPresentation`.

## Outputs to other areas

- **VisualSig** for every photo, computed by `VisualSigWorker`
  (`App/Sources/Pipeline/VisualSigWorker.swift:37`) from the same 256 px bitmap the filmstrip uses
  (the worker asks the cache first and publishes what it decodes, so no photograph is decoded twice
  for the two passes, `:128-147`), with the algorithm in [batching.md](batching.md) and the
  signature itself computed by the exported Rust reference (`FirstcutCoreBridge.visualSig`,
  `App/Sources/Shared/CoreBridge.swift:38`). Handed over in chunks of 96 through
  `SessionBackend.submitVisualSigs` (`AppModel.swift:351`), at `.utility` priority with 3 concurrent
  decodes, starting at the photo the user is on and working outward.

## Guarantees

- Nothing in the focus window is decoded on demand *by navigating*. `stats.focusMisses` increments
  only when a request names a photo in the **current** focus window and the cache cannot answer it,
  and it does not claim 0 for a whole session: the first decode of a freshly opened folder
  necessarily misses because nothing is decoded yet
  (`ImageProvider.swift:43-52`). The property that is pinned is "once the prefetch has settled,
  asking for every photo in the window costs zero extra decodes and zero misses".
- A display decode never blanks the viewer: a cached bitmap smaller than the viewer needs is returned
  while the bigger one is scheduled (`ImageProvider.swift:801-820`). `stats.displayResizes` counts
  that, and it must settle at 0 while the window is still.
- Never re-encodes images; caches decoded pixels or the camera's original compressed bytes only. The
  one redraw is a 1:1 conversion into the display layout (`inDisplayLayout`, `:1259`) at
  `interpolationQuality = .none`, which is what makes `layer.contents` a pointer swap with no
  conversion inside the commit (`:1205-1217`).
- Honors the memory budget from settings and sheds under memory pressure: a pressure warning drops
  everything outside the focus window at once, never the current batch (`shedToFocus`, `:578`;
  `evictLocked`, `:1032`).
- Display decodes are ImageIO, which reads the camera's **embedded JPEG preview**, never the sensor
  data: `CGImageSourceCreateThumbnailAtIndex` on a CR3 is a preview read on **macOS 15+** (no
  TIFF/RAW path, no `libraw`), with `kCGImageSourceThumbnailMaxPixelSize` so ImageIO subsamples in
  the DCT and `kCGImageSourceShouldCacheImmediately` so the pixels are not decoded lazily inside the
  commit (`ImageProvider.swift:6-9, 1119-1132, 1194-1203`).
- **T4 ("Exact RAW") is the one exception, and only while the setting is on.** It develops the
  sensor data through `CIRAWFilter`, in a cache kind of its own. Measured on a Canon R8 CR3 by
  alternating the two calls over five rounds: **~0.079 s** for a full-resolution 6000×4000 develop
  against **~0.09 s** for the preview every other decode uses. So a develop is *not* the slow
  option — **92 MB of memory is what keeps it out of the prefetch**, one photograph deep. Its pixels differ from the camera's embedded JPEG by 3–52/255, compared inside one
  pipeline so colour management cannot account for it, so it is a different rendering rather than a
  re-decode (`RealRawDecodeTests.testExactRawDevelopsTheSensorDataNotThePreview`).
- **T4 applies the EXIF orientation exactly once, in the filter.** `CIRAWFilter.orientation`
  defaults to the file's tag and the output geometry follows it, so `decodeExactRaw` must *not*
  also call `applying(orientation:to:)` — every other decode path in the app does, because
  ImageIO's container read does not apply the tag, and doing both rotates an orientation-8 frame
  twice. 26 of Game1JENKS's 708 frames are orientation 8.
  `testExactRawKeepsTheExifOrientation` pins this on geometry, which is what a rotation changes.

## Proposed changes

(none)

## Changelog

- v0.1: initial draft.
- v0.2 (2026-09-30): **REV-38 corrected.** `CALayer.contents` cannot take an `IOSurfaceRef`, so the
  viewer holds a `CGImage` (`CGImageViewerHost`) and display is a layer-contents swap. The IOSurface
  path would need a `CAMetalLayer` and buys nothing for v0.1: the cost in todo.md §7.1 is the decode,
  which `ImageProvider` does ahead of time. `DisplayImage.image` replaces `DisplayImage.surface`. The
  host owns zoom (pinch, click to 100%, pan, zoom lock across photos, synced groups for Compare) and
  the AF and clipping overlays.
- v0.4 (2026-10-01): **T4 shipped.** "Exact RAW" is implemented: `DecodeEngine.Kind.exactRaw` (a
  separate cache kind, so a display request can never be answered with RAW pixels), `exactRaws`
  holding the current photograph only, `setExactRaw` replacing whatever the tier was doing on every
  focus report, and `cancelled` (distinct from `failed`) so a develop in flight when the setting is
  turned off is discarded and still retryable. `decodeExactRaw` uses `CIRAWFilter(imageURL:)` — the
  class method, **not** `CIFilter(name:)`, which returns a filter with no input keys and throws an
  uncatchable `NSException` on `inputImage` — and does **not** apply the EXIF orientation, because
  the filter already has. Gate is `PhotoMeta.kind`, because `CIRAWFilter` returns a usable filter
  for a JPEG and for a junk `.cr3`. Measured and recorded in the tiers table: **~0.079 s and 92 MB**
  a develop against ~0.09 s for the preview — so memory, not latency, is what keeps T4 one
  photograph deep; `scaleFactor` and draft mode do not help. Six unit tests (no photos needed) and
  four integration tests (real CR3s).
- v0.3 (2026-10-01): matched the contract to the shipped code. **`DisplayImage` is gone entirely** —
  the provider returns a bare `CGImage`, `fullImage(_:)` and `clippingMask(_:)` are no longer
  provider methods, and `thumbnailProgress` is a `Double` rather than an `AsyncStream`. The one
  protocol became two (`ImageProviding` for app-logic, `CullImageSource` for ui), `PipelineFocus`
  became `FocusRequest`. Added **Cache tiers and the memory budget** (T0/T2/T3 shipped; T1 and T4
  not — T4 arrived in v0.4), **Signposts** (the interval names and who emits them) and **`PipelineStats`** (the field
  list). Replaced **Viewer view**: the host is `CGImageViewerHost` (`NSView` + `wantsLayer`, an
  `imageLayer` with `contentsGravity = .resize`, overlays as sublayers), installed through
  `PhotoViewerHostView.register` and resolving its source from the *active* `CullViewState`;
  `PhotoViewerLayerView` does not exist and `App/Sources/Render/` is empty. Documented
  `onFramePresented` (the `CATransaction` span that closes "arrow key → sharp photo") and
  `onViewportPixelSize`. Corrected the `focusMisses` guarantee, which never claimed 0 for a whole
  session, added the resize guarantee, and flagged that `exactRaw` is plumbed but T4 is not
  implemented. Noted the macOS 15+ dependency of the ImageIO CR3 preview read.

