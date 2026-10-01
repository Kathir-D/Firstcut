// Owner: ui.
//
// Window, toolbar and menu wiring. The window is a SwiftUI `Window` scene so the toolbar picks up
// the system Liquid Glass on macOS 26 and the material fallback on 15 for free; `AppDelegate`
// forces the dark appearance app-wide (todo.md §2).

import AppKit
import SwiftUI

@main
struct FirstcutApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @State private var environment = AppEnvironment.shared

    var body: some Scene {
        Window("Firstcut", id: WindowID.main) {
            WindowContent()
                .environment(environment)
                .frame(minWidth: 900, minHeight: 600)
                .preferredColorScheme(.dark)
                .background(WindowAccessor())
                .toolbar { FirstcutToolbar(environment: environment) }
        }
        .defaultSize(width: 1440, height: 900)
        .commands { FirstcutCommands(environment: environment) }

        // ⌘, opens this (todo.md §9.8); SwiftUI adds the "Settings…" item to the app menu itself.
        Settings {
            SettingsView(model: environment.model)
        }
    }
}

enum WindowID {
    static let main = "main"
    /// Set on the culling window by `WindowAccessor`, so the key router can tell it from a sheet or
    /// the Settings window.
    static let mainIdentifier = NSUserInterfaceItemIdentifier("firstcut.main")
}

private struct WindowContent: View {
    @Environment(AppEnvironment.self) private var environment

    var body: some View {
        RootView(state: environment.state)
            .onDrop(of: [.fileURL], isTargeted: nil) { providers in
                environment.handleDrop(providers)
            }
            .background(SettingsLaunchOpener())
    }
}

/// `-FirstcutSettings 1` opens the Settings window at launch, for the screenshot workflow.
///
/// This sends the AppKit action directly instead of SwiftUI's `openSettings()`. The environment
/// action silently did nothing on the shipping toolchain (macOS 27), which left the screenshot
/// workflow without a Settings screen, and it is also the wrong tool here: it *throws* on the
/// macOS 15 SDK and does *not* throw on the macOS 26+ one, so no spelling of it compiles under both
/// toolchains CI and the app ship with. `showSettingsWindow:` is the action the Settings menu item
/// itself sends, it has worked since macOS 14, and it has no compiler-visible spelling at all.
private struct SettingsLaunchOpener: View {
    var body: some View {
        Color.clear.task {
            guard LaunchOptions.opensSettings else { return }
            try? await Task.sleep(for: .seconds(1))
            NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
        }
    }
}

struct WindowAccessor: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        DispatchQueue.main.async { configure(view.window) }
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {
        DispatchQueue.main.async { configure(view.window) }
    }

    private func configure(_ window: NSWindow?) {
        guard let window else { return }
        window.identifier = WindowID.mainIdentifier
        window.isRestorable = false
        window.appearance = NSAppearance(named: .darkAqua)
        window.minSize = NSSize(width: 900, height: 600)
        window.titleVisibility = .hidden
        window.toolbarStyle = .unified
    }
}

// MARK: - App delegate

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.appearance = NSAppearance(named: .darkAqua)
        for window in NSApp.windows {
            window.appearance = NSAppearance(named: .darkAqua)
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    /// A crash never loses more than about a second of ratings (todo.md §6.3), and a *quit* loses
    /// none: the debounced sidecar queue, the settings and the recents list are written out first.
    func applicationWillTerminate(_ notification: Notification) {
        MainActor.assumeIsolated { AppEnvironment.shared.model.prepareForQuit() }
    }
}
