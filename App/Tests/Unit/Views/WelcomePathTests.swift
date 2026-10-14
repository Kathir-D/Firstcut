// Owner: ui.
//
// Plan 2.9: type a folder path on the Welcome screen to open it. `PathInput` itself is covered by
// its own tests, so this pins the one seam the path has to cross out of the field: `openFolder(at:)`
// on the view state reaches `AppModel.open(folder:)` and actually opens a real folder.

import Foundation
import Testing

@testable import Firstcut

@Suite("The Welcome path field opens the folder it names")
@MainActor
struct WelcomePathTests {
    @Test("openFolder(at:) opens a real folder through the model, not just a stub")
    func opensARealFolder() throws {
        let folder = try makeTempFolder()
        let model = AppModel(.preview(game: nil))
        let state = ModelCullViewState(model: model, images: PreviewImageSource(seed: 1))
        #expect(state.phase == .welcome)
        state.openFolder(at: folder)
        #expect(model.phase == .culling, "the open moved the model out of Welcome")
        #expect(state.phase == .culling)
        #expect(model.allPhotos.count == 1, "the folder's photo reached the model")
    }

    /// A real folder on disk, holding one placeholder frame so the open path that reads a folder is
    /// exercised end to end. `AppModel.open(folder:)` scans the folder; the preview/mock factory
    /// treats any recognised photo extension as a frame, so a placeholder file is enough — real CR3
    /// bytes would be the only reason to use them, and none of this reads them.
    private func makeTempFolder() throws -> URL {
        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("FirstcutWelcomePath-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let frame = folder.appendingPathComponent("IMG_0001.CR3")
        try Data("placeholder".utf8).write(to: frame)
        return folder
    }
}
