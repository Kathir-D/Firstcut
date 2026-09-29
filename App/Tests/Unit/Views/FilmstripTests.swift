// Owner: ui.

import AppKit
import CoreGraphics
import QuartzCore
import Testing

@testable import Firstcut

@Suite("Filmstrip rating visuals")
@MainActor
struct RatingVisualsTests {
  @Test("Stars and flags only appear when set")
  func visibility() {
    #expect(RatingVisuals.showsStars(Rating()) == false)
    #expect(RatingVisuals.showsStars(Rating(stars: 1)) == true)
    #expect(RatingVisuals.showsFlag(Rating()) == false)
    #expect(RatingVisuals.showsFlag(Rating(flag: .pick)) == true)
    #expect(RatingVisuals.showsFlag(Rating(flag: .reject)) == true)
  }

  @Test("Keep rings are green for keeps and red for not-keeps (task.md §6.2)")
  func keepRings() {
    #expect(RatingVisuals.keepRingColor(Rating(keep: true), mode: .keep) == NSColor.systemGreen)
    #expect(RatingVisuals.keepRingColor(Rating(), mode: .keep) == NSColor.systemRed)
    #expect(RatingVisuals.keepRingColor(Rating(stars: 4), mode: .keep) == NSColor.systemGreen)
    #expect(RatingVisuals.isKeep(Rating(stars: 3), mode: .keep) == false)
    // 3 stars is a "Good", not a keep: the ring rule is mode-aware and threshold-driven, and the
    // old version here ignored both. A 3-star photo in keep mode must not claim to be a keep, or
    // the Finish step keeps a photo the user only thought was well exposed.
    #expect(RatingVisuals.isKeep(Rating(stars: 3), mode: .stars) == false)
    #expect(RatingVisuals.isKeep(Rating(stars: 5), mode: .stars) == true)
  }

  @Test("A star path is a closed ten-point polygon")
  func starPath() {
    let path = FilmstripBadgeLayer.starPath(in: CGRect(x: 0, y: 0, width: 10, height: 10))
    let box = path.boundingBox
    #expect(path.isEmpty == false)
    #expect(box.minY == 0)
    #expect(box.maxX <= 10)
    #expect(box.maxY <= 10)
    #expect(abs(box.midX - 5) < 0.01)
    #expect(box.width > 9)
    #expect(box.height > 9)
  }
}

@Suite("Filmstrip layout")
@MainActor
struct FilmstripLayoutTests {
  private func frames(_ count: Int) -> [FilmstripFrame] {
    (0..<count).map { index in
      FilmstripFrame(
        id: PhotoID(index + 1), rating: Rating(), aspectRatio: index.isMultiple(of: 2) ? 1.5 : 0.667
      )
    }
  }

  @Test("Frames are laid out left to right at their own aspect ratio")
  func layout() {
    let strip = FilmstripContentView(frame: CGRect(x: 0, y: 0, width: 900, height: 104))
    strip.apply(frames: frames(4), selectedID: 2)
    strip.layout()

    let first = try! #require(strip.frame(for: 1))
    let second = try! #require(strip.frame(for: 2))
    #expect(second.minX > first.maxX)
    #expect(abs(first.width / first.height - 1.5) < 0.01)
    #expect(abs(second.width / second.height - 0.667) < 0.01)
    #expect(
      first.height == 104 - Appearance.filmstripInset * 2 - Appearance.filmstripPlateInset * 2)
  }

  @Test("The content is wider than the viewport so it scrolls")
  func contentWidth() {
    let strip = FilmstripContentView(frame: CGRect(x: 0, y: 0, width: 200, height: 104))
    strip.apply(frames: frames(40), selectedID: 1)
    strip.layout()
    #expect(strip.frame.width > 200)
  }

  @Test("Only the selected frame shows the Finder-style plate")
  func selectionPlate() {
    let strip = FilmstripContentView(frame: CGRect(x: 0, y: 0, width: 900, height: 104))
    strip.apply(frames: frames(3), selectedID: 2)
    strip.layout()

    let plates = (strip.layer?.sublayers ?? []).filter {
      $0.cornerRadius == Appearance.filmstripPlateCornerRadius
    }
    #expect(plates.count == 3)
    #expect(plates.filter { $0.isHidden == false }.count == 1)
    #expect(plates[1].isHidden == false)
  }

  @Test("Changing the selection does not rebuild the layers")
  func selectionUpdate() {
    let strip = FilmstripContentView(frame: CGRect(x: 0, y: 0, width: 900, height: 104))
    strip.apply(frames: frames(3), selectedID: 1)
    strip.layout()
    let before = strip.layer?.sublayers?.count
    strip.apply(frames: frames(3), selectedID: 3)
    strip.layout()
    #expect(before == 9)
    #expect(strip.layer?.sublayers?.count == 9)
  }

  @Test("Sixty frames lay out and stay inside the strip")
  func manyFrames() {
    let strip = FilmstripContentView(frame: CGRect(x: 0, y: 0, width: 1200, height: 104))
    let many = frames(60)
    strip.apply(frames: many, selectedID: 59)
    strip.layout()
    #expect(strip.frame(for: 60) != nil)
    #expect(try! #require(strip.frame(for: 60)).maxY <= 104)
    #expect(strip.frame.width > 1200)
  }
}
