// Owner: app-logic.
//
// todo.md §7.3's "folder open → first photo on screen < 1 s", from the model's side.
//
// The core side (which file to name, which header to read, that the answer is the same one the full
// scan gives) is asserted in `meta/mod.rs` and `tests/cr3_exiftool.rs`. What is left to assert here
// is the part that can go wrong *after* that read: that a frame built from one header reaches the
// screen, that nothing the user can do writes through the previous folder's backend while it is up,
// and that the real open replaces it with a shoot that is whole.

import Foundation
import Testing

@testable import Firstcut

/// A session open that can be held mid-flight, so the provisional frame is observable rather than
/// a race. An actor, because the model reads it from the main actor and a test releases it from an
/// arbitrary one.
private actor OpenGate {
    private var isOpen = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    /// Suspends until `release()` is called. Returns at once for an already-released gate, so a test
    /// that releases early does not deadlock.
    func wait() async {
        if isOpen { return }
        await withCheckedContinuation { continuation in
            waiters.append(continuation)
        }
    }

    func release() {
        isOpen = true
        for waiter in waiters { waiter.resume() }
        waiters.removeAll()
    }
}

@MainActor
@Suite("The first photo arrives before the shoot does (todo.md §7.3)")
struct FirstPhotoFastPathTests {
    /// 12 photos, one batch of 3 per burst — enough to tell a real shoot from a placeholder frame.
    /// Nonisolated because the fast path is read off the main actor: that is the whole point of it.
    private nonisolated static let photos = FixturePhotos.syntheticPhotos(count: 12, burstSize: 3)

    /// A model whose folder open can be held open, with a fast path that answers immediately.
    ///
    /// - Parameters:
    ///   - gate: released to let the session arrive. nil means the open fails.
    ///   - first: what the fast path reads, so a test can hand back a name, a different folder's
    ///     photo, or nothing at all.
    private static func model(
        gate: OpenGate?, fails: Bool = false,
        first: (@Sendable (URL) -> PhotoMeta?)? = { _ in photos[0] }
    ) -> AppModel {
        let dependencies = Dependencies(
            backend: MockSession(photos: photos),
            asyncSessionFactory: { _ in
                guard let gate else {
                    throw CocoaError(.fileReadNoSuchFile)
                }
                // Held so the frame is up first: a read that fails instantly never has one, and the
                // question here is what a frame does when the read it belongs to then fails.
                await gate.wait()
                if fails { throw CocoaError(.fileReadNoSuchFile) }
                return MockSession(photos: photos)
            },
            firstPhoto: first)
        return AppModel(dependencies)
    }

    /// Waits for a main-actor condition, because the fast path finishes on the main actor from a
    /// detached read and nothing else will let the test observe it in between.
    private static func waitFor(
        _ what: String, _ condition: @MainActor () -> Bool, timeout: TimeInterval = 5
    ) async throws {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition() {
            if Date() > deadline {
                Issue.record("\(what) never happened within \(timeout)s")
                return
            }
            await Task.yield()
            try? await Task.sleep(for: .milliseconds(2))
        }
    }

    @Test("One photograph is on screen while the folder is still being read")
    func aFrameIsUpBeforeTheScanLands() async throws {
        let gate = OpenGate()
        let model = Self.model(gate: gate)
        let folder = URL(fileURLWithPath: "/tmp/Game1JENKS", isDirectory: true)

        model.open(folder: folder)
        try await Self.waitFor("the provisional frame") { model.phase == .culling }

        #expect(model.allPhotos.count == 1, "one header read, one photograph")
        #expect(model.batches.count == 1)
        #expect(model.batches[0].provisional, "one batch, and it says it is provisional")
        #expect(model.currentPhoto?.id == Self.photos[0].id)
        #expect(model.currentPhoto?.fileName == "IMG_0001.CR3")
        #expect(model.folderName == "Game1JENKS", "the window names the folder being read")

        // The scan lands, and it replaces the frame rather than adding to it.
        await gate.release()
        try await Self.waitFor("the real open") { model.canAct }
        #expect(model.allPhotos.count == 12)
        #expect(model.batches.count == 4)
        #expect(
            model.batches.contains(where: { $0.id == AppModel.provisionalBatchID }) == false,
            "the frame's own batch is gone, whatever the real shoot's batches are"
        )
        #expect(
            model.allPhotos.contains { $0.id == Self.photos[0].id },
            "the photograph the frame showed is in the shoot — the same id, so no second decode"
        )
    }

    @Test("Nothing the user can do writes through the previous folder's backend")
    func everyMutationRefusesWhileTheFrameIsUp() async throws {
        let gate = OpenGate()
        // A shoot already open, which is what a rating here would otherwise land in.
        let previous = MockSession(photos: Self.photos)
        var dependencies = Dependencies(
            backend: previous,
            asyncSessionFactory: { _ in
                await gate.wait()
                return MockSession(photos: Self.photos)
            },
            firstPhoto: { _ in Self.photos[0] })
        dependencies.images = MockImageProvider()
        let model = AppModel(dependencies)
        model.open(previous, folderName: "OldShoot")
        model.perform(.setStars(4))
        let starsBefore = previous.data.ratings

        model.open(folder: URL(fileURLWithPath: "/tmp/Game1JENKS", isDirectory: true))
        try await Self.waitFor("the provisional frame") { model.phase == .culling }
        #expect(model.canAct == false, "the frame is display-only, so nothing may act on it")

        model.perform(.setStars(5))
        model.perform(.toggleKeep)
        model.perform(.rejectFlag)
        model.rateCurrent { $0.label = .red }
        model.undo()
        model.redo()
        model.startFinish()

        #expect(model.currentPhoto?.rating == Rating(), "no rating on the provisional photograph")
        #expect(previous.data.ratings == starsBefore, "and none written to the previous shoot")
        #expect(model.finish == .hidden, "Finish cannot plan a one-photo placeholder shoot")
        #expect(
            model.recents.allSatisfy { $0.name != "Game1JENKS" },
            "'1 of 1 rated' is not a shoot, so it is not recorded as one")

        // Once the session is there, the same keystroke lands.
        await gate.release()
        try await Self.waitFor("the real open") { model.canAct }
        model.perform(.setStars(5))
        #expect(model.currentPhoto?.rating.stars == 5)
    }

    @Test("A folder that fails to open puts the provisional frame away")
    func aFailedOpenLeavesNoFrameBehind() async throws {
        let gate = OpenGate()
        let model = Self.model(gate: gate, fails: true)
        model.open(folder: URL(fileURLWithPath: "/tmp/Game1JENKS", isDirectory: true))
        try await Self.waitFor("the provisional frame") { model.phase == .culling }
        #expect(model.allPhotos.count == 1)

        await gate.release()
        try await Self.waitFor("the open to fail") { model.lastError != nil }
        #expect(model.allPhotos.isEmpty, "a photograph from a folder that would not open is not up")
        #expect(model.batches.isEmpty)
        #expect(model.canAct == false)
        #expect(model.phase == .welcome)
    }

    @Test("A failed open leaves the shoot that was open on screen, as the slow path always did")
    func aFailedOpenKeepsThePreviousShoot() async throws {
        let previous = MockSession(photos: Self.photos)
        let gate = OpenGate()
        let model = Self.model(gate: gate, fails: true)
        model.open(previous, folderName: "OldShoot")
        model.perform(.photoNext)

        model.open(folder: URL(fileURLWithPath: "/tmp/Game1JENKS", isDirectory: true))
        try await Self.waitFor("the provisional frame") { model.phase == .culling }
        #expect(model.allPhotos.count == 1, "the frame is up, over the old shoot")
        await gate.release()
        try await Self.waitFor("the open to fail") { model.lastError != nil }

        #expect(model.lastError != nil)
        #expect(model.allPhotos.count == 12, "the old shoot is whole again")
        #expect(model.currentPhotoIndex == 1, "and the user is where they were")
        #expect(model.folderName == "OldShoot")
        #expect(model.canAct, "so it is a real shoot to act on again")
    }

    @Test("Moving to another folder takes the first folder's frame down")
    func aSupersededOpenDoesNotLeaveItsFrameUp() async throws {
        let second = OpenGate()
        let model = Self.model(
            gate: second,
            first: { url in
                // The frame is the photograph the *open* named, so the folder is legible in the answer.
                var meta = Self.photos[0]
                meta.relPath = url.lastPathComponent + "/" + meta.relPath
                meta.id = FixturePhotos.stableID(meta.relPath)
                return meta
            })
        model.open(folder: URL(fileURLWithPath: "/tmp/Game1JENKS", isDirectory: true))
        try await Self.waitFor("the first folder's frame") { model.phase == .culling }
        #expect(model.folderName == "Game1JENKS")
        let firstFrame = model.currentPhoto?.id

        // The user picks another folder before the first has finished being read.
        model.open(folder: URL(fileURLWithPath: "/tmp/Game3KC", isDirectory: true))
        #expect(model.phase.isLoading, "the second read has started")
        #expect(model.allPhotos.isEmpty, "the first folder's frame came down with it")
        #expect(model.folderName == "", "and it is not attributed to the folder now loading")
        try await Self.waitFor("the second folder's frame") { model.folderName == "Game3KC" }
        #expect(model.allPhotos.count == 1, "one frame, not two folders' worth of photographs")
        #expect(model.currentPhoto?.id != firstFrame)

        await second.release()
        try await Self.waitFor("the real open") { model.canAct }
        #expect(model.folderName == "Game3KC", "the folder that finished is the folder on screen")
        #expect(model.allPhotos.count == 12)
    }

    @Test("A folder with no first photograph opens exactly as slowly as it always did")
    func noFirstPhotoMeansTheOldPath() async throws {
        let gate = OpenGate()
        let model = Self.model(gate: gate, first: { _ in nil })
        model.open(folder: URL(fileURLWithPath: "/tmp/Empty", isDirectory: true))

        // Nothing to show, so the loading screen stays up and the shoot is not half-built.
        for _ in 0..<50 { await Task.yield() }
        #expect(model.phase.isLoading)
        #expect(model.allPhotos.isEmpty)

        await gate.release()
        try await Self.waitFor("the real open") { model.canAct }
        #expect(model.allPhotos.count == 12)
    }

    @Test("Closing a shoot that is only a provisional frame goes back to Welcome")
    func closingAProvisionalFrameGoesHome() async throws {
        let gate = OpenGate()
        let model = Self.model(gate: gate)
        model.open(folder: URL(fileURLWithPath: "/tmp/Game1JENKS", isDirectory: true))
        try await Self.waitFor("the provisional frame") { model.phase == .culling }

        model.closeSession()
        #expect(model.phase == .welcome)
        #expect(model.allPhotos.isEmpty)
        #expect(model.canAct == false)
        #expect(model.folderName == "")
    }

    @Test("The previous folder's watcher cannot rescan into a provisional frame")
    func aWatcherEventDoesNotRebuildTheFrame() async throws {
        // A shoot whose folder is about to change on disk: the watcher that reports that change
        // belongs to *this* shoot, and the frame on screen belongs to the folder still loading.
        let previous = MockSession(photos: Self.photos)
        previous.nextRescan = MockSession(photos: Array(Self.photos[0..<6])).data
        let gate = OpenGate()
        let model = Self.model(gate: gate)
        model.open(previous, folderName: "OldShoot")
        #expect(model.allPhotos.count == 12)

        model.open(folder: URL(fileURLWithPath: "/tmp/Game1JENKS", isDirectory: true))
        try await Self.waitFor("the provisional frame") { model.phase == .culling }

        // The old folder changed while the new one was being read. The frame must not be rebuilt
        // around the old folder's files — it says it is the folder being read, and the backend that
        // would be rescanned is not the one on screen.
        model.folderDidChange()
        #expect(model.allPhotos.count == 1, "one photograph, still the one the fast path read")
        #expect(model.folderName == "Game1JENKS")
        #expect(previous.data.photos.count == 12, "the event was dropped, not served late")

        await gate.release()
        try await Self.waitFor("the real open") { model.canAct }
        #expect(model.allPhotos.count == 12)
    }
}
