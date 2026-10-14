// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "DiskUsageViewer",
    platforms: [.macOS(.v14)],
    targets: [
        .target(name: "DiskCore"),
        .executableTarget(name: "DiskUsageViewer", dependencies: ["DiskCore"]),
        .testTarget(name: "DiskCoreTests", dependencies: ["DiskCore"]),
    ]
)
