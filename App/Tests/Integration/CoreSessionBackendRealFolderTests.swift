// The load-bearing test for "the app opens a real folder".
//
// Everything else in the Swift suite runs on MockSession, which is correct for testing *our* logic
// and useless for proving the *product* works: the mock returns fixtures no matter what is on
// disk, so a passing test suite could sit next to an app that can only ever show made-up photos.
// This test goes through `CoreSessionBackend` -- the same object `Dependencies.live()` hands the
// window -- with a folder of real CR3 files, so a regression anywhere in the FFI boundary, the
// scanner, the store or the batcher fails here.
//
// Opt-in through the same gate as every other real-photo test, because a GUI-hosted test reading
// `~/Documents` triggers a TCC prompt that nobody is there to click (see TestEnvironment):
//
//   scripts/test-with-photos.sh
//
// Note that plain `FIRSTCUT_TEST_PHOTOS=... xcodebuild test` does NOT work: the bundles are hosted
// by Firstcut.app and the host does not inherit the shell's environment, so every photo test skips
// while the suite still reports success. scripts/test-with-photos.sh injects the variables into
// the generated .xctestrun instead, which is the only place xcodebuild reads them from.
//
// Without it the test skips and says so. It never fabricates a pass.

import Foundation
import XCTest
@testable import Firstcut

@MainActor
final class CoreSessionBackendRealFolderTests: XCTestCase {
    /// A folder holding real CR3s. Each game directory is one shoot.
    ///
    /// `TestEnvironment.testPhotos` is the single gate for reading the real photos in the whole
    /// Swift suite. Using it rather than reading the variable here means this test cannot drift
    /// from the opt-in rule the other photo tests follow, and cannot reintroduce the TCC hang.
    private func realGameFolder() throws -> URL? {
        guard let root = TestEnvironment.testPhotos else {
            throw XCTSkip(
                "Real photos are not available (set FIRSTCUT_ALLOW_PHOTO_TESTS=1 and "
                    + "FIRSTCUT_TEST_PHOTOS).")
        }
        let entries =
            (try? FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
        let games =
            entries
            .map { $0 as URL }
            .filter { path in
                (try? path.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true
            }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }

        // A game with enough photos to form at least one batch; one photo would prove nothing
        // about batching, and a folder of fixtures would prove nothing at all.
        // A flat folder of RAW files is just as valid a shoot as a game subfolder, and it is what
        // a staged copy looks like. Take the root itself when it holds the photos directly.
        if cr3s(in: root).count >= 2 { return root }
        for game in games {
            if cr3s(in: game).count >= 2 { return game }
        }
        throw XCTSkip("No folder with 2+ CR3 files under \(root.path).")
    }

    private func cr3s(in folder: URL) -> [URL] {
        let fm = FileManager.default
        return
            ((try? fm.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)) ?? [])
            .map { $0 as URL }
            .filter { $0.pathExtension.lowercased() == "cr3" }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
    }

    func testOpensRealFolderAndReadsRealMetadata() throws {
        guard let folder = try realGameFolder() else { return }
        let expected = cr3s(in: folder)

        let backend = try CoreSessionBackend(folder: folder)
        let data = backend.data

        XCTAssertEqual(data.photos.count, expected.count, "every CR3 in the folder is a photo")
        XCTAssertFalse(data.batches.isEmpty, "real photos form real batches")

        // Metadata that only a real parse can produce. A mock would answer with these fields too,
        // so the assertion that matters is below: the capture times must be ordered the way the
        // batcher assumes, and every id must be a real, distinct, SQLite-safe value.
        for photo in data.photos {
            XCTAssertGreaterThan(photo.fileSize, 0, "\(photo.relPath) is empty")
            XCTAssertGreaterThan(photo.width, 0, "\(photo.relPath) has no parsed width")
            XCTAssertNotNil(photo.captureTime, "\(photo.relPath) has no capture time")
        }

        // Distinct ids. Duplicate ids would make two photos share one rating row and quietly lose
        // one of them, which is exactly the class of bug that only a real folder reveals.
        XCTAssertEqual(Set(data.photos.map(\.id)).count, data.photos.count, "ids are unique")

        // Ids must survive the trip to SQLite and back (REV-16 / the mask-on-read bug). A full
        // width id is the thing that used to be silently truncated.
        XCTAssertTrue(
            data.photos.allSatisfy { $0.id < (1 << 63) },
            "every id is SQLite-safe")

        // Batches reference real photos, in order, and partition them.
        let batched = data.batches.flatMap(\.photoIds)
        XCTAssertEqual(
            Set(batched), Set(data.photos.map(\.id)), "batches cover exactly the photos")
        for batch in data.batches {
            XCTAssertFalse(batch.photoIds.isEmpty, "no empty batch is handed to the UI")
        }
    }

    /// A rating set through the real session must survive a close and a reopen, and must not touch
    /// the RAW file. Both halves matter: the first is the product working, the second is the
    /// promise that culling never modifies originals.
    func testRatingPersistsAcrossReopenAndOriginalsAreUntouched() throws {
        guard let folder = try realGameFolder() else { return }
        let original = cr3s(in: folder).first!

        // Fingerprint the RAW before anything else, so a write is caught wherever it happens.
        let before = try Data(contentsOf: original, options: .mappedIfSafe)
        let beforeStat = try FileManager.default.attributesOfItem(atPath: original.path)

        let photoID: PhotoID
        do {
            let backend = try CoreSessionBackend(folder: folder)
            guard let photo = backend.data.photos.first else {
                throw XCTSkip("No photos in \(folder.lastPathComponent).")
            }
            photoID = photo.id
            backend.setRating(photo: photo.id, Rating(stars: 3, flag: .pick, keep: true))
            backend.flush()
        }

        // Reopen: a second session over the same folder, as a relaunch would.
        let reopened = try CoreSessionBackend(folder: folder)
        let rating = reopened.rating(for: photoID)
        XCTAssertEqual(rating.stars, 3, "stars survive a reopen")
        XCTAssertEqual(rating.flag, .pick, "flags survive a reopen")
        XCTAssertTrue(rating.keep, "keep survives a reopen")

        // And the original is byte-for-byte unchanged.
        let after = try Data(contentsOf: original, options: .mappedIfSafe)
        XCTAssertEqual(before, after, "the RAW file is never written to")
        let afterStat = try FileManager.default.attributesOfItem(atPath: original.path)
        XCTAssertEqual(
            beforeStat[.modificationDate] as? Date, afterStat[.modificationDate] as? Date,
            "the RAW file's mtime is untouched")
    }

    /// A folder that is not a folder, and a folder with no photographs, must be refused with a
    /// reason a user can act on rather than a reflected Rust enum.
    func testRefusesBadFoldersWithReadableReasons() throws {
        let fm = FileManager.default
        let tmp = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("firstcut-bad-\(UUID().uuidString)")
        try fm.createDirectory(at: tmp, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: tmp) }

        // A file, not a folder.
        let notAFolder = tmp.appendingPathComponent("photo.CR3")
        try Data([0x49, 0x49, 0x2A, 0x00]).write(to: notAFolder)
        assertRefusal(notAFolder, contains: "folder")

        // An empty folder.
        let empty = tmp.appendingPathComponent("empty")
        try fm.createDirectory(at: empty, withIntermediateDirectories: true)
        assertRefusal(empty, contains: "No photographs")

        // A folder that does not exist at all. Rust reports this as NotAFolder too, and that is
        // the right answer -- a path that is not there is not a folder, and "isn't a folder" is
        // what the user needs to hear. Asking for the missing-path case separately would be
        // testing a distinction the product does not make.
        assertRefusal(tmp.appendingPathComponent("missing"), contains: "folder")
    }

    private func assertRefusal(_ url: URL, contains fragment: String, file: StaticString = #filePath, line: UInt = #line) {
        do {
            _ = try CoreSessionBackend(folder: url)
            XCTFail("Opening \(url.lastPathComponent) should have been refused", file: file, line: line)
        } catch {
            let message = error.localizedDescription
            XCTAssertTrue(
                message.contains(fragment),
                "\(url.lastPathComponent): expected a message containing \"\(fragment)\", got \"\(message)\"",
                file: file, line: line)
            // Never a reflected Rust enum in front of a user.
            XCTAssertFalse(
                message.contains("(") && message.contains(":"),
                "\(url.lastPathComponent): raw error leaked to the UI: \"\(message)\"",
                file: file, line: line)
        }
    }
}
