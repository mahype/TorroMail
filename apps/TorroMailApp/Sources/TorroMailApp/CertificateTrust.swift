import SwiftUI
import TorroMailKit

/// The decision a refused certificate asks for, shown wherever the refusal
/// surfaces — in the setup wizard and on the account. A local Proton Mail
/// Bridge or a home server presents a certificate no authority vouches for,
/// and refusing it outright left those accounts impossible to set up. So the
/// user may trust it, once and for good; the warning is plain about what that
/// means, and the fingerprint is there to compare with what the server shows.
struct UntrustedCertificateNotice: View {
    @Environment(\.textScale) private var textScale
    var certificate: UntrustedCertificate
    /// Nil where the notice only informs and the button lives elsewhere.
    var trust: (() -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 10 * textScale) {
            Label {
                Text(certificateHeadline(certificate))
                    .scaledFont(.headline)
                    .fixedSize(horizontal: false, vertical: true)
            } icon: {
                Image(systemName: "exclamationmark.shield.fill")
                    .foregroundStyle(.orange)
            }

            Text(certificateReason(certificate.reason))
                .scaledFont(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            VStack(alignment: .leading, spacing: 2 * textScale) {
                Text(L("SHA-256 fingerprint"))
                    .scaledFont(.caption)
                    .foregroundStyle(.secondary)
                Text(verbatim: certificate.displayFingerprint)
                    .font(.system(size: 11 * textScale, design: .monospaced))
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Text(L("Only trust it if you know this is your server, such as a Proton Mail Bridge on this Mac. Trusting someone else’s certificate would hand them your mail and your password. TorroMail remembers the decision and asks again only if the certificate changes."))
                .scaledFont(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            if let trust {
                Button(L("Trust This Certificate"), action: trust)
                    .torroButton()
            }
        }
    }
}

func certificateHeadline(_ certificate: UntrustedCertificate) -> String {
    String(format: L("The certificate of %@ is not trusted."), certificate.server)
}

/// Why verification refused it, in words — the server only sends one.
func certificateReason(_ reason: String) -> String {
    switch reason {
    case "unknown_issuer":
        L("No known certificate authority vouches for it — usual for a self-signed certificate, as a Proton Mail Bridge or your own server uses.")
    case "expired":
        L("It has expired.")
    case "not_yet_valid":
        L("It is not valid yet. Check the date and time of this Mac.")
    case "wrong_name":
        L("It was issued for a different server name.")
    default:
        L("It could not be verified.")
    }
}

extension ConnectionFailure {
    /// One sentence per cause. Only a spoken refusal mentions the password.
    var sentence: String {
        switch self {
        case .credentialsRejected:
            L("The server rejected the username or password.")
        case let .untrustedCertificate(certificate):
            certificateHeadline(certificate)
        case .serverUnreachable:
            L("The server did not answer. Check the server and port below, and that it is running.")
        case .encryptionFailed:
            L("The encrypted connection could not be established. Check below whether the server expects SSL/TLS or STARTTLS.")
        case let .other(detail):
            detail.isEmpty
                ? L("The connection failed.")
                : String(format: L("The connection failed: %@"), detail)
        }
    }
}

/// What to say about an account's state: a refused certificate in words, any
/// other failure in the writer's own.
func connectionStateText(_ message: String) -> String {
    if let certificate = UntrustedCertificate(detail: message) {
        return certificateHeadline(certificate)
    }
    return message
}
