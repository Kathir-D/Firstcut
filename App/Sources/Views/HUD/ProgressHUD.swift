// Owner: ui.
//
// Progress HUD (todo.md §9.6): a glass capsule with the current photo's rating (§6.1), batch X of
// Y, photos left, tier counts and elapsed time. H shows and hides it.

import SwiftUI

struct ProgressHUD: View {
  let progress: CullProgress
  var photo: CullPhoto?
  var mode: RatingMode = .stars

  var body: some View {
    GlassCapsule {
      HStack(spacing: 14) {
        if let photo {
          CurrentRating(rating: photo.rating, mode: mode)
          divider
        }
        metric("\(progress.batchIndex + 1)", "of \(max(progress.batchCount, 1)) batches")
        divider
        metric("\(progress.photosRemaining)", "unrated left")
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
      (photo.map { "\(GridCell.accessibilityText(for: $0, mode: mode)). " } ?? "")
        + "Progress: batch \(progress.batchIndex + 1) of \(progress.batchCount), \(progress.photosRemaining) unrated, \(progress.keeps) keeps"
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

/// The photo on screen, as it is rated right now: five stars (or Keep / Not keep), its flag and its
/// colour label, so a rating key visibly lands without looking down at the filmstrip.
private struct CurrentRating: View {
  let rating: Rating
  let mode: RatingMode

  var body: some View {
    HStack(spacing: 8) {
      switch mode {
      case .stars:
        let stars = RatingVisuals.starCount(for: rating, mode: mode)
        HStack(spacing: 1) {
          ForEach(1...5, id: \.self) { index in
            Image(systemName: index <= stars ? "star.fill" : "star")
              .foregroundStyle(index <= stars ? Color.yellow : Appearance.tertiaryLabel)
          }
        }
        .font(.system(size: 10))
      case .keep:
        let keep = RatingVisuals.isKeep(rating, mode: mode)
        Label(keep ? "Keep" : "Not keep", systemImage: keep ? "checkmark.circle.fill" : "circle")
          .font(.system(size: 11, weight: .semibold))
          .foregroundStyle(keep ? Appearance.keepGreen : Appearance.secondaryLabel)
      }
      switch rating.flag {
      case .pick:
        Image(systemName: "flag.fill").font(.system(size: 10)).foregroundStyle(Appearance.primaryLabel)
      case .reject:
        Image(systemName: "xmark.circle.fill").font(.system(size: 11)).foregroundStyle(Appearance.rejectRed)
      case .none:
        EmptyView()
      }
      if let label = rating.label {
        Circle().fill(RatingColor.color(for: label)).frame(width: 8, height: 8)
      }
    }
    .fixedSize()
  }
}
