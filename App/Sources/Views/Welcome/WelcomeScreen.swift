// Owner: ui.
//
// Welcome state (task.md §9.6). It is a whole-window screen, not an overlay: `RootView` switches on
// `phase` so it can never share the window with the culling chrome (REV-75).
//
// Recent sessions with progress and the Dock drop target land in wave 3; the drop target is already
// wired on the window.

import SwiftUI

struct WelcomeScreen: View {
  let state: any CullViewState

  var body: some View {
    ZStack {
      Appearance.viewerBackground(darkness: state.viewerBackgroundDarkness)

      VStack(spacing: 0) {
        Spacer(minLength: 24)

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
          Button("Open Folder…") { state.send(.openFolder) }
            .controlSize(.large)
            .buttonStyle(.borderedProminent)
            .keyboardShortcut("o", modifiers: .command)
          Text("You can also drag a folder onto this window.")
            .font(.system(size: 11))
            .foregroundStyle(Appearance.tertiaryLabel)
        }
        .padding(32)
        .accessibilityElement(children: .contain)

        Spacer(minLength: 24)
      }
    }
    .accessibilityLabel("Welcome")
  }
}

struct LoadingScreen: View {
  let progress: Double

  var body: some View {
    ZStack {
      Appearance.viewerBackground(darkness: 0.13)
      VStack(spacing: 12) {
        // A determinate bar only when there is a fraction to show. `Session::open` scans every
        // header in the folder and reports nothing while it does, so an honest 0% would sit on
        // screen for seconds looking broken; a bar frozen at zero is worse than a spinner.
        if progress > 0 {
          ProgressView(value: progress)
            .progressViewStyle(.linear)
            .frame(width: 260)
          Text("Reading \(Int(progress * 100))%")
            .font(.system(size: 12))
            .foregroundStyle(Appearance.secondaryLabel)
        } else {
          ProgressView()
            .progressViewStyle(.circular)
            .controlSize(.large)
          Text("Reading the folder…")
            .font(.system(size: 12))
            .foregroundStyle(Appearance.secondaryLabel)
        }
      }
    }
    .accessibilityElement(children: .combine)
    .accessibilityLabel(progress > 0 ? "Loading \(Int(progress * 100)) percent" : "Reading the folder")
    .accessibilityValue(Text(progress, format: .percent.precision(.fractionLength(0))))
  }
}
