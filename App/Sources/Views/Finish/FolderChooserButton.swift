// Owner: ui.

import AppKit
import SwiftUI

struct FolderChooserButton: View {
    let path: Binding<String>

    var body: some View {
        Button("Choose…") {
            let panel = NSOpenPanel()
            Self.configure(panel)
            if panel.runModal() == .OK, let url = panel.url {
                path.wrappedValue = url.path
            }
        }
    }

    /// Directories only, New Folder allowed, one selection.
    static func configure(_ panel: NSOpenPanel) {
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
    }
}
