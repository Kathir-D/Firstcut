// Owner: ui.
//
// The keep-mode rating control: **two buttons**, Keep and Not keep (todo.md §6.2).
//
// ## Why two buttons and not the toggle that used to be the whole story
//
// `toggleKeep` is still the keyboard binding and still the fastest thing a photographer does — one
// chord, P, flips the decision. But a *button* cannot honestly say "toggle": pressing a Keep button
// that then means "not keep" is a control that lies about itself, and the label is the only thing
// the user reads. So the on-screen control sends `setKeep` / `setNotKeep`, which are idempotent, and
// P keeps toggling. Both spellings exist in `Command`, so a custom keymap can bind either.
//
// ## Colour is the state, and it is the *only* state
//
// The current decision is shown by which button is lit, not by a separate label: the lit button is
// this photo's answer, and the other one is the way to change it. That is also why the buttons keep
// their own shapes and symbols — a green fill alone is not the whole signal, so shape, symbol and
// fill all agree. The star count is deliberately absent: in keep mode the stars are the *other*
// mode's field, and printing one here would suggest it is what decides the tier. It does not; a keep
// is `keep: true` and one star is what it is written as (§6.2).

import SwiftUI

/// The keep-mode control. Sized from the HUD's own scale so it sits in the same capsule without
/// changing its height.
struct KeepRatingControl: View {
    let isKeep: Bool
    /// Sends `.setKeep` / `.setNotKeep`. Nil in a read-only context (the HUD, previews).
    var onSetKeep: (() -> Void)?
    var onSetNotKeep: (() -> Void)?
    var compact = false

    var body: some View {
        HStack(spacing: compact ? 6 : 8) {
            button(
                title: "Keep",
                systemImage: isKeep ? "checkmark.circle.fill" : "circle",
                tint: Appearance.keepGreen,
                active: isKeep,
                action: onSetKeep
            )
            button(
                title: "Not keep",
                systemImage: !isKeep ? "xmark.circle.fill" : "circle",
                tint: Appearance.rejectRed,
                active: !isKeep,
                action: onSetNotKeep
            )
        }
        .fixedSize()
        // One element for VoiceOver rather than two, because the answer is one bit: what matters is
        // "kept" or "not kept", and reading both buttons out loud says the same thing twice.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(isKeep ? "Kept" : "Not kept")
        .accessibilityHint("Press K to keep, or X to mark not kept")
    }

    private func button(
        title: String, systemImage: String, tint: Color, active: Bool, action: (() -> Void)?
    ) -> some View {
        let height: CGFloat = compact ? 18 : 20
        // Half the height, so the pill's ends are real semicircles and the curve runs the full
        // height — the same continuous shape as the HUD capsule these buttons sit inside, rather
        // than a 5pt box that stopped short of the control it was drawn around (todo.md §0.5).
        let radius = Appearance.pillCornerRadius(height: height)
        let shape = Capsule(style: .continuous)
        let label =
            HStack(spacing: 4) {
                Image(systemName: systemImage)
                Text(title)
                    .fixedSize()  // never truncate "Not keep" into "Not k"
            }
            .font(.system(size: compact ? 10 : 11, weight: .semibold))
            .foregroundStyle(active ? tint : Appearance.secondaryLabel)
            .padding(.horizontal, compact ? 8 : 10)
            // A fixed height, so the lit button cannot make the control jump when the symbol changes
            // between the two SF Symbols, which have different vertical extents.
            .frame(height: height)
            .background(shape.fill(active ? tint.opacity(0.22) : Color.clear))
            .overlay(shape.strokeBorder(active ? tint : Color.clear, lineWidth: 1))
            .contentShape(shape)  // the whole pill is clickable, not just the glyphs

        guard let action else { return AnyView(label) }
        return AnyView(Button(action: action) { label }.buttonStyle(.plain))
    }
}
