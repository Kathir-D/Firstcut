// Owner: ui.
//
// Rating visuals for the filmstrip (todo.md §6.1, §6.2): stars + flag badge + color label in stars
// mode, green/red keep rings in keep mode. One CALayer per frame keeps the whole strip on the GPU.

import AppKit
import QuartzCore

enum RatingVisuals {
    /// Keep, for the mode the user is working in. This used to be `rating.keep || rating.stars >= 4`
    /// with the threshold hard-coded and the mode ignored — a fourth copy of the rating-mode rule
    /// (REV-31, REV-69). It disagreed with `RatingRules` for a photo rated 3 stars in keep mode, which
    /// is a "Good" in stars mode and correctly *not* a keep. Read the one implementation instead.
    static func isKeep(_ rating: Rating, mode: RatingMode) -> Bool {
        RatingRules.isKeep(rating, mode: mode)
    }

    static func showsStars(_ rating: Rating, mode: RatingMode = .stars) -> Bool {
        starCount(for: rating, mode: mode) > 0
    }

    /// REV-76: the number of stars drawn for a rating. Exactly `stars`, never padded to five, so a
    /// 3-star photo shows three stars and reads as 3 rather than as 3-out-of-4.
    ///
    /// In stars mode a keep shows the 5 stars it means (todo.md §6) — the display half of the mapping,
    /// via `RatingRules.displayStars`. Reading `rating.stars` directly showed a keep as Unrated.
    static func starCount(for rating: Rating, mode: RatingMode = .stars) -> Int {
        min(max(Int(RatingRules.displayStars(rating, mode: mode)), 0), 5)
    }

    static func showsFlag(_ rating: Rating) -> Bool {
        rating.flag != .none
    }

    static func keepRingColor(_ rating: Rating, mode: RatingMode) -> NSColor {
        isKeep(rating, mode: mode) ? .systemGreen : .systemRed
    }
}

final class FilmstripBadgeLayer: CALayer {
    var rating = Rating() {
        didSet { if oldValue != rating { setNeedsDisplay() } }
    }
    var mode: RatingMode = .stars {
        didSet { if oldValue != mode { setNeedsDisplay() } }
    }

    override init() {
        super.init()
    }

    override init(layer: Any) {
        super.init(layer: layer)
    }

    required init?(coder aDecoder: NSCoder) {
        super.init(coder: aDecoder)
    }

    override func draw(in context: CGContext) {
        let size = bounds.size
        guard size.width > 12, size.height > 12 else { return }
        if let label = rating.label {
            drawColorLabel(in: context, size: size, label: label)
        }
        switch mode {
        case .stars:
            if RatingVisuals.showsStars(rating, mode: mode) { drawStars(in: context, size: size) }
            if RatingVisuals.showsFlag(rating) { drawFlag(in: context, size: size) }
        case .keep:
            drawKeepRing(in: context, bounds: bounds)
        }
    }

    // MARK: - Stars mode

    /// REV-76: exactly `stars` filled stars, no empty outlines. Drawing five slots with the tail
    /// outlined reads as "3 out of 4", which is the one reading that is wrong; Finder's gallery view
    /// shows filled stars only. Rating 0 draws nothing at all.
    private func drawStars(in context: CGContext, size: CGSize) {
        let count = RatingVisuals.starCount(for: rating, mode: mode)
        guard count > 0 else { return }

        let inset: CGFloat = 4
        let starSize = max(5, min(9, size.height * 0.11))
        let gap = starSize * 0.3
        let originY = size.height - inset - starSize

        for index in 0..<count {
            let rect = CGRect(
                x: inset + CGFloat(index) * (starSize + gap),
                y: originY,
                width: starSize,
                height: starSize
            )
            let path = Self.starPath(in: rect)
            context.addPath(path)
            context.setFillColor(NSColor.systemYellow.cgColor)
            context.fillPath()
            context.addPath(path)
            context.setStrokeColor(NSColor.black.withAlphaComponent(0.4).cgColor)
            context.setLineWidth(0.75)
            context.strokePath()
        }
    }

    private func drawFlag(in context: CGContext, size: CGSize) {
        let side: CGFloat = max(10, min(16, size.height * 0.2))
        let rect = CGRect(x: size.width - side - 4, y: 4, width: side, height: side)
        let radius = rect.width * 0.3
        let color: NSColor = rating.flag == .pick ? .systemGreen : .systemRed

        context.setFillColor(NSColor.black.withAlphaComponent(0.5).cgColor)
        context.fill(roundedRect: rect.insetBy(dx: -1.5, dy: -1.5), radius: radius + 1.5)
        context.setFillColor(color.cgColor)
        context.fill(roundedRect: rect, radius: radius)

        context.setStrokeColor(NSColor.white.cgColor)
        context.setLineWidth(max(1, side * 0.1))
        context.setLineCap(.round)
        if rating.flag == .pick {
            let path = CGMutablePath()
            path.move(to: CGPoint(x: rect.minX + rect.width * 0.28, y: rect.midY + rect.height * 0.02))
            path.addLine(to: CGPoint(x: rect.minX + rect.width * 0.44, y: rect.minY + rect.height * 0.24))
            path.addLine(to: CGPoint(x: rect.minX + rect.width * 0.74, y: rect.maxY - rect.height * 0.24))
            context.addPath(path)
        } else {
            context.move(to: CGPoint(x: rect.minX + rect.width * 0.3, y: rect.minY + rect.height * 0.3))
            context.addLine(
                to: CGPoint(x: rect.maxX - rect.width * 0.3, y: rect.maxY - rect.height * 0.3))
            context.move(to: CGPoint(x: rect.maxX - rect.width * 0.3, y: rect.minY + rect.height * 0.3))
            context.addLine(
                to: CGPoint(x: rect.minX + rect.width * 0.3, y: rect.maxY - rect.height * 0.3))
        }
        context.strokePath()
    }

    // MARK: - Keep mode

    private func drawKeepRing(in context: CGContext, bounds: CGRect) {
        let lineWidth: CGFloat = 2
        let rect = bounds.insetBy(dx: lineWidth / 2 + 1, dy: lineWidth / 2 + 1)
        context.setStrokeColor(RatingVisuals.keepRingColor(rating, mode: mode).cgColor)
        context.setLineWidth(lineWidth)
        context.addPath(
            CGPath(
                roundedRect: rect,
                cornerWidth: Appearance.filmstripCornerRadius,
                cornerHeight: Appearance.filmstripCornerRadius,
                transform: nil
            ))
        context.strokePath()
    }

    // MARK: - Color label

    private func drawColorLabel(in context: CGContext, size: CGSize, label: ColorLabel) {
        let side: CGFloat = max(5, min(8, size.height * 0.1))
        let rect = CGRect(x: 4, y: size.height - side - 4, width: side, height: side)
        context.setFillColor(NSColor(RatingColor.color(for: label)).cgColor)
        context.fillEllipse(in: rect)
        context.setStrokeColor(NSColor.black.withAlphaComponent(0.5).cgColor)
        context.setLineWidth(0.75)
        context.strokeEllipse(in: rect)
    }

    // MARK: - Geometry

    static func starPath(in rect: CGRect) -> CGPath {
        let center = CGPoint(x: rect.midX, y: rect.midY)
        let outer = rect.width / 2
        let inner = outer * 0.42
        let path = CGMutablePath()
        for index in 0..<10 {
            let radius = index.isMultiple(of: 2) ? outer : inner
            let angle = -CGFloat.pi / 2 + CGFloat(index) * CGFloat.pi / 5
            let point = CGPoint(x: center.x + cos(angle) * radius, y: center.y + sin(angle) * radius)
            if index == 0 { path.move(to: point) } else { path.addLine(to: point) }
        }
        path.closeSubpath()
        return path
    }
}

extension CGContext {
    fileprivate func fill(roundedRect rect: CGRect, radius: CGFloat) {
        addPath(CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil))
        fillPath()
    }
}
