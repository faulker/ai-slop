import AppKit

/// Custom NSView that draws squiggly (wavy) underlines at specified screen positions.
final class SquigglyUnderlineView: NSView {

    struct Underline: Equatable {
        /// Screen-space rectangle of the word (already flipped to top-left origin).
        let rect: CGRect
        /// Color of the underline (red for spelling, blue for grammar).
        let color: NSColor
        /// Index into the coordinator's currentResults array.
        let resultIndex: Int

        static func == (lhs: Underline, rhs: Underline) -> Bool {
            lhs.rect == rhs.rect && lhs.color == rhs.color && lhs.resultIndex == rhs.resultIndex
        }
    }

    /// Current underlines to draw.
    var underlines: [Underline] = []

    override var isFlipped: Bool { true }

    // MARK: - Drawing

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)

        for underline in underlines {
            drawSquiggly(in: underline.rect, color: underline.color)
        }
    }

    /// Draws a squiggly (sine-wave) underline beneath the given rect.
    private func drawSquiggly(in rect: CGRect, color: NSColor) {
        let path = NSBezierPath()
        let amplitude = Constants.squigglyAmplitude
        let wavelength = Constants.squigglyWavelength
        let baselineOffset: CGFloat = 2.0
        let y = rect.maxY + baselineOffset

        path.move(to: NSPoint(x: rect.minX, y: y))

        var x = rect.minX
        let halfWave = wavelength / 2.0

        while x < rect.maxX {
            let nextX = min(x + halfWave, rect.maxX)
            let isPeak = Int((x - rect.minX) / halfWave) % 2 == 0
            let controlY = y + (isPeak ? amplitude : -amplitude)

            // Use different control points for a smooth sine-like curve instead of sawtooth
            path.curve(to: NSPoint(x: nextX, y: y),
                       controlPoint1: NSPoint(x: x + halfWave * 0.25, y: controlY),
                       controlPoint2: NSPoint(x: x + halfWave * 0.75, y: controlY))
            x = nextX
        }

        color.setStroke()
        path.lineWidth = Constants.squigglyStrokeWidth
        path.lineJoinStyle = .round
        path.lineCapStyle = .round
        path.stroke()
    }

    // MARK: - Hit Testing

    /// Returns the underline at the given point, or nil.
    func underline(at point: NSPoint) -> Underline? {
        return underlines.first { underline in
            // Add vertical padding for easier clicking
            underline.rect.insetBy(dx: 0, dy: -Constants.hitTestPadding).contains(point)
        }
    }
}
