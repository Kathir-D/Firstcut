// Owner: ui.

import AppKit
import SwiftUI

enum Appearance {
    static let filmstripHeight: CGFloat = 104
    static let filmstripInset: CGFloat = 10
    static let filmstripGap: CGFloat = 6
    static let filmstripPlateInset: CGFloat = 5
    static let filmstripCornerRadius: CGFloat = 6
    static let filmstripPlateCornerRadius: CGFloat = 10
    static let infoPanelWidth: CGFloat = 288
    static let hudMaxWidth: CGFloat = 520

    static let glassCornerRadius: CGFloat = 10
    static let capsuleCornerRadius: CGFloat = 18

    static var windowBackground: Color { Color(nsColor: .windowBackgroundColor) }
    static var barBackground: Color { Color(nsColor: .controlBackgroundColor) }
    static var separator: Color { Color(nsColor: .separatorColor) }
    static var primaryLabel: Color { Color(nsColor: .labelColor) }
    static var secondaryLabel: Color { Color(nsColor: .secondaryLabelColor) }
    static var tertiaryLabel: Color { Color(nsColor: .tertiaryLabelColor) }

    static let keepGreen = Color(nsColor: .systemGreen)
    static let rejectRed = Color(nsColor: .systemRed)
    static let plateFill = Color(nsColor: .quaternaryLabelColor).opacity(0.55)

    static func viewerBackground(darkness: Double) -> Color {
        let level = 0.04 + 0.16 * min(max(darkness, 0), 1)
        return Color(white: level)
    }
}

enum RatingColor {
    static func color(for label: ColorLabel) -> Color {
        switch label {
        case .red: Color(nsColor: .systemRed)
        case .yellow: Color(nsColor: .systemYellow)
        case .green: Color(nsColor: .systemGreen)
        case .blue: Color(nsColor: .systemBlue)
        case .purple: Color(nsColor: .systemPurple)
        }
    }
}
