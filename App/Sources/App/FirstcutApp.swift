// Owner: ui. Placeholder entry point; ui replaces it with the real window and toolbar.
import SwiftUI

@main
struct FirstcutApp: App {
    var body: some Scene {
        WindowGroup {
            Text("Firstcut")
                .frame(minWidth: 800, minHeight: 500)
        }
    }
}
