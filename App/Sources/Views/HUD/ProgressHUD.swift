// Owner: ui.
//
// Progress HUD (todo.md §9.6): a glass capsule with the current photo's rating (§6.1), batch X of
// Y, photos left, tier counts and elapsed time. H shows and hides it.

import SwiftUI

struct ProgressHUD: View {
    let progress: CullProgress
    var photo: CullPhoto?
    var mode: RatingMode = .stars
    /// Sends `.setKeep` / `.setNotKeep`. Nil in a read-only context, which is why the control has
    /// a no-action form rather than the HUD being the only place it appears.
    var onSetKeep: (() -> Void)?
    var onSetNotKeep: (() -> Void)?

    var body: some View {
        GlassCapsule(isInteractive: false) {
            // `isInteractive: false` so the capsule *background* does not swallow clicks meant for
            // the viewer behind it; the Keep / Not keep buttons below still take their own, because
            // hit testing is per-view, not inherited from the background.
            HStack(spacing: 14) {
                if let photo {
                    // Keep mode gets the two-button control, which is the control for that mode;
                    // stars mode keeps the stars, which are the whole rating there.
                    if mode == .keep {
                        KeepRatingControl(
                            isKeep: RatingVisuals.isKeep(photo.rating, mode: mode),
                            onSetKeep: onSetKeep, onSetNotKeep: onSetNotKeep, compact: true)
                    } else {
                        CurrentRating(rating: photo.rating)
                    }
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
/// The stars-mode readout: the five stars, plus the pick flag, reject flag and colour label.
///
/// **Stars mode only.** Keep mode is `KeepRatingControl`, which has its own two buttons, so this used
/// to carry a `.keep` branch that rendered a single "Keep" / "Not keep" label. Nothing could reach
/// it — the HUD routes keep mode to the two-button control above — and keeping it meant two
/// different renderings of the same answer, one of which nobody ever saw. A reviewer looking for
/// how a keep is drawn would have found this one first, so it went.
private struct CurrentRating: View {
    let rating: Rating

    var body: some View {
        HStack(spacing: 8) {
            let stars = rating.stars
            HStack(spacing: 1) {
                ForEach(1...5, id: \.self) { index in
                    Image(systemName: index <= stars ? "star.fill" : "star")
                        .foregroundStyle(index <= stars ? Color.yellow : Appearance.tertiaryLabel)
                }
            }
            .font(.system(size: 10))
            switch rating.flag {
            case .pick:
                Image(systemName: "flag.fill").font(.system(size: 10)).foregroundStyle(
                    Appearance.primaryLabel)
            case .reject:
                Image(systemName: "xmark.circle.fill").font(.system(size: 11)).foregroundStyle(
                    Appearance.rejectRed)
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
