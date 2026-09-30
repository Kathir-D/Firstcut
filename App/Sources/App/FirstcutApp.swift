// Owner: ui.
//
// Window, toolbar and menu wiring. The window is a SwiftUI `Window` scene so the toolbar picks up
// the system Liquid Glass on macOS 26 and the material fallback on 15 for free; `AppDelegate`
// forces the dark appearance app-wide (task.md §2).

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

    // ⌘, opens this (task.md §9.8); SwiftUI adds the "Settings…" item to the app menu itself.
    Settings {
      SettingsView(model: environment.model)
    }
  }
}

enum WindowID {
  static let main = "main"
}

private struct WindowContent: View {
  @Environment(AppEnvironment.self) private var environment

  var body: some View {
    RootView(state: environment.state)
      .onDrop(of: [.fileURL], isTargeted: nil) { providers in
        environment.handleDrop(providers)
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

  /// A crash never loses more than about a second of ratings (task.md §6.3), and a *quit* loses
  /// none: the debounced sidecar queue, the settings and the recents list are written out first.
  func applicationWillTerminate(_ notification: Notification) {
    MainActor.assumeIsolated { AppEnvironment.shared.model.prepareForQuit() }
  }
}
