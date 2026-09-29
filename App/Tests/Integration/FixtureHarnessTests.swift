// Owner: qa.
//
// Smoke tests for the test harness itself. These run everywhere, including CI, because they touch
// only committed fixtures — never the 42 GB in ~/Documents/testing.
//
// They double as the regression net for the measured facts in task.md §3: if core-meta regenerates
// an exiftool dump and something about the shoot changed, these fail loudly instead of the batcher
// quietly re-deriving different thresholds.

import XCTest

final class FixtureHarnessTests: XCTestCase {

    func testRepositoryRootIsFound() throws {
        let root = try XCTUnwrap(TestEnvironment.repositoryRoot, FixtureError.repositoryRootNotFound.description)
        XCTAssertTrue(
            FileManager.default.fileExists(atPath: root.appendingPathComponent("project.yml").path),
            "repositoryRoot resolved to \(root.path), which has no project.yml"
        )
    }

    func testEveryGameHasACommittedExiftoolDump() throws {
        XCTAssertEqual(Game.allCases.count, 4, "task.md §3 says four games")
        for game in Game.allCases {
            let records = try Fixtures.exifToolRecords(for: game)
            XCTAssertEqual(records.count, game.expectedPhotoCount, "\(game.rawValue) record count")
        }
    }

    func testFirstRecordIsFullyPopulated() throws {
        for game in Game.allCases {
            let first = try XCTUnwrap(Fixtures.exifToolRecords(for: game).first, "\(game.rawValue) is empty")
            XCTAssertFalse(first.fileName.isEmpty)
            XCTAssertGreaterThan(first.fileSize, 0, "\(game.rawValue)/\(first.fileName) FileSize")
            XCTAssertNotNil(first.captureUnixMicroseconds, "\(game.rawValue)/\(first.fileName) time")
            XCTAssertNotNil(first.shutterCount, "\(game.rawValue)/\(first.fileName) ShutterCount")
            XCTAssertNotNil(first.serialNumber, "\(game.rawValue)/\(first.fileName) SerialNumber")
            XCTAssertNotNil(first.model, "\(game.rawValue)/\(first.fileName) Model")
            XCTAssertNotNil(first.focalLength, "\(game.rawValue)/\(first.fileName) FocalLength")
            XCTAssertNotNil(first.fNumber, "\(game.rawValue)/\(first.fileName) FNumber")
            XCTAssertNotNil(first.iso, "\(game.rawValue)/\(first.fileName) ISO")
            XCTAssertEqual(first.imageWidth, 6000)
            XCTAssertEqual(first.imageHeight, 4000)
        }
    }

    /// §3: `SubSecTimeOriginal` has 10 ms resolution. If two photos in a game land on the same
    /// 10 ms tick, ordering needs a tie-breaker that the test set never exercises — which is
    /// exactly the kind of silent assumption the fixtures are supposed to catch.
    func testNoTwoPhotosShareASubSecondTick() throws {
        for game in Game.allCases {
            let ticks = try Fixtures.exifToolRecords(for: game).compactMap { record -> Int64? in
                guard let us = record.captureUnixMicroseconds else { return nil }
                return us / 10_000
            }
            XCTAssertEqual(
                ticks.count, game.expectedPhotoCount,
                "\(game.rawValue): a record lost its capture time"
            )
            XCTAssertEqual(
                Set(ticks).count, ticks.count,
                "\(game.rawValue) has photos sharing a 10 ms tick"
            )
        }
    }

    /// §3: `ShutterCount` is monotonic, which is why it is usable as ordering evidence and as a
    /// signal that frames were deleted in-camera.
    func testShutterCountIsMonotonic() throws {
        for game in Game.allCases {
            let counts = try Fixtures.exifToolRecords(for: game).compactMap(\.shutterCount)
            XCTAssertEqual(counts.count, game.expectedPhotoCount, "\(game.rawValue) ShutterCount missing")
            for (previous, next) in zip(counts, counts.dropFirst()) {
                XCTAssertGreaterThanOrEqual(
                    next, previous, "\(game.rawValue) ShutterCount went backwards"
                )
            }
        }
    }

    /// Documented on purpose, because it is a trap: senior-dev verified (REV-26) that file-name
    /// order equals capture order in all 2,880 files, and Game4VRE runs 9146→9999, so the
    /// `IMG_9999 → IMG_0001` rollover never occurs in the test set. A name-based `order()` would
    /// pass every test we have. The synthetic rollover fixture in task.md §11 is the only thing
    /// protecting the rule; see REQ-qa-5.
    func testFileNameOrderCoincidentallyMatchesCaptureOrder() throws {
        for game in Game.allCases {
            let names = try Fixtures.exifToolRecords(for: game).map(\.fileName)
            XCTAssertEqual(
                names, names.sorted(),
                "\(game.rawValue) names are not in capture order; the fixtures are stale or the "
                    + "shoot gained a rollover"
            )
        }
    }

    /// Tests must skip, not fail, when the photos are absent — otherwise the suite is red on CI.
    func testPhotoDiscoveryIsOptional() throws {
        let folders = TestEnvironment.discoveredPhotoFolders()
        if TestEnvironment.testPhotos == nil {
            XCTAssertTrue(folders.isEmpty, "no photos root, but folders were discovered")
            return
        }
        XCTAssertEqual(
            folders.count, 4, "expected the four games under \(TestEnvironment.testPhotos!.path)"
        )
        for folder in folders {
            XCTAssertGreaterThan(TestEnvironment.rawFileCount(in: folder), 0)
        }
    }

    func testAmbiguousTailRangeResolvesToRealPhotos() throws {
        try skipUnlessTestPhotos()
        let game = Game.ambiguousTailGame
        let folder = try XCTUnwrap(game.photos)
        for number in stride(from: 6117, through: 6164, by: 8) {
            let name = String(format: "IMG_%04d.CR3", number)
            XCTAssertNotNil(
                Fixtures.photoURL(name, in: game),
                "\(name) is named in task.md §3 and §5.4 but is not in \(folder.lastPathComponent)"
            )
        }
    }
}
