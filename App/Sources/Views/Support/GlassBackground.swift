// Owner: ui.
//
// Liquid Glass on macOS 26+, NSVisualEffectView material on macOS 15. Reduce Transparency swaps
// both for a plain fill, which is also what a non-glass fallback looks like.

import SwiftUI

struct GlassBackground<Content: View>: View {
    var cornerRadius: CGFloat = Appearance.glassCornerRadius
    var isInteractive = true
    @ViewBuilder var content: () -> Content

    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        if reduceTransparency {
            content()
                .background(Appearance.barBackground, in: shape)
        } else {
            glassOrMaterial(shape)
        }
    }

    /// Liquid Glass where the toolchain and the system have it, the closest material elsewhere.
    ///
    /// `glassEffect` is in the macOS 26 SDK (Xcode 26, Swift 6.2). CI keeps Xcode 16 as the
    /// minimum-toolchain check (todo.md §2), which has no such API, so the call is compiled only where
    /// it exists. The `#if` wraps whole statements, never half of an `if`/`else` chain, which the
    /// newer compiler rejects.
    @ViewBuilder
    private func glassOrMaterial(_ shape: RoundedRectangle) -> some View {
        #if compiler(>=6.2)
            if #available(macOS 26.0, *) {
                content()
                    .glassEffect(
                        isInteractive ? .regular : .regular.tint(.clear),
                        in: shape
                    )
            } else {
                material(shape)
            }
        #else
            material(shape)
        #endif
    }

    private func material(_ shape: RoundedRectangle) -> some View {
        content()
            .background(.ultraThinMaterial, in: shape)
            .clipShape(shape)
            .overlay(shape.strokeBorder(Color.white.opacity(0.08), lineWidth: 0.5))
    }
}

struct GlassCapsule<Content: View>: View {
    var isInteractive = true
    @ViewBuilder var content: () -> Content

    var body: some View {
        GlassBackground(
            cornerRadius: Appearance.capsuleCornerRadius, isInteractive: isInteractive, content: content)
    }
}
