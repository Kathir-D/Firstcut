import Foundation
import Testing
@testable import Firstcut

@Suite("probe")
struct ProbeTests {
    @Test func dates() throws {
        let c = ExifDate.parse("2026:08:27 19:54:49.84-06:00", offsetTimeOriginal: "-06:00")
        print("PROBE-DATE", c?.unixMs ?? -1, c?.offsetMinutes.map(Int.init) ?? 0, c?.subsecResolutionMs ?? 0)
    }

    @MainActor
    @Test func writes() throws {
        let session = MockSession(photos: FixturePhotos.syntheticPhotos(count: 12, burstSize: 3))
        let model = AppModel(.testing(backend: session))
        model.open(session, folderName: "T")
        model.perform(.setStars(5))
        model.moveBatch(by: 1)
        print("PROBE-W", model.currentPhoto?.fileName ?? "-", session.xmpWrites.count)
        model.perform(.setStars(5))
        print("PROBE-W2", model.currentPhoto?.fileName ?? "-", session.xmpWrites.count, session.xmpWrites)
    }

    @MainActor
    @Test func undoOther() throws {
        let session = MockSession(photos: FixturePhotos.syntheticPhotos(count: 12, burstSize: 3))
        let model = AppModel(.testing(backend: session))
        model.open(session, folderName: "T")
        model.perform(.setStars(5))
        let ratedID = model.currentPhoto?.id ?? 0
        print("PROBE-U1", session.canUndo, session.data.ratings.count)
        model.moveBatch(by: 2)
        print("PROBE-U2", model.currentBatchIndex, session.canUndo)
        model.perform(.undo)
        print("PROBE-U3", model.currentBatchIndex, model.currentPhoto?.id ?? 0, ratedID, session.canUndo)
    }
}
