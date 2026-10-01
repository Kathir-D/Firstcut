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
private struct SettingsLaunchOpener: View {
  @Environment(\.openSettings) private var openSettings

  var body: some View {
    Color.clear.task {
      guard LaunchOptions.opensSettings else { return }
      try? await Task.sleep(for: .seconds(1))
      do {
        try openSettings()
      } catch {
        // The environment action has refused on some OS builds (it errored silently on macOS 27,
        // leaving the screenshot workflow without a Settings window). The AppKit action the
        // Settings menu item itself sends is the same thing and does not depend on the SwiftUI
        // action's bookkeeping; the older `showPreferencesWindow:` spelling covers macOS 13.
        NSLog("Firstcut: openSettings() failed at launch (\(error)); falling back to AppKit")
        if !NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil) {
          NSApp.sendAction(Selector(("showPreferencesWindow:")), to: nil, from: nil)
        }
      }
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
