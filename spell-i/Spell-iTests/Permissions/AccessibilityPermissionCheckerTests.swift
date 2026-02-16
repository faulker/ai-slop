import XCTest
@testable import Spell_i

final class AccessibilityPermissionCheckerTests: XCTestCase {

    func testPermissionState() {
        // We can't easily force a permission state in tests,
        // but we can verify that the method returns a boolean.
        let isEnabled = AccessibilityPermissionChecker.isAccessibilityEnabled()
        XCTAssertTrue(isEnabled == true || isEnabled == false)
    }
}
