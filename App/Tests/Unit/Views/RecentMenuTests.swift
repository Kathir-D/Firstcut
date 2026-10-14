// Owner: ui.
//
// The "File > Open Recent" submenu label in `FirstcutCommands.openRecentTitle`.

import Foundation
import Testing

@testable import Firstcut

@Suite("The Open Recent submenu label")
struct RecentMenuTests {
    @Test("A folder's row reads its name and rated-over-total counts")
    func title() {
        let folder = RecentFolder(
            path: "/tmp/Wedding",
            name: "Wedding",
            openedAt: .now,
            totalPhotos: 708,
            ratedPhotos: 412
        )
        #expect(FirstcutCommands.openRecentTitle(folder) == "Wedding — 412 of 708 rated")
    }

    @Test("An empty folder still shows its name and zeros")
    func emptyFolder() {
        let folder = RecentFolder(
            path: "/tmp/Empty",
            name: "Empty",
            openedAt: .now,
            totalPhotos: 0,
            ratedPhotos: 0
        )
        #expect(FirstcutCommands.openRecentTitle(folder) == "Empty — 0 of 0 rated")
    }
}
