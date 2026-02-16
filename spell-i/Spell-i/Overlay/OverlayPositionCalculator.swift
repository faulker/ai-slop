import AppKit

/// Converts Accessibility API screen coordinates to overlay view coordinates.
/// Handles coordinate system differences (AX uses bottom-left origin, AppKit uses top-left for views).
struct OverlayPositionCalculator {

    /// Converts an AX bounds rect (Quartz/CG coordinates: top-left origin, Y increases downward)
    /// to overlay view coords (also top-left origin via isFlipped, relative to the screen).
    static func viewRect(fromAXRect axRect: CGRect, in screen: NSScreen) -> CGRect {
        // AX and Core Graphics use top-left origin. Our flipped overlay view also uses top-left.
        // We just need to make the rect local to the screen.
        let primaryScreenHeight = NSScreen.screens.first?.frame.height ?? 0

        // Screen's top-left in Quartz coords:
        // NSScreen.frame uses Cocoa (bottom-left origin). Convert the screen's top-left to Quartz.
        let screenTopQuartzY = primaryScreenHeight - screen.frame.origin.y - screen.frame.height

        let localX = axRect.origin.x - screen.frame.origin.x
        let localY = axRect.origin.y - screenTopQuartzY

        return CGRect(x: localX, y: localY, width: axRect.size.width, height: axRect.size.height)
    }

    /// Returns the screen containing the given AX rect.
    /// Falls back to the primary screen if no exact match is found.
    static func screen(for axRect: CGRect) -> NSScreen? {
        let point = CGPoint(x: axRect.midX, y: axRect.midY)

        // Exact match: AX and NSScreen both use bottom-left origin (Quartz/Cocoa)
        if let match = NSScreen.screens.first(where: { $0.frame.contains(point) }) {
            return match
        }

        // Fallback: AX may return coords slightly outside screen frames (rounding, multi-monitor gaps).
        // Use the primary screen rather than returning nil and dropping the underline.
        return NSScreen.main ?? NSScreen.screens.first
    }

    /// Calculates the position for the correction popup relative to a word rect.
    /// wordRect is in top-left view coordinates (Y increases downward).
    /// Prefers below the word, but flips above if near screen bottom.
    static func popupPosition(for wordRect: CGRect, popupSize: CGSize, in screen: NSScreen) -> CGPoint {
        let margin: CGFloat = 4.0
        let belowY = wordRect.maxY + margin

        // Compare in view coords: screen height is the boundary
        if belowY + popupSize.height <= screen.frame.height {
            return CGPoint(x: wordRect.minX, y: belowY)
        } else {
            // Place above
            return CGPoint(x: wordRect.minX, y: wordRect.minY - popupSize.height - margin)
        }
    }
}
