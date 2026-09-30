// Owner: qa.
//
// The performance suite skeleton (todo.md §7.3). The real measurements land in wave 2, once the
// pipeline exists; what has to exist in wave 1 is the harness and the baseline format, so the
// numbers are recorded the same way every time from the first run.
//
// Run ONLY this bundle, on a quiet machine, with the photos present:
//   xcodebuild -project Firstcut.xcodeproj -scheme Firstcut-Perf \
//     -destination 'platform=macOS,arch=arm64' test
// (scheme pending: REQ-qa-3). Until then the bundle also runs under the `Firstcut` scheme, and
// every timing test skips itself when the machine is too busy or the photos are missing.

import XCTest

final class PerformanceHarnessTests: XCTestCase {

    /// Baselines in docs/qa/perf-baselines.md are per game and per machine. A run with no photos
    /// cannot produce a comparable number, so it must skip rather than write a meaningless row.
    func testPhotosAreDiscoveredOrTheRunIsSkipped() throws {
        try skipUnlessTestPhotos()
        for game in Game.allCases {
            let folder = try XCTUnwrap(game.photos, "\(game.rawValue) folder missing under the photos root")
            let count = TestEnvironment.rawFileCount(in: folder)
            XCTAssertEqual(
                count, game.expectedPhotoCount,
                "\(game.rawValue) has \(count) RAW files, the fixtures say \(game.expectedPhotoCount). "
                    + "Fix the fixture or the expectation — do not just relax the test."
            )
        }
    }

    /// §7.3 timings are only comparable on an idle machine, so every timing test starts with
    /// `skipUnlessMachineIsQuiet()` and the run that records a baseline must have passed it. Until
    /// wave 2 there is nothing to time yet, so this file only proves the harness resolves.
    func testHarnessResolvesThePhotosRoot() throws {
        if TestEnvironment.testPhotos == nil { return }
        XCTAssertNotNil(Game.allCases.first?.photos)
    }

    /// The four games must stay the four games. A shoot disappearing from the machine is a
    /// measurement problem, not a reason to silently shrink the suite.
    func testAllFourGamesAreAvailableForMeasurement() throws {
        try skipUnlessTestPhotos()
        let expected = Set(Game.allCases.map(\.photoFolderName))
        let found = Set(TestEnvironment.discoveredPhotoFolders().map(\.lastPathComponent))
        XCTAssertEqual(
            found, expected,
            "photos root does not contain exactly the four games; baselines would not be comparable"
        )
    }
}
