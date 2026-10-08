// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "TorroMailApp",
    defaultLocalization: "en",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .executable(name: "TorroMailApp", targets: ["TorroMailApp"]),
        .executable(name: "TorroMailHermesControl", targets: ["TorroMailHermesControl"]),
        .executable(name: "TorroMailKitContract", targets: ["TorroMailKitContract"])
    ],
    dependencies: [
        .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.9.2"),
        .package(url: "https://github.com/jpsim/Yams", from: "6.0.0")
    ],
    targets: [
        .target(name: "TorroMailKit", dependencies: [.product(name: "Yams", package: "Yams")]),
        .executableTarget(name: "TorroMailHermesControl", dependencies: ["TorroMailKit"]),
        .executableTarget(
            name: "TorroMailApp",
            dependencies: [
                "TorroMailKit",
                .product(name: "Sparkle", package: "Sparkle")
            ],
            resources: [
                .process("Resources")
            ]
        ),
        .executableTarget(
            name: "TorroMailKitContract",
            dependencies: ["TorroMailKit", .product(name: "Yams", package: "Yams")],
            path: "Tests/TorroMailKitContract"
        )
    ]
)
