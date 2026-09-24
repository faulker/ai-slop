// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "DisableIMusic",
    platforms: [.macOS(.v14)],
    targets: [
        .target(name: "BlockerCore"),
        .executableTarget(name: "DisableIMusic", dependencies: ["BlockerCore"]),
        .testTarget(name: "BlockerCoreTests", dependencies: ["BlockerCore"]),
    ]
)
