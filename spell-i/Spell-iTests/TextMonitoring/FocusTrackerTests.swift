import XCTest
@testable import Spell_i

final class FocusTrackerTests: XCTestCase {

    func testInitialBundleIDIsNil() {
        let tracker = FocusTracker()
        // Before startTracking, currentBundleID should be nil
        XCTAssertNil(tracker.currentBundleID)
    }

    func testHandleAppActivationUpdatesBundleID() {
        let tracker = FocusTracker()
        let delegate = MockFocusTrackerDelegate()
        tracker.delegate = delegate

        // Simulate an app activation with a known bundle ID
        let mockApp = NSRunningApplication.current
        let notification = Notification(
            name: NSWorkspace.didActivateApplicationNotification,
            object: nil,
            userInfo: [NSWorkspace.applicationUserInfoKey: mockApp]
        )

        // Set a different initial ID so the change is detected
        #if DEBUG
        tracker.setCurrentBundleIDForTesting("com.some.other.app")
        #endif

        tracker.handleAppActivation(notification)

        // Should have updated to the mock app's bundle ID
        XCTAssertEqual(tracker.currentBundleID, mockApp.bundleIdentifier)
        XCTAssertTrue(delegate.didChangeApp, "Delegate should be notified of app change")
    }

    func testSameAppActivationDoesNotNotify() {
        let tracker = FocusTracker()
        let delegate = MockFocusTrackerDelegate()
        tracker.delegate = delegate

        let mockApp = NSRunningApplication.current
        #if DEBUG
        tracker.setCurrentBundleIDForTesting(mockApp.bundleIdentifier)
        #endif

        let notification = Notification(
            name: NSWorkspace.didActivateApplicationNotification,
            object: nil,
            userInfo: [NSWorkspace.applicationUserInfoKey: mockApp]
        )

        tracker.handleAppActivation(notification)

        XCTAssertFalse(delegate.didChangeApp, "Should not notify when same app re-activates")
    }

    func testStopTrackingCleanup() {
        let tracker = FocusTracker()
        tracker.startTracking()
        tracker.stopTracking()
        // Should not crash — verifies teardown is clean
    }
}

// MARK: - Mock

private class MockFocusTrackerDelegate: FocusTrackerDelegate {
    var didChangeApp = false
    var didChangeElement = false
    var didDetectWindowMove = false

    func focusTrackerDidChangeApp() {
        didChangeApp = true
    }

    func focusTrackerDidChangeElement() {
        didChangeElement = true
    }

    func focusTrackerDidDetectWindowMove() {
        didDetectWindowMove = true
    }
}
