// Owner: ui.
//
// Launch arguments, so a screenshot or a UI check can start the app in a chosen state instead of
// whatever the last run left behind. Used by the phase-exclusivity fix (REV-75) and by qa.
//
//   -FirstcutPhase welcome|loading|culling|finishing   start in that phase
//   -FirstcutBatches <n>                             how many batches the mock builds
//   -FirstcutSeed <n>                                mock seed; the same seed is the same shoot
//   -FirstcutRatingMode stars|keep
//
// Unknown values are ignored and the app starts in its normal state.

import Foundation

enum LaunchOptions {
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

  /// Everything a caller can override at launch, in one value.
  struct Overrides {
    var phase: CullPhase?
    var batchCount: Int?
    var seed: UInt64?
    var ratingMode: RatingMode?

    static let none = Overrides()
  }

  static var overrides: Overrides {
    Overrides(phase: startPhase, batchCount: batchCount, seed: seed, ratingMode: ratingMode)
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
