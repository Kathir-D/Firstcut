// Owner: ui.
//
// Launch arguments, so a screenshot or a UI check can start the app in a chosen state instead of
// whatever the last run left behind. Used by the phase-exclusivity fix (REV-75) and by qa.
//
//   -FirstcutPhase welcome|loading|culling|finishing   start in that phase
//   -FirstcutBatches <n>                             how many batches the mock builds
//   -FirstcutSeed <n>                                mock seed; the same seed is the same shoot
//   -FirstcutRatingMode stars|keep
//   -FirstcutOpenFolder <path>                       open a real folder at launch
//
// Unknown values are ignored and the app starts in its normal state.
//
// ## -FirstcutOpenFolder
//
// Opens a folder on disk at launch instead of showing the welcome screen. It exists because
// "the app opens a real folder" is a claim that has to be demonstrated rather than asserted: a
// screenshot of the welcome screen proves nothing, and driving NSOpenPanel from a script needs
// somebody to click Allow.
//
// It is a launch argument, not a preference, and that distinction is the point. ⌘O and the
// welcome screen's button both go through NSOpenPanel, nothing is remembered between launches, and
// no path is ever defaulted to -- a folder enters this app only when a person hands it over, whether
// by picking it or by typing it on the command line.

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

  /// A folder to open at launch, or nil. Never persisted: see the note in the header.
  static var folderToOpen: URL? {
    guard let path = value(for: "-FirstcutOpenFolder"), !path.isEmpty else { return nil }
    let url = URL(fileURLWithPath: (NSString(string: path) as NSString).expandingTildeInPath)
    return FileManager.default.fileExists(atPath: url.path) ? url : nil
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
