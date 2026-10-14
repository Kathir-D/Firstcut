// Owner: ui.

import SwiftUI
import Testing

@testable import Firstcut

@Suite("Open Folder button")
@MainActor
struct OpenFolderButtonTests {
    @Test("helpText documents that the first folder is saved first")
    func helpText() {
        #expect(OpenFolderButton.helpText == "Open another folder (⌘O). This one is saved first.")
    }
}
