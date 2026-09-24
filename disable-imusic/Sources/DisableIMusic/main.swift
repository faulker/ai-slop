import AppKit

// Accessory policy: no Dock icon. LSUIElement in Info.plist does the same for the bundled app.
let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
