import AppKit

final class AppDelegate: NSObject, NSApplicationDelegate, StatusBarControllerDelegate {

    private var coordinator: TextMonitorCoordinator?
    private var overlayController: OverlayWindowController?
    private var popupController: CorrectionPopupController?
    private var statusBarController: StatusBarController?
    private var onboardingController: OnboardingWindowController?
    private var settingsController: SettingsWindowController?

    private let logger = Logger(category: "AppDelegate")

    // MARK: - NSApplicationDelegate

    func applicationDidFinishLaunching(_ notification: Notification) {
        if AccessibilityPermissionChecker.isAccessibilityEnabled() {
            setupApp()
        } else {
            showOnboarding()
        }
    }

    // MARK: - Onboarding

    private func showOnboarding() {
        let onboarding = OnboardingWindowController()
        onboarding.onComplete = { [weak self] in
            self?.setupApp()
            self?.onboardingController = nil
        }
        onboarding.showOnboarding()
        onboardingController = onboarding
    }

    // MARK: - App Setup

    private func setupApp() {
        let coordinator = TextMonitorCoordinator()
        let overlay = OverlayWindowController()
        let popup = CorrectionPopupController()
        let statusBar = StatusBarController()

        self.coordinator = coordinator
        self.overlayController = overlay
        self.popupController = popup
        self.statusBarController = statusBar

        statusBar.delegate = self
        popup.anchorView = overlay.popupAnchorView

        // Wire coordinator lint results → overlay underlines
        coordinator.onLintResults = { [weak overlay] items in
            let underlines = items.enumerated().map { index, item -> SquigglyUnderlineView.Underline in
                let color = item.errorType.lowercased() == "spelling"
                    ? Constants.spellingErrorColor
                    : Constants.grammarErrorColor
                return SquigglyUnderlineView.Underline(
                    rect: item.screenRect,
                    color: color,
                    resultIndex: index
                )
            }
            if underlines.isEmpty {
                overlay?.clearUnderlines()
            } else {
                overlay?.updateUnderlines(underlines)
            }
        }

        // Wire overlay underline click → correction popup
        // Capture all needed data NOW (before focus changes can clear results)
        overlay.onUnderlineClicked = { [weak self] index, screenRect in
            guard let self = self,
                  let coordinator = self.coordinator,
                  index < coordinator.currentResults.count else { return }
            let item = coordinator.currentResults[index]
            let range = NSRange(location: item.startOffset, length: item.endOffset - item.startOffset)
            let capturedElement = coordinator.lastLintElement

            // Suppress lint/focus operations while the popup menu is visible
            coordinator.isPopupActive = true

            // Set callbacks with captured data — they won't be invalidated by focus changes
            let originalWord = item.originalWord
            self.popupController?.onCorrectionSelected = { [weak coordinator] replacement, _ in
                coordinator?.isPopupActive = false
                coordinator?.applyCorrection(replacement: replacement, range: range, element: capturedElement, originalWord: originalWord)
            }
            self.popupController?.onAddToDictionary = { [weak coordinator] word in
                coordinator?.isPopupActive = false
                coordinator?.addWordToDictionary(word)
            }
            self.popupController?.onIgnore = { [weak coordinator] word in
                coordinator?.isPopupActive = false
                coordinator?.ignoreWord(word)
            }

            self.popupController?.show(
                near: screenRect,
                originalWord: item.originalWord,
                suggestions: item.suggestions,
                resultIndex: index
            )

            // menu.popUp is synchronous — when we get here, the menu was dismissed.
            // If no action was taken (user clicked away), reset the flag.
            coordinator.isPopupActive = false
        }

        // Wire engine state changes to status bar
        coordinator.onEngineStateChanged = { [weak statusBar] state in
            statusBar?.updateState(state)
        }

        // Start monitoring immediately (independent of engine init)
        coordinator.start()
        overlayController?.showOverlay()

        // Initialize engine in background (fire-and-forget, will trigger performLint when ready)
        coordinator.initializeEngine()

        logger.info("App setup complete — monitoring started, engine initializing")
    }

    // MARK: - StatusBarControllerDelegate

    func statusBarDidRequestDumpAXTree() {
        // Delay slightly so the menu dismisses and the previous app regains focus
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
            let reader = AccessibilityReader()
            reader.dumpFocusedElementTree()
        }
    }

    func statusBarDidRequestSettings() {
        if settingsController == nil {
            settingsController = SettingsWindowController()
        }
        settingsController?.showWindow()
    }

    func statusBarDidToggleEnabled(_ enabled: Bool) {
        if enabled {
            coordinator?.start()
            overlayController?.showOverlay()
        } else {
            coordinator?.stop()
            overlayController?.hideOverlay()
            popupController?.dismiss()
        }
    }

    func statusBarDidRequestQuit() {
        NSApp.terminate(nil)
    }
}
