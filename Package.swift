// swift-tools-version: 6.2
import Foundation
import PackageDescription

let release: (url: String, checksum: String)? = (url: "https://github.com/meritt/unseen.rs/releases/download/v0.1.0/unseen-v0.1.0-xcframework.zip", checksum: "c548264ca8fec1c2c63614b29dc0d2a5a1f7b49564b71f50a1e90abaf62394d3")

let local = "packages/swift/UnseenFFI.xcframework"
let ffi: Target
if FileManager.default.fileExists(atPath: Context.packageDirectory + "/" + local) {
    ffi = .binaryTarget(name: "UnseenFFI", path: local)
} else if let release {
    ffi = .binaryTarget(name: "UnseenFFI", url: release.url, checksum: release.checksum)
} else {
    fatalError("no released UnseenFFI yet: build \(local) with `cargo xtask xcframework`")
}

let package = Package(
    name: "Unseen",
    platforms: [.macOS(.v26)],
    products: [
        .library(name: "Unseen", targets: ["Unseen"]),
    ],
    targets: [
        ffi,
        .target(name: "Unseen", dependencies: ["UnseenFFI"], path: "packages/swift/Sources/Unseen"),
        .testTarget(name: "UnseenTests", dependencies: ["Unseen"], path: "packages/swift/Tests/UnseenTests"),
    ]
)
