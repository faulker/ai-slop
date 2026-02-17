import XCTest
@testable import Spell_i

final class StatusBarControllerTests: XCTestCase {

    func testInitialStatusText() {
        let controller = StatusBarController()
        XCTAssertEqual(controller.statusMenuItemTitleForTesting, "Engine: Starting...")
    }

    func testUpdateStateReady() {
        let controller = StatusBarController()
        controller.updateState(.ready)
        XCTAssertEqual(controller.statusMenuItemTitleForTesting, "Engine: Ready")
    }

    func testUpdateStateInitializing() {
        let controller = StatusBarController()
        controller.updateState(.initializing)
        XCTAssertEqual(controller.statusMenuItemTitleForTesting, "Engine: Starting...")
    }

    func testUpdateStateDegraded() {
        let controller = StatusBarController()
        controller.updateState(.degraded(retryCount: 1))
        XCTAssertEqual(
            controller.statusMenuItemTitleForTesting,
            "Engine: Degraded — retrying (2/\(Constants.engineRetryDelays.count))..."
        )
    }

    func testUpdateStateFailed() {
        let controller = StatusBarController()
        controller.updateState(.failed)
        XCTAssertEqual(controller.statusMenuItemTitleForTesting, "Engine: Failed — restart app")
    }

    func testMenuContainsSettingsItem() {
        let controller = StatusBarController()
        let settingsItem = controller.menuForTesting?.items.first(where: { $0.title == "Settings..." })
        XCTAssertNotNil(settingsItem, "Menu should contain a Settings... item")
        XCTAssertEqual(settingsItem?.keyEquivalent, ",")
    }

    func testMenuContainsDumpAXTreeItem() {
        let controller = StatusBarController()
        let dumpItem = controller.menuForTesting?.items.first(where: { $0.title == "Dump AX Tree" })
        XCTAssertNotNil(dumpItem, "Menu should contain a Dump AX Tree item")
    }

    func testDelegateNotifiedOnToggle() {
        let controller = StatusBarController()
        let delegate = MockStatusBarDelegate()
        controller.delegate = delegate

        guard let toggleItem = controller.menuForTesting?.items.first(where: { $0.title == "Enabled" }) else {
            XCTFail("Could not find Enabled menu item")
            return
        }
        _ = toggleItem.target?.perform(toggleItem.action, with: toggleItem)

        XCTAssertTrue(delegate.didToggle, "Delegate should be notified of toggle")
        XCTAssertFalse(delegate.lastEnabledState, "First toggle should disable (was enabled)")
    }
}

// MARK: - Mock

private class MockStatusBarDelegate: StatusBarControllerDelegate {
    var didToggle = false
    var lastEnabledState = true
    var didRequestSettings = false
    var didRequestDumpAXTree = false
    var didRequestQuit = false

    func statusBarDidToggleEnabled(_ enabled: Bool) {
        didToggle = true
        lastEnabledState = enabled
    }

    func statusBarDidRequestSettings() {
        didRequestSettings = true
    }

    func statusBarDidRequestDumpAXTree() {
        didRequestDumpAXTree = true
    }

    func statusBarDidRequestQuit() {
        didRequestQuit = true
    }
}
