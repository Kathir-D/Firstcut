// Owner: ui.
//
// Progress HUD (task.md §9.6): a glass capsule with batch X of Y, photos left, tier counts and
// elapsed time. Auto-hides unless the user turns it on with H.

import SwiftUI

struct ProgressHUD: View {
  let progress: CullProgress

  var body: some View {
    GlassCapsule {
      HStack(spacing: 14) {
        metric("\(progress.batchNumber)", "of \(max(progress.batchCount, 1)) batches")
        divider
        metric("\(progress.unratedPhotos)", "unrated left")
        divider
        metric("\(progress.keeps)", "keep")
        metric("\(progress.good)", "good")
        metric("\(progress.maybe)", "maybe")
        divider
        metric(PhotoFormatter.elapsed(progress.elapsed), "elapsed")
      }
      .padding(.horizontal, 16)
      .padding(.vertical, 9)
    }
    .frame(maxWidth: Appearance.hudMaxWidth)
    .accessibilityElement(children: .combine)
    .accessibilityLabel(
      "Progress: batch \(progress.batchNumber) of \(progress.batchCount), \(progress.unratedPhotos) unrated, \(progress.keeps) keeps"
    )
  }

  private var divider: some View {
    Rectangle()
      .fill(Appearance.separator)
      .frame(width: 1, height: 18)
  }

  private func metric(_ value: String, _ label: String) -> some View {
    VStack(alignment: .leading, spacing: 1) {
      Text(value)
        .font(.system(size: 12, weight: .semibold))
        .foregroundStyle(Appearance.primaryLabel)
        .monospacedDigit()
      Text(label)
        .font(.system(size: 9))
        .foregroundStyle(Appearance.secondaryLabel)
    }
    .fixedSize()
  }
}
