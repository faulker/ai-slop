import Foundation
import Testing
@testable import BlockerCore

@Suite struct ShouldBlockTests {
    @Test func blocksMusicWhenEnabled() {
        #expect(shouldBlock(bundleID: "com.apple.Music", enabled: true))
        #expect(shouldBlock(bundleID: "com.apple.iTunes", enabled: true))
    }

    @Test func leavesOtherPlayersAlone() {
        #expect(!shouldBlock(bundleID: "com.spotify.client", enabled: true))
        #expect(!shouldBlock(bundleID: "org.videolan.vlc", enabled: true))
        #expect(!shouldBlock(bundleID: "com.apple.podcasts", enabled: true))
    }

    @Test func blocksNothingWhenDisabled() {
        #expect(!shouldBlock(bundleID: "com.apple.Music", enabled: false))
    }

    @Test func ignoresMissingBundleID() {
        #expect(!shouldBlock(bundleID: nil, enabled: true))
    }
}

@Suite @MainActor struct BlockerSettingsTests {
    /// Makes an isolated defaults suite and wipes it after the test body runs.
    private func withDefaults(_ body: (UserDefaults) -> Void) {
        let name = "DisableIMusicTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        body(defaults)
    }

    @Test func defaultsToOnAndFirstLaunch() {
        withDefaults { defaults in
            let settings = BlockerSettings(defaults: defaults)
            #expect(settings.blockingEnabled)
            #expect(settings.showMenuBarIcon)
            #expect(settings.isFirstLaunch)
            #expect(settings.blockedCount == 0)
        }
    }

    @Test func persistsChangesAcrossInstances() {
        withDefaults { defaults in
            let first = BlockerSettings(defaults: defaults)
            first.blockingEnabled = false
            first.showMenuBarIcon = false
            first.blockedCount = 5

            let second = BlockerSettings(defaults: defaults)
            #expect(!second.blockingEnabled)
            #expect(!second.showMenuBarIcon)
            #expect(!second.isFirstLaunch)
            #expect(second.blockedCount == 0)
        }
    }
}
