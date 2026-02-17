import XCTest
@testable import Spell_i

final class SettingsTests: XCTestCase {

    override func tearDown() {
        // Clean up any test-written defaults
        UserDefaults.standard.removeObject(forKey: "maxTraversalDepth")
        super.tearDown()
    }

    func testDefaultMaxTraversalDepth() {
        UserDefaults.standard.removeObject(forKey: "maxTraversalDepth")
        XCTAssertEqual(Settings.shared.maxTraversalDepth, Constants.maxTraversalDepth)
    }

    func testSetMaxTraversalDepth() {
        Settings.shared.maxTraversalDepth = 20
        XCTAssertEqual(Settings.shared.maxTraversalDepth, 20)
    }

    func testMaxTraversalDepthClampedToMinimum() {
        Settings.shared.maxTraversalDepth = 1
        XCTAssertEqual(Settings.shared.maxTraversalDepth, 4)
    }

    func testMaxTraversalDepthClampedToMaximum() {
        Settings.shared.maxTraversalDepth = 100
        XCTAssertEqual(Settings.shared.maxTraversalDepth, 30)
    }
}
