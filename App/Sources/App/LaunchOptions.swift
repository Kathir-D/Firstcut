// Owner: ui.
//
// Launch arguments, so a screenshot or a UI check can start the app in a chosen state instead of
// whatever the last run left behind. Used by the phase-exclusivity fix (REV-75) and by qa.
//
//   -FirstcutFolder <path>                            open this real folder at launch
//   -FirstcutMockShoot 1                              build the synthetic shoot instead (screenshots)
//   -FirstcutPhase welcome|loading|culling|finishing   start the mock shoot in that phase
//   -FirstcutBatches <n>                             how many batches the mock builds
//   -FirstcutSeed <n>                                mock seed; the same seed is the same shoot
//   -FirstcutRatingMode stars|keep
//   -FirstcutViewMode loupe|grid|compare2|compare3|compare4
//   -FirstcutFinish 1                                open the Finish sheet
//   -FirstcutSettings 1                              open the Settings window
//   -FirstcutSnapshot <path.png>                     draw the main window into a PNG after a moment
//
// Unknown values are ignored. With no flags at all the app starts on the welcome screen with no
// session, which is the honest default: it will not invent photographs to show.

import Foundation

enum LaunchOptions {
  /// A real folder to open at launch. This is the flag the screenshot check uses, because it is
  /// the only path that produces real pixels from `~/Documents/testing`.
  static var folder: URL? {
    value(for: "-FirstcutFolder").map {
      URL(fileURLWithPath: ($0 as NSString).expandingTildeInPath)
    }
  }

  /// Opt in to `PreviewCullViewState`. Off by default — the generated shoot is for screenshots of a
  /// populated window with no folder to hand, not for what the app does.
  static var usesMockShoot: Bool {
    switch value(for: "-FirstcutMockShoot") {
    case "0", "no", "false": false
    case nil: folder == nil && (startPhase != nil || batchCount != nil || seed != nil)
    default: true
    }
  }

  static var startPhase: CullPhase? {
    switch value(for: "-FirstcutPhase") {
    case "welcome": .welcome
    case "loading": .loading(progress: 0.42)
    case "culling": .culling
    case "finishing": .finishing
    default: nil
    }
  }

  static var batchCount: Int? { intValue(for: "-FirstcutBatches") }

  static var seed: UInt64? {
    intValue(for: "-FirstcutSeed").map { UInt64($0) }
  }

  static var ratingMode: RatingMode? {
    switch value(for: "-FirstcutRatingMode") {
    case "stars": .stars
    case "keep": .keep
    default: nil
    }
  }

  static var viewMode: CullViewMode? {
    switch value(for: "-FirstcutViewMode") {
    case "loupe": .loupe
    case "grid": .grid
    case "compare2": .compare(count: 2)
    case "compare3": .compare(count: 3)
    case "compare4": .compare(count: 4)
    default: nil
    }
  }

  static var opensFinish: Bool { value(for: "-FirstcutFinish") == "1" }
  static var opensSettings: Bool { value(for: "-FirstcutSettings") == "1" }

  /// Where to write a snapshot of the main window, for the screenshot workflow.
  static var snapshotPath: String? { value(for: "-FirstcutSnapshot") }

  /// Everything a caller can override at launch, in one value.
  struct Overrides {
    var phase: CullPhase?
    var batchCount: Int?
    var seed: UInt64?
    var ratingMode: RatingMode?
    var usesMockShoot: Bool

    static let none = Overrides(usesMockShoot: false)
  }

  static var overrides: Overrides {
    Overrides(
      phase: startPhase, batchCount: batchCount, seed: seed, ratingMode: ratingMode,
      usesMockShoot: usesMockShoot)
  }

  private static func value(for key: String) -> String? {
    let arguments = ProcessInfo.processInfo.arguments
    guard let index = arguments.firstIndex(of: key), index + 1 < arguments.count else { return nil }
    return arguments[index + 1]
  }

  private static func intValue(for key: String) -> Int? {
    value(for: key).flatMap(Int.init)
  }
}
