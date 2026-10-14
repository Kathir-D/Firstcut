// Owner: ui.

import AppKit
import SwiftUI
import Testing

@testable import Firstcut

@Suite("Folder Chooser button")
@MainActor
struct FolderChooserButtonTests {
    @Test("configures a directory-only, single-selection open panel")
    func configure() {
        let panel = NSOpenPanel()
        FolderChooserButton.configure(panel)

        #expect(panel.canChooseFiles == false)
        #expect(panel.canChooseDirectories == true)
        #expect(panel.canCreateDirectories == true)
        #expect(panel.allowsMultipleSelection == false)
    }
}
