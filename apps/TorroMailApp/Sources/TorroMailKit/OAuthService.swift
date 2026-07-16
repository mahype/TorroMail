import AuthenticationServices
import CryptoKit
import Foundation

/// The half of OAuth that needs a browser and a person.
///
/// Renewing a token happens in the MCP server, not here — it has to work when
/// the app is closed. Getting the *first* token cannot: it needs the user to
/// see the provider's own page and say yes. That is this file.
///
/// Follows RFC 8252: the system browser via `ASWebAuthenticationSession`, PKCE
/// instead of a client secret, and a custom scheme to come back on.
public enum OAuthService {
    public struct Identity: Sendable {
        /// What the provider says the address is — which beats what the user
        /// typed, and is what the account gets saved as.
        public let email: String
        public let name: String?
    }

    public struct Result: Sendable {
        public let tokens: OAuthTokens
        public let identity: Identity?
    }

    public enum Failure: LocalizedError {
        case notConfigured(OAuthIssuer)
        case cancelled
        case providerRefused(String)
        case malformedResponse(String)

        public var errorDescription: String? {
            switch self {
            case .notConfigured(let issuer):
                "This build has no \(issuer.displayName) client registered."
            case .cancelled:
                "The sign-in window was closed."
            case .providerRefused(let detail):
                detail
            case .malformedResponse(let detail):
                "The provider's answer could not be read: \(detail)"
            }
        }
    }

    /// Runs the whole dance: browser, code, token exchange, identity.
    @MainActor
    public static func signIn(
        issuer: OAuthIssuer,
        loginHint: String?,
        presentationAnchor: ASPresentationAnchor?
    ) async throws -> Result {
        guard ProviderCatalog.isConfigured(issuer) else {
            throw Failure.notConfigured(issuer)
        }

        let verifier = PKCE.makeVerifier()
        let code = try await authorize(
            issuer: issuer,
            challenge: PKCE.challenge(for: verifier),
            loginHint: loginHint,
            anchor: presentationAnchor
        )
        let tokens = try await exchange(issuer: issuer, code: code, verifier: verifier)
        // Identity is a nicety: a token that works is still a success even if
        // the provider will not say who it belongs to.
        let identity = try? await fetchIdentity(issuer: issuer, accessToken: tokens.accessToken)
        return Result(tokens: tokens, identity: identity)
    }

    // MARK: - Steps

    @MainActor
    private static func authorize(
        issuer: OAuthIssuer,
        challenge: String,
        loginHint: String?,
        anchor: ASPresentationAnchor?
    ) async throws -> String {
        var components = URLComponents(url: issuer.authorizationEndpoint, resolvingAgainstBaseURL: false)
        var query = [
            URLQueryItem(name: "client_id", value: issuer.clientID),
            URLQueryItem(name: "redirect_uri", value: issuer.redirectURI),
            URLQueryItem(name: "response_type", value: "code"),
            URLQueryItem(name: "scope", value: issuer.scopes.joined(separator: " ")),
            URLQueryItem(name: "code_challenge", value: challenge),
            URLQueryItem(name: "code_challenge_method", value: "S256")
        ]
        // Without this Google hands back an access token and no refresh token
        // on the second and later sign-ins, and the account dies an hour after
        // setup with no way to renew.
        if issuer == .google {
            query.append(URLQueryItem(name: "access_type", value: "offline"))
            query.append(URLQueryItem(name: "prompt", value: "consent"))
        }
        if let loginHint, !loginHint.isEmpty {
            query.append(URLQueryItem(name: "login_hint", value: loginHint))
        }
        components?.queryItems = query

        guard let url = components?.url else {
            throw Failure.malformedResponse("the authorization URL could not be built")
        }

        let callback = try await present(url: url, scheme: issuer.redirectScheme, anchor: anchor)
        return try authorizationCode(from: callback)
    }

    @MainActor
    private static func present(
        url: URL,
        scheme: String,
        anchor: ASPresentationAnchor?
    ) async throws -> URL {
        let presenter = AnchorProvider(anchor: anchor)
        return try await withCheckedThrowingContinuation { continuation in
            let session = ASWebAuthenticationSession(
                url: url,
                callbackURLScheme: scheme
            ) { callback, error in
                // Capturing `presenter` strongly is deliberate: nothing else
                // holds it, and `presentationContextProvider` below is a weak
                // reference. Without this the provider dies the moment this
                // function returns and the sheet has no window to appear in.
                defer { presenter.finish() }

                if let error {
                    let cancelled = (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin
                    continuation.resume(throwing: cancelled ? Failure.cancelled : error)
                    return
                }
                guard let callback else {
                    continuation.resume(throwing: Failure.malformedResponse("no callback URL"))
                    return
                }
                continuation.resume(returning: callback)
            }
            session.presentationContextProvider = presenter
            // A fresh session every time: reusing the browser's cookies would
            // silently sign in whoever is already logged in, which is wrong
            // for someone adding their second account.
            session.prefersEphemeralWebBrowserSession = true
            presenter.keepAlive(session)
            session.start()
        }
    }

    private static func authorizationCode(from callback: URL) throws -> String {
        let items = URLComponents(url: callback, resolvingAgainstBaseURL: false)?.queryItems ?? []
        if let error = items.first(where: { $0.name == "error" })?.value {
            let description = items.first { $0.name == "error_description" }?.value
            throw Failure.providerRefused(description ?? error)
        }
        guard let code = items.first(where: { $0.name == "code" })?.value else {
            throw Failure.malformedResponse("no authorization code came back")
        }
        return code
    }

    private static func exchange(
        issuer: OAuthIssuer,
        code: String,
        verifier: String
    ) async throws -> OAuthTokens {
        let form = [
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": issuer.redirectURI,
            "client_id": issuer.clientID,
            "code_verifier": verifier
        ]
        let json = try await postForm(issuer.tokenEndpoint, fields: form)

        guard let accessToken = json["access_token"] as? String else {
            let detail = (json["error_description"] as? String)
                ?? (json["error"] as? String)
                ?? "no access token"
            throw Failure.providerRefused(detail)
        }
        guard let refreshToken = json["refresh_token"] as? String else {
            // Without one the account works for an hour and then cannot be
            // renewed — better to fail here, loudly, than to hand the user an
            // account that quietly dies over lunch.
            throw Failure.providerRefused(
                "the provider issued no refresh token, so access could not be kept"
            )
        }
        let lifetime = (json["expires_in"] as? Double) ?? 3600

        return OAuthTokens(
            accessToken: accessToken,
            refreshToken: refreshToken,
            expiresAt: Date().addingTimeInterval(lifetime)
        )
    }

    private static func fetchIdentity(issuer: OAuthIssuer, accessToken: String) async throws -> Identity {
        var request = URLRequest(url: issuer.userInfoEndpoint)
        request.setValue("Bearer \(accessToken)", forHTTPHeaderField: "Authorization")
        request.timeoutInterval = 10

        let (data, _) = try await URLSession.shared.data(for: request)
        let json = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]

        // Microsoft's Graph calls them something else than Google's OIDC
        // endpoint does; both are trying to say the same two things.
        let email = (json["email"] as? String)
            ?? (json["mail"] as? String)
            ?? (json["userPrincipalName"] as? String)
        guard let email else {
            throw Failure.malformedResponse("no address in the profile")
        }
        let name = (json["name"] as? String) ?? (json["displayName"] as? String)
        return Identity(email: email, name: name)
    }

    private static func postForm(_ url: URL, fields: [String: String]) async throws -> [String: Any] {
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.timeoutInterval = 20
        request.httpBody = Data(formEncoded(fields).utf8)

        let (data, _) = try await URLSession.shared.data(for: request)
        guard let json = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            throw Failure.malformedResponse("the token endpoint did not answer with JSON")
        }
        return json
    }

    /// `application/x-www-form-urlencoded`: not the same escaping as a URL
    /// query. `+` is a space here, so it — and every other sub-delimiter —
    /// has to be escaped rather than passed through.
    private static func formEncoded(_ fields: [String: String]) -> String {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~")
        return fields
            .map { key, value in
                let escapedKey = key.addingPercentEncoding(withAllowedCharacters: allowed) ?? key
                let escapedValue = value.addingPercentEncoding(withAllowedCharacters: allowed) ?? value
                return "\(escapedKey)=\(escapedValue)"
            }
            .joined(separator: "&")
    }
}

/// What the app stores for an OAuth account. Serialized into the keychain in
/// the shape `torromail-oauth` reads on the Rust side — the two must agree,
/// and `AccountTrial`/`--check-account` is what proves they do.
public struct OAuthTokens: Sendable, Equatable {
    public let accessToken: String
    public let refreshToken: String
    public let expiresAt: Date

    public init(accessToken: String, refreshToken: String, expiresAt: Date) {
        self.accessToken = accessToken
        self.refreshToken = refreshToken
        self.expiresAt = expiresAt
    }

    /// The keychain blob. `expires_at` is absolute unix seconds because the
    /// reader has no idea when this was written.
    public func keychainPayload() throws -> String {
        let object: [String: Any] = [
            "type": "oauth2",
            "access_token": accessToken,
            "refresh_token": refreshToken,
            "expires_at": Int(expiresAt.timeIntervalSince1970)
        ]
        let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        guard let text = String(data: data, encoding: .utf8) else {
            throw OAuthService.Failure.malformedResponse("the token set could not be encoded")
        }
        return text
    }
}

/// Proof-Key for Code Exchange (RFC 7636). What stands in for a client secret
/// on a public client: the app proves the code it redeems is the one it asked
/// for, without a shared secret to leak.
enum PKCE {
    static func makeVerifier() -> String {
        var bytes = [UInt8](repeating: 0, count: 32)
        // The verifier is the entire protection; a predictable one would let
        // an intercepted code be redeemed by whoever intercepted it.
        _ = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
        return base64URL(Data(bytes))
    }

    static func challenge(for verifier: String) -> String {
        base64URL(Data(SHA256.hash(data: Data(verifier.utf8))))
    }

    /// base64url, unpadded — RFC 7636 §4.2 spells this out because plain
    /// base64 breaks in a URL.
    private static func base64URL(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}

/// Tells `ASWebAuthenticationSession` which window to hang the sheet on, and
/// keeps the session alive while it runs.
///
/// The lifetimes here are a knot: the session's completion handler holds this
/// object, and this object holds the session — a cycle, and on purpose. It is
/// what keeps both alive while the user is off in the browser, since nothing
/// outside does. `finish()` cuts it once the handler has run.
private final class AnchorProvider: NSObject, ASWebAuthenticationPresentationContextProviding {
    private let anchor: ASPresentationAnchor?
    private var session: ASWebAuthenticationSession?

    init(anchor: ASPresentationAnchor?) {
        self.anchor = anchor
    }

    func keepAlive(_ session: ASWebAuthenticationSession) {
        self.session = session
    }

    func finish() {
        session = nil
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        anchor ?? ASPresentationAnchor()
    }
}
