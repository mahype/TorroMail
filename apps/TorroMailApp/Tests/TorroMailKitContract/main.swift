import TorroMailKit
import Foundation

func require(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() {
        FileHandle.standardError.write(Data("Contract failed: \(message)\n".utf8))
        Foundation.exit(1)
    }
}

let model = TorroMailModel.preview()

// Decisions, not options: the sidebar carries exactly four destinations —
// Overview, Mail Accounts, Settings, Log. Individual accounts live one level
// deeper, as cards inside the accounts destination.
require(
    model.accounts.map(\.name) == ["Work", "Personal"],
    "accounts are the primary navigation"
)
require(
    model.selectedSidebarItem == .dashboard,
    "the app lands on the dashboard"
)
require(
    model.accountPath.isEmpty,
    "the account list starts without a drill-down"
)
require(
    model.pendingActionCount == 1,
    "the accounts item badges every waiting approval"
)

// The dashboard leads with one word about the whole app. An account that was
// never tested is not a problem — only rejected credentials are.
require(
    model.health == .ready,
    "untested credentials must not raise a dashboard alarm"
)
require(
    TorroMailModel(
        accounts: [],
        selectedSidebarItem: .dashboard,
        generalSettings: GeneralSettings(),
        audit: []
    ).health == .notConfigured,
    "with no accounts the dashboard explains the app instead of reporting status"
)
require(
    model.pendingActions.map(\.id) == ["pending-1"],
    "the dashboard collects approvals across every account"
)
require(
    model.accountName(id: "work") == "Work",
    "approvals can name the account they belong to"
)
require(
    model.connectedClients == ["Claude Desktop"],
    "the dashboard can say who is connected"
)

// Status only on exception: healthy accounts stay quiet, unverified ones ask
// for attention.
require(
    !model.accounts[0].connectionState.needsAttention,
    "a connected account must not demand attention"
)
require(
    model.accounts[1].connectionState.needsAttention,
    "an untested connection must ask for attention"
)
require(
    !model.accounts[1].connectionState.isBroken,
    "untested credentials are not broken credentials"
)
require(
    ConnectionState.failed("auth rejected").isBroken,
    "rejected credentials show the red dot"
)
require(
    model.accounts[0].pendingActions.count == 1,
    "pending approvals belong to their account"
)

// The one lifecycle decision: start at login. Executable and transport are
// internal details and never user-facing.
require(
    model.generalSettings.launchAtLogin,
    "launch at login is the default"
)
require(
    model.generalSettings.mcpExecutable == "torromail-mcp",
    "supervisor knows its executable without exposing it in the UI"
)

// TorroMail works in the background, but it stays reachable: the Dock tile is
// the default way back to the window. The menu bar stays opt-in.
require(
    model.generalSettings.showDockIcon,
    "the Dock icon is on by default"
)
require(
    !model.generalSettings.showMenuBarIcon,
    "the menu bar item is off by default"
)
require(
    AppPresence.resolve(showDockIcon: false, hasOpenWindow: true) == .foreground,
    "an open window is reachable in the Dock even with the Dock icon off"
)
require(
    AppPresence.resolve(showDockIcon: false, hasOpenWindow: false) == .background,
    "closing the window drops TorroMail out of the Dock"
)
require(
    AppPresence.resolve(showDockIcon: true, hasOpenWindow: false) == .foreground,
    "with the Dock icon on TorroMail stays in the Dock without a window"
)

// The product boundary is part of the contract: the app is a control
// surface, never a mail client.
require(
    TorroMailProductBoundary.disallowedUserMailSurfaces == [
        "Inbox UI",
        "Message reader",
        "Thread browser",
        "Manual triage workflow"
    ],
    "product boundary should reject mail-client surfaces"
)
require(
    TorroMailProductBoundary.allowedAppRole == "Setup, consent, MCP lifecycle, status, audit, and diagnostics",
    "app role should be control-surface only"
)

// The appearance contract: the control surface follows the system in both
// modes, spaces on an 8-point base, and stays on semantic colors outside
// declared brand moments.
require(TorroMailAppearance.spacingUnit == 8, "spacing should use an 8-point base")
require(TorroMailAppearance.supportsLightMode, "light mode should be supported")
require(TorroMailAppearance.supportsDarkMode, "dark mode should be supported")
require(TorroMailAppearance.usesSemanticColors, "theme should use semantic colors")

// Account lifecycle stays in the GUI: add opens the new account, remove
// drops back to the account list. The wizard hands over an account it has
// already proven — an unconfigured shell never reaches this list, because
// anything in it is published to assistants.
model.addAccount(
    MailAccount(
        id: "club",
        name: "Club",
        email: "club@example.org",
        provider: .imapSmtp,
        loginMethod: .password,
        imapHost: "imap.example.org",
        username: "club@example.org",
        connectionState: .connected
    )
)
require(model.accounts.count == 3, "wizard adds an account")
let newID = model.accounts[2].id
require(
    model.selectedSidebarItem == .accounts && model.accountPath == [newID],
    "adding an account opens its settings"
)
require(
    model.accounts[2].connectionState == .connected,
    "the wizard only hands over an account whose login it proved"
)

model.removeAccount(id: newID)
require(model.accounts.count == 2, "remove deletes the configuration")
require(
    model.accountPath.isEmpty,
    "removing the open account falls back to the account list"
)

// Permissions are three groups — read, write, send — with named presets on
// top and folder exceptions below. The preset is derived from the values, so
// a "custom" state can never drift out of sync with the switches.
require(
    PermissionSet().matchingPreset == .readAndDrafts,
    "a new account starts at read + drafts"
)
require(
    !PermissionSet().send,
    "sending is never on by default"
)
require(
    PermissionPreset.allCases.allSatisfy { !$0.write.permanentDelete },
    "no preset enables permanent deletion"
)

var permissions = PermissionSet()
permissions.apply(.fullAccess)
require(
    permissions.matchingPreset == .fullAccess,
    "applying a preset is recognized as that preset"
)
permissions.write.mark = false
require(
    permissions.matchingPreset == nil,
    "any manual change turns the preset custom"
)

// Folder exceptions scope the groups but can never exceed them; switching
// per-folder off keeps them stored, just inert.
permissions.apply(.readAndDrafts)
permissions.perFolder = true
permissions.folderRules["Private"] = FolderRule(read: false, write: false)
permissions.folderRules["Archive"] = FolderRule(read: true, write: false)
require(
    !permissions.canAccess("Private"),
    "a folder without rights is invisible to assistants"
)
require(
    permissions.readAccess(in: "INBOX") == .fullMessage
        && permissions.writeAccess(in: "INBOX").drafts,
    "folders without an exception follow the account"
)
require(
    permissions.readAccess(in: "Archive") == .fullMessage
        && permissions.writeAccess(in: "Archive").isEmpty,
    "a read-only folder takes no writes"
)
permissions.perFolder = false
require(
    permissions.canAccess("Private") && permissions.folderRules["Private"] != nil,
    "switching per-folder off keeps exceptions stored but inert"
)

// Permanent delete is an escalation of the trash right and falls with it.
require(
    !WriteAccess(trash: false, permanentDelete: true).sanitized().permanentDelete,
    "permanent delete cannot outlive the trash right"
)
require(
    model.accounts[0].permissions.matchingPreset == .fullAccess,
    "the preview work account demonstrates a preset with folder exceptions"
)

// The policy document is the bridge to the MCP server: what the switches
// say is what the server enforces. The pairing allowlist travels with it —
// hashes only, never a usable key.
let contractKey = "torro_claude-desktop_deadbeef"
let contractPairing = MCPClientKeyStore.Pairing(
    clientID: "claude-desktop",
    name: "Claude Desktop",
    tokenSHA256: MCPClientKeyStore.sha256Hex(contractKey)
)
let policyData = (try? PolicyDocument.data(for: model.accounts, clients: [contractPairing])) ?? Data()
let policyObject = (try? JSONSerialization.jsonObject(with: policyData)) as? [String: Any] ?? [:]
let policyAccounts = policyObject["accounts"] as? [[String: Any]] ?? []
let policyClients = policyObject["clients"] as? [[String: Any]] ?? []
require(
    policyClients.first?["id"] as? String == "claude-desktop"
        && policyClients.first?["token_sha256"] as? String == MCPClientKeyStore.sha256Hex(contractKey),
    "paired clients travel as an allowlist of key hashes"
)
require(
    !(String(data: policyData, encoding: .utf8) ?? "").contains(contractKey),
    "the document carries the key's hash, never the key"
)
require(
    MCPClientKeyStore.sha256Hex("abc")
        == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    "the published hash is plain SHA-256, hex-encoded — what the server computes"
)
require(
    MCPClientKeyStore.maskedToken(forClient: "hermes").hasPrefix("torro_hermes_")
        && MCPClientKeyStore.maskedToken(forClient: "hermes").hasSuffix("••••••••••••"),
    "the masked key keeps the recognizable prefix and none of the secret"
)
let workPolicy = policyAccounts.first { ($0["id"] as? String) == "work" } ?? [:]
require(
    workPolicy["read"] as? String == "with_attachments",
    "read levels travel by name"
)
require(
    workPolicy["send"] as? Bool == true,
    "the send decision reaches the document"
)
let workWrite = workPolicy["write"] as? [String: Any] ?? [:]
require(
    (workWrite["mark"] as? Bool) == true && (workWrite["permanent_delete"] as? Bool) == false,
    "write sub-rights travel individually"
)
let privateRule = (workPolicy["folder_rules"] as? [String: Any])?["Private"] as? [String: Any] ?? [:]
require(
    (privateRule["read"] as? Bool) == false && (privateRule["write"] as? Bool) == false,
    "folder exceptions travel with the document"
)

// Connection facts travel for any account that has a host, and the secret
// itself never does — only the keychain reference.
let imapAccount = MailAccount(
    id: "club",
    name: "Club",
    email: "club@example.org",
    provider: .imapSmtp,
    loginMethod: .password,
    imapHost: "imap.example.org",
    username: "club@example.org"
)
let imapData = (try? PolicyDocument.data(for: [imapAccount], clients: [])) ?? Data()
let imapObject = (try? JSONSerialization.jsonObject(with: imapData)) as? [String: Any] ?? [:]
let imapEntry = ((imapObject["accounts"] as? [[String: Any]])?.first?["imap"]) as? [String: Any] ?? [:]
require(
    imapEntry["host"] as? String == "imap.example.org"
        && imapEntry["secret_ref"] as? String == "keychain://TorroMail/club",
    "connection facts carry a keychain reference, never a password"
)
require(
    imapEntry["port"] as? Int == 993 && imapEntry["auth"] == nil,
    "a password account names its port and says nothing about OAuth"
)

// An OAuth account carries the same block plus what the server needs to renew
// the token on its own — it runs when the app does not, and a Google access
// token only lives an hour.
let oauthAccount = MailAccount(
    id: "gmail",
    name: "Sven",
    email: "sven@gmail.com",
    provider: .gmail,
    loginMethod: .oauth,
    oauthIssuer: .google,
    imapHost: "imap.gmail.com",
    username: "sven@gmail.com"
)
let oauthData = (try? PolicyDocument.data(for: [oauthAccount], clients: [])) ?? Data()
let oauthObject = (try? JSONSerialization.jsonObject(with: oauthData)) as? [String: Any] ?? [:]
let oauthEntry = ((oauthObject["accounts"] as? [[String: Any]])?.first?["imap"]) as? [String: Any] ?? [:]
require(
    oauthEntry["auth"] as? String == "xoauth2",
    "an OAuth account tells the server to authenticate with a bearer token"
)
require(
    oauthEntry["token_endpoint"] as? String == "https://oauth2.googleapis.com/token"
        && (oauthEntry["client_id"] as? String)?.isEmpty == false,
    "the server is told where and how to renew the token itself"
)
require(
    oauthEntry["secret_ref"] as? String == "keychain://TorroMail/gmail",
    "the token set stays in the keychain like any other secret"
)
// The document is a plain file on disk. The reference may name a secret; the
// secret may never be one.
let oauthText = String(data: oauthData, encoding: .utf8) ?? ""
require(
    ["access_token", "refresh_token", "client_secret", "password"]
        .allSatisfy { !oauthText.contains($0) },
    "no token, password or client secret is written into the document"
)

// An account whose setup never finished publishes no connection facts at
// all. The server refuses it rather than serving fixture mail as the user's.
let unfinished = MailAccount(
    id: "half",
    name: "Half",
    email: "half@example.org",
    provider: .imapSmtp,
    loginMethod: .password
)
let unfinishedData = (try? PolicyDocument.data(for: [unfinished], clients: [])) ?? Data()
let unfinishedObject = (try? JSONSerialization.jsonObject(with: unfinishedData)) as? [String: Any] ?? [:]
require(
    ((unfinishedObject["accounts"] as? [[String: Any]])?.first?["imap"]) == nil,
    "an account without a host carries no connection block"
)

// What you configure survives a launch. The password does not travel with
// it, and runtime facts start fresh.
let storedState = AppStateStore.State(
    accounts: model.accounts,
    settings: GeneralSettings(launchAtLogin: false, showMenuBarIcon: true)
)
let encodedState = (try? AppStateStore.encode(storedState)) ?? Data()
let restored = try? AppStateStore.decode(encodedState)
require(
    restored?.accounts.map(\.id) == ["work", "personal"],
    "accounts survive a save and load"
)
require(
    restored?.accounts.first?.permissions == model.accounts[0].permissions,
    "permissions including folder exceptions survive"
)
require(
    restored?.settings.showMenuBarIcon == true && restored?.settings.launchAtLogin == false,
    "settings survive too"
)
require(
    restored?.accounts.first?.connectionState == .connected,
    "a verified account keeps its verified state"
)
require(
    restored?.accounts.first?.pendingActions.isEmpty == true,
    "pending approvals are runtime facts and do not survive"
)
// The stored keys are exactly the configured facts: no password, no
// runtime state. (`loginMethod` may say "password" — that is a method,
// not a secret.)
let storedAccounts = ((try? JSONSerialization.jsonObject(with: encodedState)) as? [String: Any])?["accounts"] as? [[String: Any]]
require(
    Set(storedAccounts?.first?.keys ?? [:].keys) == [
        "id", "name", "email", "provider", "loginMethod", "imapHost",
        "imapPort", "smtpHost", "smtpPort", "username", "knownMailboxes",
        "permissions", "searchCache", "isVerified"
    ],
    "the state file stores the configured facts and nothing else"
)
require(
    AppStateStore.load(from: URL(fileURLWithPath: "/nonexistent/torromail-state.json")).accounts.isEmpty,
    "a first launch starts empty instead of failing"
)

// A state file written before ports were configurable carries neither key.
// It must still load — an upgrade that drops the user's accounts is worse
// than any wrong port. Built by stripping the keys from a real save, so it
// stays honest as the rest of the schema moves.
var legacyObject = ((try? JSONSerialization.jsonObject(with: encodedState)) as? [String: Any]) ?? [:]
legacyObject["accounts"] = (legacyObject["accounts"] as? [[String: Any]])?.map { account in
    var stripped = account
    stripped["imapPort"] = nil
    stripped["smtpPort"] = nil
    return stripped
}
let legacyState = (try? JSONSerialization.data(withJSONObject: legacyObject)) ?? Data()
let legacy = try? AppStateStore.decode(legacyState)
require(
    legacy?.accounts.map(\.id) == ["work", "personal"],
    "accounts saved before ports existed still load"
)
require(
    legacy?.accounts.allSatisfy { $0.imapPort == 993 && $0.smtpPort == 587 } == true,
    "accounts predating the port fields fall back to the ports they implicitly used"
)

// Connecting a client merges into its configuration instead of replacing
// it — other servers survive.
let snippet = MCPClientSetup.configSnippet(
    commandPath: "/Applications/TorroMail.app/Contents/MacOS/torromail-mcp",
    token: contractKey
)
require(
    snippet.contains("\"mcpServers\"") && snippet.contains("torromail-mcp"),
    "the config snippet names the server and its command"
)
require(
    snippet.contains("\"TORROMAIL_TOKEN\": \"\(contractKey)\""),
    "the snippet carries the access key as environment, not as an argument"
)

let temporaryConfig = FileManager.default.temporaryDirectory
    .appendingPathComponent("torromail-client-config-\(ProcessInfo.processInfo.processIdentifier).json")
let temporaryClient = MCPClient(
    id: "test",
    displayName: "Test Client",
    setup: .mcpServersJSON(configURL: temporaryConfig)
)
try? Data(#"{"mcpServers":{"other":{"command":"/usr/local/bin/other-server"}}}"#.utf8)
    .write(to: temporaryConfig)
require(
    !MCPClientSetup.isConfigured(temporaryClient),
    "a client without the server is not reported as configured"
)
try? MCPClientSetup.add(to: temporaryClient, commandPath: "/tmp/torromail-mcp", token: contractKey)
let mergedData = (try? Data(contentsOf: temporaryConfig)) ?? Data()
let mergedServers = ((try? JSONSerialization.jsonObject(with: mergedData)) as? [String: Any])?["mcpServers"] as? [String: Any] ?? [:]
require(
    mergedServers["other"] != nil && mergedServers["torromail"] != nil,
    "adding the server preserves other configured servers"
)
require(
    ((mergedServers["torromail"] as? [String: Any])?["env"] as? [String: String])?["TORROMAIL_TOKEN"] == contractKey,
    "connecting writes the access key into the client's environment"
)
require(
    MCPClientSetup.isConfigured(temporaryClient),
    "a client with the server is reported as configured"
)
try? FileManager.default.removeItem(at: temporaryConfig)

// An unreadable configuration is an error, never silently replaced.
let brokenConfig = FileManager.default.temporaryDirectory
    .appendingPathComponent("torromail-broken-config-\(ProcessInfo.processInfo.processIdentifier).json")
let brokenClient = MCPClient(
    id: "broken",
    displayName: "Broken Client",
    setup: .mcpServersJSON(configURL: brokenConfig)
)
try? Data("{ this is not json".utf8).write(to: brokenConfig)
var refusedToClobber = false
do {
    try MCPClientSetup.add(to: brokenClient, commandPath: "/tmp/torromail-mcp", token: contractKey)
} catch {
    refusedToClobber = true
}
require(refusedToClobber, "an unreadable configuration is reported instead of overwritten")
require(
    (try? String(contentsOf: brokenConfig, encoding: .utf8)) == "{ this is not json",
    "the unreadable configuration is left untouched"
)
try? FileManager.default.removeItem(at: brokenConfig)

// An empty placeholder file (Cursor ships one) is a fresh start, not a
// corrupt config: connecting writes a valid document.
let emptyConfig = FileManager.default.temporaryDirectory
    .appendingPathComponent("torromail-empty-config-\(ProcessInfo.processInfo.processIdentifier).json")
let emptyClient = MCPClient(
    id: "empty",
    displayName: "Empty Client",
    setup: .mcpServersJSON(configURL: emptyConfig)
)
try? Data().write(to: emptyConfig)
try? MCPClientSetup.add(to: emptyClient, commandPath: "/tmp/torromail-mcp", token: contractKey)
require(
    MCPClientSetup.isConfigured(emptyClient),
    "connecting a client with an empty placeholder config succeeds"
)
try? FileManager.default.removeItem(at: emptyConfig)

// Claude Code keeps its servers in a JSON file too, so detection reads the
// same top-level `mcpServers` object — the CLI only owns the writing.
let claudeCodeConfig = FileManager.default.temporaryDirectory
    .appendingPathComponent("torromail-claudecode-\(ProcessInfo.processInfo.processIdentifier).json")
let claudeCodeClient = MCPClient(
    id: "claude-code",
    displayName: "Claude Code",
    setup: .claudeCodeCLI(
        executableURL: URL(fileURLWithPath: "/usr/bin/true"),
        configURL: claudeCodeConfig
    )
)
try? Data(#"{"projects":{},"mcpServers":{"torromail":{"command":"/x"}}}"#.utf8)
    .write(to: claudeCodeConfig)
require(
    MCPClientSetup.isConfigured(claudeCodeClient),
    "Claude Code is detected from its JSON config, not by launching the CLI"
)
try? FileManager.default.removeItem(at: claudeCodeConfig)

// A CLI is found by absolute path (a GUI app has no shell PATH); a missing
// one leaves that client off the list rather than guessing.
require(
    MCPClientRegistry.resolveExecutable(
        candidates: ["/nonexistent/claude", "/usr/bin/true"],
        fileManager: .default
    )?.path == "/usr/bin/true",
    "executable resolution skips missing candidates and returns the first real one"
)
require(
    MCPClientRegistry.resolveExecutable(
        candidates: ["/nonexistent/claude"],
        fileManager: .default
    ) == nil,
    "executable resolution returns nil when nothing is installed"
)

// MCP executable resolution prefers the dev workspace before PATH.
let locator = MCPExecutableLocator(
    executableName: "torromail-mcp",
    workspaceRoot: "/repo"
)
require(
    locator.candidatePaths == ["/repo/target/debug/torromail-mcp", "torromail-mcp"],
    "MCP executable should be resolved from the dev workspace before PATH"
)
require(
    !MCPServerStatus.notFound("torromail-mcp").isRunning,
    "a missing executable is not a running server"
)
require(
    MCPServerStatus.running("torromail-mcp").isRunning,
    "running state should be observable"
)

print("TorroMailKit control-surface contract passed")
