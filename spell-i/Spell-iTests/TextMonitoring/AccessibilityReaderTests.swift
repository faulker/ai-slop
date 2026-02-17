import XCTest
@testable import Spell_i

final class AccessibilityReaderTests: XCTestCase {

    // MARK: - Text Edit Roles

    func testTextEditRolesContainsExpectedRoles() {
        let expected: Set<String> = [
            "AXTextArea", "AXTextField", "AXComboBox", "AXSearchField", "AXStaticText"
        ]
        for role in expected {
            XCTAssertTrue(Constants.textEditRoles.contains(role), "textEditRoles should contain \(role)")
        }
    }

    func testTextEditRolesDoesNotContainNonTextRoles() {
        let nonTextRoles = ["AXButton", "AXImage", "AXToolbar", "AXMenu", "AXCheckBox"]
        for role in nonTextRoles {
            XCTAssertFalse(Constants.textEditRoles.contains(role), "textEditRoles should not contain \(role)")
        }
    }

    // MARK: - Container Roles

    func testContainerRolesContainsExpectedRoles() {
        let expected: Set<String> = [
            "AXWebArea", "AXGroup", "AXScrollArea", "AXList", "AXCell",
            "AXSection", "AXLayoutArea", "AXSplitGroup", "AXTabGroup",
            "AXRow", "AXOutline", "AXTable"
        ]
        for role in expected {
            XCTAssertTrue(Constants.containerRolesForTraversal.contains(role),
                          "containerRolesForTraversal should contain \(role)")
        }
    }

    func testContainerRolesDoesNotContainLeafRoles() {
        let leafRoles = ["AXTextField", "AXButton", "AXImage", "AXStaticText"]
        for role in leafRoles {
            XCTAssertFalse(Constants.containerRolesForTraversal.contains(role),
                           "containerRolesForTraversal should not contain \(role)")
        }
    }

    // MARK: - Traversal Depth

    func testMaxTraversalDepthIsAtLeast12() {
        XCTAssertGreaterThanOrEqual(Constants.maxTraversalDepth, 12,
                                    "maxTraversalDepth should be at least 12 for deep Chromium hierarchies")
    }

    // MARK: - Role Set Disjointness

    func testTextEditRolesAndContainerRolesAreDisjoint() {
        let overlap = Constants.textEditRoles.intersection(Constants.containerRolesForTraversal)
        XCTAssertTrue(overlap.isEmpty,
                      "textEditRoles and containerRolesForTraversal should not overlap, found: \(overlap)")
    }
}
