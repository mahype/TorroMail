// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "TorroMailApp",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .executable(name: "TorroMailApp", targets: ["TorroMailApp"]),
        .executable(name: "TorroMailKitContract", targets: ["TorroMailKitContract"])
    ],
    targets: [
        .target(name: "TorroMailKit"),
        .executableTarget(
            name: "TorroMailApp",
            dependencies: ["TorroMailKit"]
        ),
        .executableTarget(
            name: "TorroMailKitContract",
            dependencies: ["TorroMailKit"],
            path: "Tests/TorroMailKitContract"
        )
    ]
)
