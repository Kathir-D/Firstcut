// Owner: ui.

import Testing

@testable import Firstcut

@Suite("Photo formatting")
struct PhotoFormatterTests {
  @Test("Shutter speeds read like a camera")
  func shutter() {
    #expect(PhotoFormatter.shutter(0.0005) == "1/2000")
    #expect(PhotoFormatter.shutter(0.004) == "1/250")
    #expect(PhotoFormatter.shutter(1.3) == "1.3 s")
    #expect(PhotoFormatter.shutter(nil) == "—")
    #expect(PhotoFormatter.shutter(0) == "—")
  }

  @Test("Aperture, ISO and focal length")
  func exposure() {
    #expect(PhotoFormatter.aperture(2.8) == "f/2.8")
    #expect(PhotoFormatter.aperture(nil) == "—")
    #expect(PhotoFormatter.iso(800) == "800")
    #expect(PhotoFormatter.iso(0) == "—")
    #expect(PhotoFormatter.focalLength(200) == "200 mm")
    #expect(PhotoFormatter.focalLength(23.5) == "23.5 mm")
    #expect(PhotoFormatter.exposureCompensation(0) == "0 EV")
    #expect(PhotoFormatter.exposureCompensation(-0.7) == "-0.7 EV")
  }

  @Test("Dimensions and file size")
  func size() {
    #expect(PhotoFormatter.dimensions(width: 6000, height: 4000) == "6000 × 4000")
    #expect(PhotoFormatter.fileSize(13_200_000).contains("MB"))
  }

  @Test("Capture time keeps Canon sub-seconds at 10 ms resolution")
  func captureTime() {
    let canon = CaptureTime(
      unixMs: 1_787_324_089_840, subsecResolutionMs: 10, offsetMinutes: -360, source: .exif)
    #expect(PhotoFormatter.captureTime(canon).hasSuffix(".84"))

    let whole = CaptureTime(
      unixMs: 1_787_324_089_000, subsecResolutionMs: 1000, offsetMinutes: nil, source: .fileModified
    )
    #expect(PhotoFormatter.captureTime(whole).hasSuffix(".") == false)

    #expect(PhotoFormatter.captureTime(nil) == "—")
  }

  @Test("Elapsed time")
  func elapsed() {
    #expect(PhotoFormatter.elapsed(0) == "0:00")
    #expect(PhotoFormatter.elapsed(65) == "1:05")
    #expect(PhotoFormatter.elapsed(3_725) == "1:02:05")
  }
}

// REV-69: there is exactly ONE rating-mode mapping, in `RatingRules` (app-logic), mirroring
// core-store's Rust `Rating::tier` / `display_rating`. This file used to hold a second copy,
// `RatingTiers` in `App/Sources/Views/`, and the two *disagreed*: ui's said a keep with 0 stars
// was Unrated in stars mode, which is the exact bug REV-69 describes — the user's keeps appear
// to vanish when they switch modes. The duplicate is deleted; these tests now pin the one rule,
// including the cross-mode cases the copies disagreed about.
@Suite("Rating tiers")
struct TierTests {
  @Test("Stars map to the tiers in task.md §6.1")
  func stars() {
    #expect(RatingRules.tier(of: Rating(stars: 5), mode: .stars) == .keep)
    #expect(RatingRules.tier(of: Rating(stars: 4), mode: .stars) == .keep)
    #expect(RatingRules.tier(of: Rating(stars: 3), mode: .stars) == .good)
    #expect(RatingRules.tier(of: Rating(stars: 2), mode: .stars) == .maybe)
    #expect(RatingRules.tier(of: Rating(stars: 1), mode: .stars) == .maybe)
    #expect(RatingRules.tier(of: Rating(), mode: .stars) == .unrated)
  }

  @Test("A reject flag wins over the star count")
  func reject() {
    #expect(RatingRules.tier(of: Rating(stars: 5, flag: .reject), mode: .stars) == .rejected)
    #expect(RatingRules.tier(of: Rating(stars: 5, flag: .reject), mode: .keep) == .rejected)
  }

  @Test("Keep mode: keep and unrated only, no stars tiers")
  func keepMode() {
    #expect(RatingRules.tier(of: Rating(keep: true), mode: .keep) == .keep)
    #expect(RatingRules.tier(of: Rating(), mode: .keep) == .unrated)
    // 3 stars is "good" and the Finish step does not keep it, so keep mode must not claim it.
    #expect(RatingRules.tier(of: Rating(stars: 3), mode: .keep) == .unrated)
  }

  @Test("isKeep follows the stored keep flag or a full star rating")
  func isKeep() {
    #expect(RatingRules.isKeep(Rating(keep: true), mode: .stars))
    #expect(RatingRules.isKeep(Rating(stars: 4), mode: .stars))
    #expect(RatingRules.isKeep(Rating(stars: 3), mode: .stars) == false)
  }

  @Test("A keep reads as a keep in BOTH modes, and a full star rating as a keep in both")
  func crossMode() {
    // These are the two cells the deleted ui copy and the old RatingRules disagreed about. If
    // either regresses, keeps vanish on a mode switch or the ring lies about the Finish step.
    #expect(RatingRules.tier(of: Rating(keep: true), mode: .stars) == .keep)
    #expect(RatingRules.tier(of: Rating(keep: true), mode: .keep) == .keep)
    #expect(RatingRules.tier(of: Rating(stars: 4), mode: .keep) == .keep)
    #expect(RatingRules.tier(of: Rating(stars: 5), mode: .keep) == .keep)
    // …and the converse: 1-3 stars are not a keep in keep mode, because Finish will not keep them.
    for stars in 1...3 {
      #expect(RatingRules.tier(of: Rating(stars: UInt8(stars)), mode: .keep) == .unrated)
    }
  }
}
