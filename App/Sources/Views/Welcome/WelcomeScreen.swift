// Owner: ui.
//
// Welcome state (todo.md §9.6). It is a whole-window screen, not an overlay: `RootView` switches on
// `phase` so it can never share the window with the culling chrome (REV-75).
//
// Recent folders show how far through each shoot the user got, and reopening one resumes it: the
// session database restores the batches, ratings and the photo they were on. Dragging a folder onto
// the window is wired in `FirstcutApp`.

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

        if !state.recentFolders.isEmpty {
          RecentFoldersList(state: state)
            .frame(maxWidth: 460)
            .padding(.horizontal, 32)
        }

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

/// "Recent" on the Welcome screen: name, where it is, and how much of it has been rated.
private struct RecentFoldersList: View {
  let state: any CullViewState

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      Text("Recent")
        .font(.system(size: 11, weight: .semibold))
        .foregroundStyle(Appearance.secondaryLabel)
        .padding(.horizontal, 10)
      ForEach(state.recentFolders.prefix(6)) { folder in
        Button {
          state.openRecent(folder)
        } label: {
          HStack(spacing: 10) {
            Image(systemName: "folder")
              .foregroundStyle(Appearance.secondaryLabel)
            VStack(alignment: .leading, spacing: 2) {
              Text(folder.name)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(Appearance.primaryLabel)
                .lineLimit(1)
              Text(abbreviated(folder.path))
                .font(.system(size: 10))
                .foregroundStyle(Appearance.tertiaryLabel)
                .lineLimit(1)
                .truncationMode(.head)
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 3) {
              Text("\(folder.ratedPhotos) of \(folder.totalPhotos) rated")
                .font(.system(size: 10))
                .foregroundStyle(Appearance.secondaryLabel)
              ProgressView(value: folder.fractionRated)
                .progressViewStyle(.linear)
                .frame(width: 84)
            }
          }
          .padding(.horizontal, 10)
          .padding(.vertical, 7)
          .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background(Appearance.plateFill.opacity(0.5), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .contextMenu {
          Button("Remove from Recent") { state.forgetRecent(folder) }
        }
        .accessibilityLabel(
          "\(folder.name), \(folder.ratedPhotos) of \(folder.totalPhotos) photos rated")
        .accessibilityHint("Opens this folder and resumes where you left off")
      }
    }
  }

  private func abbreviated(_ path: String) -> String {
    (path as NSString).abbreviatingWithTildeInPath
  }
}
