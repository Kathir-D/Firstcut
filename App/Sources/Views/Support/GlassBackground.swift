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
    } else if #available(macOS 26.0, *) {
      content()
        .glassEffect(
          isInteractive ? .regular : .regular.tint(.clear),
          in: shape
        )
    } else {
      content()
        .background(.ultraThinMaterial, in: shape)
        .clipShape(shape)
        .overlay(shape.strokeBorder(Color.white.opacity(0.08), lineWidth: 0.5))
    }
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
