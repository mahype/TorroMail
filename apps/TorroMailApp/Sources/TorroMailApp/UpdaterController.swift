import Foundation
import Sparkle

/// Owns Sparkle for the lifetime of TorroMail and exposes only the two update
/// decisions the control surface needs: whether to check automatically, and
/// whether to check now.
///
/// SwiftPM command-line runs and locally assembled bundles without a release
/// key stay inert. That keeps development launches from contacting the release
/// feed or showing an updater error for a deliberately unsigned build.
@MainActor
final class UpdaterController: ObservableObject {
    private let controller: SPUStandardUpdaterController?

    @Published var automaticallyChecksForUpdates: Bool {
        didSet {
            guard let updater = controller?.updater,
                  updater.automaticallyChecksForUpdates != automaticallyChecksForUpdates
            else { return }
            updater.automaticallyChecksForUpdates = automaticallyChecksForUpdates
        }
    }

    init(bundle: Bundle = .main) {
        if Self.canStart(in: bundle) {
            controller = SPUStandardUpdaterController(
                startingUpdater: true,
                updaterDelegate: nil,
                userDriverDelegate: nil
            )
        } else {
            controller = nil
        }
        automaticallyChecksForUpdates =
            controller?.updater.automaticallyChecksForUpdates ?? false
    }

    var isAvailable: Bool { controller != nil }

    var lastUpdateCheckDate: Date? { controller?.updater.lastUpdateCheckDate }

    func checkForUpdates() {
        controller?.checkForUpdates(nil)
    }

    private static func canStart(in bundle: Bundle) -> Bool {
        guard bundle.bundleIdentifier != nil,
              bundle.bundlePath.hasSuffix(".app"),
              bundle.object(forInfoDictionaryKey: "SUFeedURL") != nil,
              bundle.object(forInfoDictionaryKey: "TorroMailDisableUpdates") as? Bool != true,
              let publicKey = bundle.object(forInfoDictionaryKey: "SUPublicEDKey") as? String,
              !publicKey.isEmpty,
              !publicKey.hasPrefix("__")
        else { return false }
        return true
    }
}
