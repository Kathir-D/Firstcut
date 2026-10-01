import Foundation
import Testing

@testable import Firstcut

@Suite("Files appearing and vanishing (todo.md §11)")
@MainActor
struct FolderChangeTests {
    private func shoot(count: Int) -> [PhotoMeta] { FixturePhotos.syntheticPhotos(count: count) }

    private func model(over mock: MockSession) -> AppModel {
        let model = AppModel(.testing(backend: mock))
        model.open(mock, folderName: "Shoot")
        return model
    }

    @Test("Only photo files count as a change: Firstcut's own sidecar writes must not cause a rescan")
    func sidecarWritesAreIgnored() {
        #expect(FolderWatcher.touchesPhotos(["/Shoot/IMG_0001.CR3"]))
        #expect(FolderWatcher.touchesPhotos(["/Shoot/a/b/DSC_1.jpg", "/Shoot/IMG_2.xmp"]))
        #expect(FolderWatcher.touchesPhotos(["/Shoot/IMG_0001.CR3.XMP", "/Shoot/.DS_Store"]) == false)
        #expect(FolderWatcher.touchesPhotos([]) == false)
    }

    @Test("New photos appear in the model, and the user stays on the photo they were on")
    func newPhotosAppear() {
        let all = shoot(count: 36)
        let mock = MockSession(photos: Array(all.prefix(24)))
        let model = model(over: mock)
        model.perform(.photoNext)
        model.perform(.photoNext)
        let staying = model.currentPhoto?.id
        #expect(model.progress.totalPhotos == 24)

        mock.nextRescan = SessionData(
            folder: "/mock", photos: all, batches: FixturePhotos.batches(for: all))
        model.folderDidChange()

        #expect(model.progress.totalPhotos == 36)
        #expect(model.currentPhoto?.id == staying, "a re-read must not move the user")
    }

    @Test("Photos that vanish drop out, and the selection is clamped rather than left dangling")
    func photosVanish() {
        let all = shoot(count: 36)
        let mock = MockSession(photos: all)
        let model = model(over: mock)
        // Go to the last photo of the last batch, then remove everything after the first batch.
        for _ in 0..<40 { model.perform(.batchNext) }
        let kept = Array(all.prefix(12))

        mock.nextRescan = SessionData(
            folder: "/mock", photos: kept, batches: FixturePhotos.batches(for: kept))
        model.folderDidChange()

        #expect(model.progress.totalPhotos == 12)
        #expect(model.currentPhoto != nil, "the selection moved to a photo that still exists")
        #expect(kept.contains { $0.id == model.currentPhoto?.id })
    }

    @Test("An unchanged set of photos is not rebuilt, so the caches are kept")
    func unchangedIsANoOp() {
        let all = shoot(count: 24)
        let mock = MockSession(photos: all)
        let model = model(over: mock)
        let batchesBefore = model.batches.map(\.id)
        mock.nextRescan = SessionData(
            folder: "/mock", photos: all, batches: FixturePhotos.batches(for: all))
        model.folderDidChange()
        #expect(model.batches.map(\.id) == batchesBefore)
    }

    @Test("A backend that cannot rescan leaves the model alone")
    func noRescanNoChange() {
        let mock = MockSession(photos: shoot(count: 24))
        let model = model(over: mock)
        let before = model.progress.totalPhotos
        model.folderDidChange()  // `nextRescan` is nil, so `rescan()` answers nil
        #expect(model.progress.totalPhotos == before)
    }
}
