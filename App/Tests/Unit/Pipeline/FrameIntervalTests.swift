// Owner: pipeline + app-logic.
//
// The key-to-frame span of todo.md §7.3: a signpost that opens on a keystroke and closes when a
// frame is *committed*. These are the tests that keep it honest, because the failure mode is a
// number that looks fine and measures the wrong thing.
//
// Three properties are worth pinning, and each one fails if the corresponding code is removed:
//
// 1. A span opens on a navigation command and stays open until a frame arrives. Nothing in the model
//    closes it — the view layer does, through `frameDidPresent`.
// 2. A `standIn` does **not** close a span that is waiting for the display decode. If it did, a
//    missed prefetch would report a sub-millisecond key-to-frame over a 256 px thumbnail.
// 3. A navigation that does not move closes its own span. Otherwise a → at the end of the shoot
//    reports the time until the *next* keystroke, which is a wrong number rather than no number.

import Foundation
import Testing

@testable import Firstcut

@MainActor
@Suite("Key to frame (todo.md §7.3)")
struct FrameIntervalTests {
  private static func makeModel(photoCount: Int = 12) -> (AppModel, MockSession) {
    let session = MockSession(photos: FixturePhotos.syntheticPhotos(count: photoCount, burstSize: 3))
    let model = AppModel(.testing(backend: session))
    model.open(session, folderName: "Test")
    return (model, session)
  }

  @Test("A navigation opens a span that only a presented frame closes")
  func theSpanSurvivesTheHandler() throws {
    let (model, _) = Self.makeModel()
    #expect(model.lastFrameLatencyMs == nil)

    model.perform(.photoNext)
    // The handler has returned; SwiftUI has not re-rendered, so nothing has been presented yet.
    #expect(model.lastFrameLatencyMs == nil)
    #expect(model.framesPresented == 0)

    model.frameDidPresent(.display)
    #expect(model.framesPresented == 1)
    let latency = try #require(model.lastFrameLatencyMs)
    #expect(latency >= 0)
    // Recording wall-clock rather than a constant is the whole point: a span that cannot see the
    // clock would report the same number for a keystroke and for a decode that took 200 ms.
    #expect(model.worstFrameLatencyMs ?? 0 >= latency)
  }

  @Test("A stand-in does not close a span waiting for the display decode (todo.md §7.1)")
  func aStandInIsNotASharpPhoto() {
    let (model, _) = Self.makeModel()
    model.perform(.photoNext)
    model.frameDidPresent(.standIn)

    #expect(model.standInFramesPresented == 1)
    #expect(model.lastFrameLatencyMs == nil, "a 256 px stand-in is not the sharp photograph")
    #expect(model.framesPresented == 0)

    // The decode lands a moment later and that *is* the frame the row is about.
    model.frameDidPresent(.display)
    #expect(model.framesPresented == 1)
    #expect(model.lastFrameLatencyMs != nil)
  }

  @Test("A frame that arrives with no span open is ignored rather than counted")
  func framesWithoutASpanAreIgnored() {
    let (model, _) = Self.makeModel()
    model.frameDidPresent(.display)
    model.frameDidPresent(.standIn)

    #expect(model.framesPresented == 0)
    #expect(model.lastFrameLatencyMs == nil)
    // A stand-in with no span is still a stand-in the user saw, so it is still counted.
    #expect(model.standInFramesPresented == 1)
  }

  @Test("A key that moves nothing closes its own span instead of leaking it")
  func aKeyThatDoesNothingDoesNotLeak() {
    let (model, _) = Self.makeModel(photoCount: 3)  // one batch of 3, and → stops at its edge
    model.perform(.photoNext)
    model.perform(.photoNext)
    #expect(model.currentPhotoIndex == 2)

    // The third → moves nothing, so no frame is coming. A span left open here would be closed by
    // the *next* keystroke's frame and reported as a slow key-to-frame for a key that did nothing.
    model.perform(.photoNext)
    #expect(model.currentPhotoIndex == 2)
    #expect(model.framesPresented == 0)

    model.frameDidPresent(.display)
    #expect(model.framesPresented == 0, "the span was already closed: no frame was owed")
    #expect(model.lastFrameLatencyMs == nil)

    // A key that does move is still measured, so the rule did not disable the whole path.
    model.perform(.photoPrevious)
    model.frameDidPresent(.display)
    #expect(model.framesPresented == 1)
  }

  @Test("Batch navigation is measured under its own name, and a batch edge closes immediately")
  func batchEdgesDoNotAccumulate() {
    let (model, _) = Self.makeModel()
    #expect(model.moveBatch(by: -1) == false)
    // `perform` at an edge is the path that used to leave a span open.
    model.perform(.batchPrevious)
    #expect(model.framesPresented == 0)
    #expect(model.lastFrameLatencyMs == nil)

    model.perform(.batchNext)
    model.frameDidPresent(.display)
    #expect(model.framesPresented == 1)
  }

  @Test("Zoom to 100% opens its own span, and zooming back out does not")
  func zoomSpansOnlyOpenWhenZoomingIn() {
    let (model, _) = Self.makeModel()
    model.perform(.toggleZoom(at: NormalizedPoint(x: 0.5, y: 0.5)))
    #expect(model.viewer.zoomed)
    #expect(model.framesPresented == 0, "the span is waiting for the frame that answers the click")
    model.frameDidPresent(.display)
    #expect(model.framesPresented == 1)

    model.perform(.toggleZoom(at: nil))
    #expect(model.viewer.zoomed == false)
    // Zooming out is a transform of a bitmap already on screen: no new frame, so no span to close
    // and — more importantly — no number that would be counted as a zoom-to-sharp.
    #expect(model.framesPresented == 1)
  }

  @Test("Key repeat replaces the open span rather than queueing one per press")
  func keyRepeatReplacesTheSpan() {
    let (model, _) = Self.makeModel()
    model.updateSettings { $0.general.arrowBehaviorAtBatchEnd = .continueIntoNextBatch }
    for _ in 0..<5 { model.perform(.photoNext) }
    #expect(model.currentBatchIndex == 1)
    #expect(model.currentPhotoIndex == 2)
    // Five presses, one span: four were ended by the press after them.
    model.frameDidPresent(.display)
    #expect(model.framesPresented == 1)
  }

  @Test("Closing the session or quitting ends a span that no frame will close")
  func endFrameIntervalOnShutdown() {
    let (model, _) = Self.makeModel()
    model.perform(.photoNext)
    model.closeSession()
    // A frame arriving after the session closed must not be reported as a measurement.
    model.frameDidPresent(.display)
    #expect(model.framesPresented == 0)
    #expect(model.lastFrameLatencyMs == nil)
  }

  @Test("The view state carries the measurement, and the preview state reports none")
  func theViewStateCarriesTheNumber() {
    let (model, _) = Self.makeModel()
    let state = ModelCullViewState(model: model, images: PreviewImageSource(seed: 9))
    #expect(state.lastFrameLatencyMs == nil)
    #expect(state.standInFramesPresented == 0)

    model.perform(.photoNext)
    model.frameDidPresent(.display)
    #expect(state.lastFrameLatencyMs != nil)
    #expect(state.worstFrameLatencyMs != nil)
  }
}
