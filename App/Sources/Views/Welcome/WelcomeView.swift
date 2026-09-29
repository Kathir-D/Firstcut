// Owner: ui.
//
// Welcome state (task.md §9.6). Recent sessions with progress, drag and drop and the Dock drop
// target land in wave 3; this is the wave 1 shape so the app is usable before then.

import SwiftUI

struct WelcomeView: View {
  var onOpenFolder: () -> Void = {}

  var body: some View {
    VStack(spacing: 14) {
      Image(systemName: "photo.on.rectangle.angled")
        .font(.system(size: 44, weight: .light))
        .foregroundStyle(Appearance.secondaryLabel)
      Text("Firstcut")
        .font(.system(size: 22, weight: .semibold))
        .foregroundStyle(Appearance.primaryLabel)
      Text("Open a folder of photos to start culling.")
        .font(.system(size: 12))
        .foregroundStyle(Appearance.secondaryLabel)
      Button("Open Folder…", action: onOpenFolder)
        .controlSize(.large)
        .buttonStyle(.borderedProminent)
        .keyboardShortcut("o", modifiers: .command)
    }
    .padding(32)
    .accessibilityElement(children: .contain)
  }
}
