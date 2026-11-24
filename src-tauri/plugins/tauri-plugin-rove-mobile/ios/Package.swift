// swift-tools-version:5.9

import PackageDescription

let package = Package(
    name: "tauri-plugin-rove-mobile",
    platforms: [.iOS(.v14), .macOS(.v10_15)],
    products: [
        .library(
            name: "tauri-plugin-rove-mobile",
            type: .static,
            targets: ["tauri-plugin-rove-mobile"]
        ),
    ],
    dependencies: [
        .package(name: "Tauri", path: "../.tauri/tauri-api"),
    ],
    targets: [
        .target(
            name: "tauri-plugin-rove-mobile",
            dependencies: [.byName(name: "Tauri")],
            path: "Sources"
        ),
    ]
)
