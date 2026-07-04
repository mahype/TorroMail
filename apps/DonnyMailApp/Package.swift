// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "DonnyMailApp",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .executable(name: "DonnyMailApp", targets: ["DonnyMailApp"]),
        .executable(name: "DonnyMailKitContract", targets: ["DonnyMailKitContract"])
    ],
    targets: [
        .target(name: "DonnyMailKit"),
        .executableTarget(
            name: "DonnyMailApp",
            dependencies: ["DonnyMailKit"]
        ),
        .executableTarget(
            name: "DonnyMailKitContract",
            dependencies: ["DonnyMailKit"],
            path: "Tests/DonnyMailKitContract"
        )
    ]
)
