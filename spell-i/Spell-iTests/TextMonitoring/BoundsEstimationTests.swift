import XCTest
@testable import Spell_i

final class BoundsEstimationTests: XCTestCase {

    // MARK: - Helpers

    /// Mirror of the production NSFont measurement logic so tests compute expected values
    /// the same way the implementation does.
    private static let referenceFont = NSFont.systemFont(ofSize: 12)
    private static let referenceFontAttrs: [NSAttributedString.Key: Any] = [.font: referenceFont]
    private static let lineHeightToFontSizeRatio: CGFloat = 0.75
    private static let effectiveWidthMargin: CGFloat = 1.15

    /// Computes the expected rect using the same NSFont logic as production code.
    private func expectedRect(word: String, childText: String, childBounds: CGRect) -> CGRect? {
        guard let wordRange = childText.range(of: word) else { return nil }
        guard !childText.isEmpty else { return nil }

        let fullTextWidth = (childText as NSString).size(withAttributes: Self.referenceFontAttrs).width
        guard fullTextWidth > 0 else { return nil }

        let prefix = String(childText[childText.startIndex..<wordRange.lowerBound])
        let prefixWidth = (prefix as NSString).size(withAttributes: Self.referenceFontAttrs).width
        let wordMeasuredWidth = (word as NSString).size(withAttributes: Self.referenceFontAttrs).width

        let estimatedFontSize = childBounds.height * Self.lineHeightToFontSizeRatio
        let scaledTextWidth = fullTextWidth * (estimatedFontSize / 12.0)
        let effectiveWidth = min(childBounds.width, scaledTextWidth * Self.effectiveWidthMargin)

        let wordX = childBounds.origin.x + (prefixWidth / fullTextWidth) * effectiveWidth
        let wordW = (wordMeasuredWidth / fullTextWidth) * effectiveWidth

        return CGRect(x: wordX, y: childBounds.origin.y, width: wordW, height: childBounds.height)
    }

    // MARK: - proportionalRect: Basic positioning

    func testProportionalRectWordAtStart() {
        let childBounds = CGRect(x: 100, y: 200, width: 300, height: 20)
        let childText = "Hello World testing"
        let result = AccessibilityReader.proportionalRect(
            word: "Hello",
            childText: childText,
            childBounds: childBounds
        )
        let expected = expectedRect(word: "Hello", childText: childText, childBounds: childBounds)

        XCTAssertNotNil(result)
        XCTAssertNotNil(expected)
        guard let rect = result, let exp = expected else { return }

        XCTAssertEqual(rect.origin.x, exp.origin.x, accuracy: 0.01, "X should match expected")
        XCTAssertEqual(rect.origin.y, 200, accuracy: 0.01, "Y should match child")
        XCTAssertEqual(rect.height, 20, accuracy: 0.01, "Height should match child")
        XCTAssertEqual(rect.width, exp.width, accuracy: 0.01, "Width should match expected")
        // Word at start: X should be at child origin
        XCTAssertEqual(rect.origin.x, 100, accuracy: 0.01, "First word X should be at child start")
    }

    func testProportionalRectWordInMiddle() {
        let childBounds = CGRect(x: 50, y: 100, width: 200, height: 16)
        let childText = "Hello World"
        let result = AccessibilityReader.proportionalRect(
            word: "World",
            childText: childText,
            childBounds: childBounds
        )
        let expected = expectedRect(word: "World", childText: childText, childBounds: childBounds)

        XCTAssertNotNil(result)
        XCTAssertNotNil(expected)
        guard let rect = result, let exp = expected else { return }

        XCTAssertEqual(rect.origin.x, exp.origin.x, accuracy: 0.01, "X should match expected offset")
        XCTAssertEqual(rect.width, exp.width, accuracy: 0.01, "Width should match expected")
    }

    func testProportionalRectWordAtEnd() {
        let childBounds = CGRect(x: 0, y: 0, width: 100, height: 14)
        let childText = "the end"
        let result = AccessibilityReader.proportionalRect(
            word: "end",
            childText: childText,
            childBounds: childBounds
        )
        let expected = expectedRect(word: "end", childText: childText, childBounds: childBounds)

        XCTAssertNotNil(result)
        XCTAssertNotNil(expected)
        guard let rect = result, let exp = expected else { return }

        XCTAssertEqual(rect.origin.x, exp.origin.x, accuracy: 0.01)
        XCTAssertEqual(rect.width, exp.width, accuracy: 0.01)
    }

    func testProportionalRectSingleWord() {
        let childBounds = CGRect(x: 10, y: 20, width: 80, height: 18)
        let result = AccessibilityReader.proportionalRect(
            word: "test",
            childText: "test",
            childBounds: childBounds
        )

        XCTAssertNotNil(result)
        guard let rect = result else { return }

        // Single word: X at child origin, width = effectiveWidth (may be capped)
        XCTAssertEqual(rect.origin.x, 10, accuracy: 0.01)
        XCTAssertGreaterThan(rect.width, 0, "Single word should have positive width")
    }

    func testProportionalRectWordNotFound() {
        let childBounds = CGRect(x: 0, y: 0, width: 200, height: 20)
        let result = AccessibilityReader.proportionalRect(
            word: "missing",
            childText: "Hello World",
            childBounds: childBounds
        )

        XCTAssertNil(result, "Should return nil when word is not in child text")
    }

    func testProportionalRectEmptyChildText() {
        let childBounds = CGRect(x: 0, y: 0, width: 200, height: 20)
        let result = AccessibilityReader.proportionalRect(
            word: "test",
            childText: "",
            childBounds: childBounds
        )

        XCTAssertNil(result, "Should return nil for empty child text")
    }

    func testProportionalRectPreservesYAndHeight() {
        let childBounds = CGRect(x: 50, y: 300, width: 400, height: 22)
        let result = AccessibilityReader.proportionalRect(
            word: "error",
            childText: "This has an error in it",
            childBounds: childBounds
        )

        XCTAssertNotNil(result)
        guard let rect = result else { return }

        XCTAssertEqual(rect.origin.y, 300, accuracy: 0.01, "Y should be preserved from child")
        XCTAssertEqual(rect.height, 22, accuracy: 0.01, "Height should be preserved from child")
    }

    func testProportionalRectFirstOccurrence() {
        let childBounds = CGRect(x: 0, y: 0, width: 270, height: 20)
        let childText = "the cat and the dog"
        let result = AccessibilityReader.proportionalRect(
            word: "the",
            childText: childText,
            childBounds: childBounds
        )

        XCTAssertNotNil(result)
        guard let rect = result else { return }

        // "the" first appears at offset 0
        XCTAssertEqual(rect.origin.x, 0, accuracy: 0.01, "Should match first occurrence at start")
    }

    // MARK: - NSFont proportional width properties

    func testNarrowCharsProduceNarrowerRect() {
        // "iii" should be narrower than "mmm" due to proportional widths
        let childBounds = CGRect(x: 0, y: 0, width: 500, height: 20)

        let resultNarrow = AccessibilityReader.proportionalRect(
            word: "iii",
            childText: "iii xxx",
            childBounds: childBounds
        )
        let resultWide = AccessibilityReader.proportionalRect(
            word: "mmm",
            childText: "mmm xxx",
            childBounds: childBounds
        )

        XCTAssertNotNil(resultNarrow)
        XCTAssertNotNil(resultWide)
        guard let rNarrow = resultNarrow, let rWide = resultWide else { return }

        XCTAssertLessThan(rNarrow.width, rWide.width,
                          "Narrow chars (iii) should produce a narrower rect than wide chars (mmm)")
    }

    func testWidthCappedWhenElementMuchWiderThanText() {
        // Simulate Chrome scenario: element is 800px wide but text is short
        let childBounds = CGRect(x: 0, y: 0, width: 800, height: 16)
        let childText = "short"

        let result = AccessibilityReader.proportionalRect(
            word: "short",
            childText: childText,
            childBounds: childBounds
        )

        XCTAssertNotNil(result)
        guard let rect = result else { return }

        // The effective width should be capped well below 800
        XCTAssertLessThan(rect.width, 800,
                          "Width should be capped when element is much wider than text")
        // But still positive
        XCTAssertGreaterThan(rect.width, 0)
    }

    func testWidthNotCappedWhenElementFitsTightly() {
        // When element width is smaller than scaled text width, no capping occurs
        // Use a long text in a tight element
        let childText = "This is a longer sentence with many words in it"
        let fullTextWidth = (childText as NSString).size(withAttributes: Self.referenceFontAttrs).width
        let estimatedFontSize: CGFloat = 20 * Self.lineHeightToFontSizeRatio
        let scaledTextWidth = fullTextWidth * (estimatedFontSize / 12.0)
        // Set element width smaller than what the margin-scaled text would be
        let tightWidth = scaledTextWidth * 0.8
        let childBounds = CGRect(x: 0, y: 0, width: tightWidth, height: 20)

        let result = AccessibilityReader.proportionalRect(
            word: "sentence",
            childText: childText,
            childBounds: childBounds
        )
        let expected = expectedRect(word: "sentence", childText: childText, childBounds: childBounds)

        XCTAssertNotNil(result)
        XCTAssertNotNil(expected)
        guard let rect = result, let exp = expected else { return }

        // When element is tight, effectiveWidth == childBounds.width (no capping)
        XCTAssertEqual(rect.width, exp.width, accuracy: 0.01)
        XCTAssertEqual(rect.origin.x, exp.origin.x, accuracy: 0.01)
    }

    func testLaterWordsNotPushedFarRightInWideElement() {
        // The original Chrome/Google Keep bug: later words pushed too far right
        // With a very wide element (800px) and short text, the last word should NOT
        // be near the right edge of the element
        let childBounds = CGRect(x: 0, y: 0, width: 800, height: 16)
        let childText = "This is a test sentence"

        let result = AccessibilityReader.proportionalRect(
            word: "sentence",
            childText: childText,
            childBounds: childBounds
        )

        XCTAssertNotNil(result)
        guard let rect = result else { return }

        // "sentence" should end well before 800px (the element edge)
        let wordEnd = rect.origin.x + rect.width
        XCTAssertLessThan(wordEnd, 400,
                          "Last word should not be pushed to the far right of a wide element")
    }

    // MARK: - Contiguity: words tile properly

    func testWordsAreContiguous() {
        let childBounds = CGRect(x: 0, y: 0, width: 300, height: 20)
        let childText = "abc def"

        let resultAbc = AccessibilityReader.proportionalRect(
            word: "abc",
            childText: childText,
            childBounds: childBounds
        )
        let resultDef = AccessibilityReader.proportionalRect(
            word: "def",
            childText: childText,
            childBounds: childBounds
        )

        XCTAssertNotNil(resultAbc)
        XCTAssertNotNil(resultDef)
        guard let rAbc = resultAbc, let rDef = resultDef else { return }

        // "def" starts after "abc " (4 chars including space)
        // The gap between abc's end and def's start should equal approximately one space width
        let gap = rDef.origin.x - (rAbc.origin.x + rAbc.width)
        let spaceWidth = (" " as NSString).size(withAttributes: Self.referenceFontAttrs).width
        let fullTextWidth = (childText as NSString).size(withAttributes: Self.referenceFontAttrs).width
        let estimatedFontSize = childBounds.height * Self.lineHeightToFontSizeRatio
        let scaledTextWidth = fullTextWidth * (estimatedFontSize / 12.0)
        let effectiveWidth = min(childBounds.width, scaledTextWidth * Self.effectiveWidthMargin)
        let expectedSpaceGap = (spaceWidth / fullTextWidth) * effectiveWidth

        XCTAssertEqual(gap, expectedSpaceGap, accuracy: 0.5,
                       "Gap between adjacent words should equal one space width")
    }
}
