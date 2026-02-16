import AppKit

if CommandLine.arguments.contains("--validate-ffi") {
    FFIValidator.validate()
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = AppDelegate()
app.delegate = delegate
app.run()
