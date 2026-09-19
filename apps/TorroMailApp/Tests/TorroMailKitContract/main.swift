import TorroMailKit
import Foundation

func require(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() {
        FileHandle.standardError.write(Data("Contract failed: \(message)\n".utf8))
        Foundation.exit(1)
    }
}

// Typography defaults, invalid saved preferences and keyboard boundaries.
require(TextSize.defaultPercent == 100, "text starts at the original system size")
require(TextSize.normalized(0) == 100 && TextSize.normalized(999) == 100,
        "invalid stored text sizes recover to the default")
require(TextSize.smaller(than: 85) == 85 && TextSize.larger(than: 160) == 160,
        "text size shortcuts stop at the supported boundaries")
var textPercent = TextSize.steps.first!
for expected in TextSize.steps.dropFirst() {
    textPercent = TextSize.larger(than: textPercent)
    require(textPercent == expected, "larger text visits each supported size once")
}
for expected in TextSize.steps.dropLast().reversed() {
    textPercent = TextSize.smaller(than: textPercent)
    require(textPercent == expected, "smaller text visits each supported size once")
}
require(TextSize.smaller(than: TextSize.defaultPercent) == 85 && TextSize.larger(than: TextSize.defaultPercent) == 115,
        "the original text size has one neighboring step in either direction")

let model = TorroMailModel.preview()

var mappedAccount = model.accounts[0]
mappedAccount.specialMailboxes = SpecialMailboxSettings(
    manual: true, drafts: "Entw&APw-rfe", sent: "Gesendet",
    archive: "Archiv", junk: "Spam", trash: "Papierkorb"
)
let mappedData = try! PolicyDocument.data(for: [mappedAccount], clients: [])
let mappedJSON = try! JSONSerialization.jsonObject(with: mappedData) as! [String: Any]
let mappedFields = (mappedJSON["accounts"] as! [[String: Any]])[0]["mailbox_overrides"] as! [String: String]
require(mappedFields["drafts"] == "Entw&APw-rfe" && mappedFields["sent"] == "Gesendet"
        && mappedFields["archive"] == "Archiv" && mappedFields["junk"] == "Spam"
        && mappedFields["trash"] == "Papierkorb", "all five folder choices reach the policy")
mappedAccount.specialMailboxes.manual = false
let automaticData = try! PolicyDocument.data(for: [mappedAccount], clients: [])
let automaticJSON = try! JSONSerialization.jsonObject(with: automaticData) as! [String: Any]
require((automaticJSON["accounts"] as! [[String: Any]])[0]["mailbox_overrides"] == nil,
        "automatic mode does not publish stale choices")
let restoredAccount = try! JSONDecoder().decode(MailAccount.self, from: JSONEncoder().encode(mappedAccount))
require(restoredAccount.specialMailboxes.sent == "Gesendet", "folder choices survive account persistence")
require(AccountMailboxList.displayName("Entw&APw-rfe") == "Entwürfe", "folder labels decode modified UTF-7")

// Decisions, not options: every sidebar destination is something the user
// decides — Overview, Mail Accounts, MCP Clients, Settings, Updates, Log, Help.
// Individual accounts live one level deeper, as cards inside the accounts
// destination.
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

// The launch-at-login toggle is a promise about the system's login items,
// and the setting alone keeps no promise: every launch reconciles the
// system's registration towards the setting, whichever side drifted.
require(
    LoginItemSync.resolve(wantsLaunchAtLogin: true, systemHasLoginItem: false) == .register,
    "an enabled setting registers the login item the system is missing"
)
require(
    LoginItemSync.resolve(wantsLaunchAtLogin: false, systemHasLoginItem: true) == .unregister,
    "a disabled setting removes a stale registration"
)
require(
    LoginItemSync.resolve(wantsLaunchAtLogin: true, systemHasLoginItem: true) == .inSync,
    "a kept promise touches the system's list not at all"
)
require(
    LoginItemSync.resolve(wantsLaunchAtLogin: false, systemHasLoginItem: false) == .inSync,
    "off and unregistered needs no ServiceManagement call"
)

// Reopening TorroMail (Dock click, launching it again from the Finder) must
// show exactly one window. Two actors can respond — this app and AppKit's
// default handling — so the rule is mutual exclusion: whoever opens the
// window silences the other, or every reopen would open it twice.
require(
    ReopenResponse.resolve(hasVisibleWindows: false) == .showMainWindow,
    "reopening without a window opens the main window ourselves — and only ourselves"
)
require(
    ReopenResponse.resolve(hasVisibleWindows: true) == .letSystemProceed,
    "with a window on screen the system merely brings it forward"
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

// A key an older build wrote carries that build's access list, so the current
// app cannot read it — and an item that exists is refreshed in place, which
// leaves the stale list alone. Reconnecting has to drop it and mint a fresh
// one, or the client stays locked out no matter how often it reconnects.
final class KeyStorageProbe: @unchecked Sendable {
    var stored: [String: String] = [:]
    var unreadable: Set<String> = []
    var deleted: [String] = []
}

func probeStorage(_ probe: KeyStorageProbe) -> MCPClientKeyStore.Storage {
    MCPClientKeyStore.Storage(
        read: { account in
            probe.unreadable.contains(account) ? nil : probe.stored[account]
        },
        write: { password, account in
            probe.stored[account] = password
            probe.unreadable.remove(account)
        },
        delete: { account in
            probe.stored[account] = nil
            probe.unreadable.remove(account)
            probe.deleted.append(account)
        }
    )
}

let legacyProbe = KeyStorageProbe()
legacyProbe.stored["client-key-legacy"] = "torro_legacy_unreachable"
legacyProbe.unreadable.insert("client-key-legacy")
let healedKey = try? MCPClientKeyStore.tokenCreatingIfNeeded(
    forClient: "legacy",
    storage: probeStorage(legacyProbe)
)
require(
    healedKey?.hasPrefix("torro_legacy_") == true && healedKey != "torro_legacy_unreachable",
    "an unreadable key is replaced, so reconnecting actually re-pairs the client"
)
require(
    legacyProbe.deleted == ["client-key-legacy"],
    "the stale item is dropped first — only a fresh add carries the current access list"
)

// The other half of the same rule: a key that works is never rotated behind
// the user's back, because the client's config still carries it.
let healthyProbe = KeyStorageProbe()
healthyProbe.stored["client-key-steady"] = "torro_steady_intact"
let steadyKey = try? MCPClientKeyStore.tokenCreatingIfNeeded(
    forClient: "steady",
    storage: probeStorage(healthyProbe)
)
require(
    steadyKey == "torro_steady_intact" && healthyProbe.deleted.isEmpty,
    "reconnecting keeps a readable key stable"
)

// Renewal replaces the item outright: an in-place refresh would keep a legacy
// access list alive, which is the failure this whole path exists to end.
let renewProbe = KeyStorageProbe()
renewProbe.stored["client-key-rotate"] = "torro_rotate_old"
let renewedKey = try? MCPClientKeyStore.renewToken(
    forClient: "rotate",
    storage: probeStorage(renewProbe)
)
require(
    renewedKey?.hasPrefix("torro_rotate_") == true && renewedKey != "torro_rotate_old",
    "renewal mints a different key"
)
require(
    renewProbe.deleted == ["client-key-rotate"],
    "renewal drops the old item instead of refreshing it in place"
)

// A client reads TORROMAIL_TOKEN once, when it spawns the server, so a
// session that started before the key changed keeps presenting the old one
// and every tool call it makes fails. Saying so is the whole point: the user
// has to restart the assistant, and nothing else will fix it.
let keyChange = Date(timeIntervalSince1970: 1_000)
require(
    MCPClientKeyStore.restartPending(keyChangedAt: keyChange, lastConnected: nil),
    "a client that never connected has to start before its key can take effect"
)
require(
    MCPClientKeyStore.restartPending(
        keyChangedAt: keyChange,
        lastConnected: keyChange.addingTimeInterval(-60)
    ),
    "a session older than the key change is still carrying the old key"
)
require(
    !MCPClientKeyStore.restartPending(
        keyChangedAt: keyChange,
        lastConnected: keyChange.addingTimeInterval(60)
    ),
    "a client that connected after the change has already loaded the new key"
)
require(
    !MCPClientKeyStore.restartPending(keyChangedAt: nil, lastConnected: nil),
    "an untouched key asks nothing of the user"
)

// The same trap one layer down, and this one holds the mail passwords. An
// item an older build wrote cannot be read by this one, and refreshing it in
// place keeps that access list — so re-entering the password would fail
// exactly as silently as reconnecting did. Storing a value the caller is
// already holding can replace such an item outright: what it protects is
// unreachable anyway.
final class ItemStoreProbe: @unchecked Sendable {
    var stored: [String: String] = [:]
    var unreadable: Set<String> = []
    var operations: [String] = []
}

func probeItemStore(_ probe: ItemStoreProbe) -> KeychainStore.ItemStore {
    KeychainStore.ItemStore(
        add: { account, secret in
            probe.operations.append("add")
            if probe.stored[account] != nil { return false }
            probe.stored[account] = secret
            return true
        },
        read: { account in
            probe.unreadable.contains(account) ? nil : probe.stored[account]
        },
        exists: { account in probe.stored[account] != nil },
        update: { account, secret in
            probe.operations.append("update")
            probe.stored[account] = secret
        },
        delete: { account in
            probe.operations.append("delete")
            probe.stored[account] = nil
            probe.unreadable.remove(account)
        }
    )
}

let legacySecret = ItemStoreProbe()
legacySecret.stored["account-1"] = "unreachable"
legacySecret.unreadable.insert("account-1")
try? KeychainStore.savePassword(
    "freshly typed",
    forAccount: "account-1",
    store: probeItemStore(legacySecret)
)
require(
    legacySecret.operations == ["add", "delete", "add"],
    "an unreadable item is replaced, so re-entering the password repairs the account"
)
require(
    legacySecret.stored["account-1"] == "freshly typed",
    "the replacement carries the value the caller just supplied"
)

// A readable item keeps its access list: it is what lets the bundled server
// in, and rewriting it buys nothing.
let liveSecret = ItemStoreProbe()
liveSecret.stored["account-2"] = "current"
try? KeychainStore.savePassword(
    "rotated",
    forAccount: "account-2",
    store: probeItemStore(liveSecret)
)
require(
    liveSecret.operations == ["add", "update"],
    "a readable item is refreshed in place, never dropped"
)

let firstSecret = ItemStoreProbe()
try? KeychainStore.savePassword(
    "brand new",
    forAccount: "account-3",
    store: probeItemStore(firstSecret)
)
require(
    firstSecret.operations == ["add"] && firstSecret.stored["account-3"] == "brand new",
    "a first write is a plain add"
)

// Whether a password is stored is answered from the item's existence, not
// from its value — an unreadable item still holds one. Those are exactly the
// accounts waiting to be repaired, and a field that looks empty there reads
// as "TorroMail threw my password away".
let unreadableItem = ItemStoreProbe()
unreadableItem.stored["account-4"] = "unreachable"
unreadableItem.unreadable.insert("account-4")
require(
    KeychainStore.hasPassword(forAccount: "account-4", store: probeItemStore(unreadableItem)),
    "a stored password is reported even when its value cannot be read"
)
require(
    !KeychainStore.hasPassword(forAccount: "account-4", store: probeItemStore(ItemStoreProbe())),
    "an account without an item has nothing to show"
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
// The encryption is stated, not left to be inferred from the port. A server
// speaking implicit TLS somewhere other than 993 is the whole reason.
require(
    imapEntry["security"] as? String == "tls",
    "the IMAP block says which encryption to use"
)
let statedSecurity = MailAccount(
    id: "odd",
    name: "Odd",
    email: "odd@example.org",
    provider: .imapSmtp,
    loginMethod: .password,
    imapHost: "imap.example.org",
    imapPort: 1993,
    imapSecurity: .tls,
    smtpHost: "smtp.example.org",
    smtpPort: 2465,
    smtpSecurity: .startTLS,
    username: "odd@example.org"
)
let oddData = (try? PolicyDocument.data(for: [statedSecurity], clients: [])) ?? Data()
let oddObject = (try? JSONSerialization.jsonObject(with: oddData)) as? [String: Any] ?? [:]
let oddAccount = (oddObject["accounts"] as? [[String: Any]])?.first ?? [:]
require(
    (oddAccount["imap"] as? [String: Any])?["security"] as? String == "tls"
        && (oddAccount["smtp"] as? [String: Any])?["security"] as? String == "starttls",
    "an unconventional port does not override the stated encryption"
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
        "imapPort", "imapSecurity", "smtpHost", "smtpPort", "smtpSecurity",
        "username", "knownMailboxes", "permissions", "specialMailboxes", "searchCache", "isVerified"
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
    stripped["imapSecurity"] = nil
    stripped["smtpSecurity"] = nil
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
require(
    legacy?.accounts.allSatisfy { $0.imapSecurity == .tls && $0.smtpSecurity == .startTLS } == true,
    "and to the encryption those ports used to imply, so they connect as before"
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

// MARK: - Account health

// The whole point of the feature: a green dot must mean "checked recently and
// fine", not "worked once during setup".

func healthRecord(
    _ account: String,
    _ outcome: HealthOutcome,
    secondsAgo: TimeInterval = 0,
    detail: String = ""
) -> HealthRecord {
    HealthRecord(
        accountID: account,
        at: Date().addingTimeInterval(-secondsAgo),
        outcome: outcome,
        source: "periodic",
        detail: detail
    )
}

require(
    HealthLog.derive(records: [], accountID: "work", fallback: .connected) == .connected,
    "with no records at all the stored state stands"
)
require(
    HealthLog.derive(
        records: [healthRecord("work", .ok)],
        accountID: "work",
        fallback: .needsTest
    ) == .connected,
    "a successful check is green whatever the stored state said"
)
require(
    HealthLog.derive(
        records: [healthRecord("work", .rejected, detail: "credentials rejected: NO")],
        accountID: "work",
        fallback: .connected
    ).isBroken,
    "refused credentials go red immediately — they will not fix themselves"
)
require(
    HealthLog.derive(
        records: [
            healthRecord("work", .ok, secondsAgo: 300),
            healthRecord("work", .unreachable, secondsAgo: 200),
            healthRecord("work", .unreachable, secondsAgo: 100)
        ],
        accountID: "work",
        fallback: .needsTest
    ) == .connected,
    "two unreachable checks are a flaky network, not a broken account"
)
require(
    HealthLog.derive(
        records: [
            healthRecord("work", .ok, secondsAgo: 400),
            healthRecord("work", .unreachable, secondsAgo: 300),
            healthRecord("work", .unreachable, secondsAgo: 200),
            healthRecord("work", .unreachable, secondsAgo: 100)
        ],
        accountID: "work",
        fallback: .needsTest
    ).isBroken,
    "three in a row is a problem worth showing"
)
require(
    HealthLog.derive(
        records: [
            healthRecord("work", .unreachable, secondsAgo: 500),
            healthRecord("work", .unreachable, secondsAgo: 400),
            healthRecord("work", .ok, secondsAgo: 300),
            healthRecord("work", .unreachable, secondsAgo: 200),
            healthRecord("work", .unreachable, secondsAgo: 100)
        ],
        accountID: "work",
        fallback: .needsTest
    ) == .connected,
    "a success in between resets the count"
)
require(
    HealthLog.derive(
        records: [healthRecord("other", .rejected)],
        accountID: "work",
        fallback: .connected
    ) == .connected,
    "another account's trouble is not this account's"
)
// Rejected credentials outlive an outage: the server going quiet afterwards is
// not evidence the password started working again.
require(
    HealthLog.derive(
        records: [
            healthRecord("work", .rejected, secondsAgo: 300, detail: "credentials rejected: NO"),
            healthRecord("work", .unreachable, secondsAgo: 200),
            healthRecord("work", .unreachable, secondsAgo: 100)
        ],
        accountID: "work",
        fallback: .connected
    ) == .failed("credentials rejected: NO"),
    "a short outage after a rejection keeps the rejection, reason and all"
)

// A line whose outcome word we do not know is skipped, not fatal: an older or
// newer build writing a word this one has never heard of must not take the
// status display down with it.
let mixedLog = FileManager.default.temporaryDirectory
    .appendingPathComponent("torromail-contract-health.jsonl")
try? FileManager.default.removeItem(at: mixedLog)
FileManager.default.createFile(
    atPath: mixedLog.path,
    contents: Data(
        """
        {"ts":1,"account":"work","outcome":"quantum","source":"periodic","detail":""}
        {"ts":2,"account":"work","outcome":"ok","source":"periodic","detail":""}
        not json at all
        """.utf8
    )
)
require(
    HealthLog.load(from: mixedLog).count == 1,
    "an unknown outcome and a broken line are both skipped, and the good line survives"
)

// Retention is per account. With a global tail, one long session against a
// busy account crowds a quiet account off the end, that account derives from
// its stored state, and the stale-green bug is back — only under load, which
// is the worst way for it to come back.
let crowdedLog = FileManager.default.temporaryDirectory
    .appendingPathComponent("torromail-contract-crowded.jsonl")
try? FileManager.default.removeItem(at: crowdedLog)
HealthLog.append(
    HealthRecord(accountID: "quiet", at: Date(timeIntervalSince1970: 1), outcome: .rejected),
    to: crowdedLog
)
for index in 2...200 {
    HealthLog.append(
        HealthRecord(
            accountID: "busy",
            at: Date(timeIntervalSince1970: TimeInterval(index)),
            outcome: .ok
        ),
        to: crowdedLog
    )
}
let crowded = HealthLog.load(from: crowdedLog)
require(
    crowded.contains { $0.accountID == "quiet" },
    "a quiet account's record survives a busy account's flood"
)
require(
    crowded.filter { $0.accountID == "busy" }.count == 20,
    "and the busy account is capped at its own per-account retention"
)
// What was appended is what comes back — a log the app cannot read back is a
// log that silently stops deciding anything.
require(
    HealthLog.derive(records: crowded, accountID: "quiet", fallback: .connected).isBroken,
    "the quiet account's rejection still derives red after the round trip"
)

// MARK: - What the check made of stderr

// The binary writes the outcome word alone on the first line and the reason
// after it. Getting this wrong is silent: the dot still moves, it just blames
// the wrong thing — so every shape the binary can produce is pinned here.
require(
    AccountCheckResult.parsing(stderr: "rejected\ncredentials rejected: NO [AUTHENTICATIONFAILED]")
        == AccountCheckResult(
            outcome: .rejected,
            detail: "credentials rejected: NO [AUTHENTICATIONFAILED]"
        ),
    "the first line names the outcome and the rest is the reason"
)
require(
    AccountCheckResult.parsing(stderr: "unreachable\ndns lookup for imap.example.com failed")
        == AccountCheckResult(
            outcome: .unreachable,
            detail: "dns lookup for imap.example.com failed"
        ),
    "an unreachable server is classified as one, reason intact"
)
require(
    AccountCheckResult.parsing(stderr: "unreachable\ntls handshake failed\ncaused by: timed out")
        .detail == "tls handshake failed\ncaused by: timed out",
    "a reason spanning several lines survives whole"
)
// A binary older than the two-part contract writes prose only. Reading that as
// unreachable earns a grace period; reading it as rejected would accuse a
// password that is probably fine.
require(
    AccountCheckResult.parsing(stderr: "could not open a connection to imap.example.com")
        == AccountCheckResult(
            outcome: .unreachable,
            detail: "could not open a connection to imap.example.com"
        ),
    "prose with no outcome word is unreachable and keeps every word of itself"
)
require(
    AccountCheckResult.parsing(stderr: "  \n ") == AccountCheckResult(
        outcome: .unreachable,
        detail: "connection check failed"
    ),
    "a check that said nothing still has to say something"
)
require(
    AccountCheckResult.parsing(stderr: "rejected") == AccountCheckResult(
        outcome: .rejected,
        detail: "connection check failed"
    ),
    "an outcome word with no reason after it keeps the outcome"
)
// This function is only ever handed a *failing* check's stderr, so `ok` on the
// first line is a contradiction — and believing it would turn a failed check
// green, which is the exact bug this feature exists to remove.
require(
    AccountCheckResult.parsing(stderr: "ok\nwait, no").outcome == .unreachable,
    "a failing check cannot talk its way into being ok"
)
// The manual button acts on one check alone: whatever it found, the user asked
// just now and deserves the answer now.
require(
    AccountCheckResult(outcome: .ok, detail: "").state == .connected,
    "a passing check is green"
)
require(
    AccountCheckResult(outcome: .unreachable, detail: "the connection check timed out").state
        == .failed("the connection check timed out"),
    "a failing check shows its reason rather than waiting for a streak"
)

// Notifications follow crossings, not states — otherwise a broken account
// announces itself every fifteen minutes until the user stops reading.
let brokenAccount = MailAccount(
    id: "work",
    name: "Work",
    email: "work@example.com",
    provider: .imapSmtp,
    loginMethod: .password,
    username: "work@example.com",
    connectionState: .failed("credentials rejected: NO")
)
let healthyAccount = MailAccount(
    id: "work",
    name: "Work",
    email: "work@example.com",
    provider: .imapSmtp,
    loginMethod: .password,
    username: "work@example.com",
    connectionState: .connected
)
require(
    HealthLog.transitions(previous: ["work": false], accounts: [brokenAccount]).count == 1,
    "going from healthy to broken is worth saying once"
)
require(
    HealthLog.transitions(previous: ["work": true], accounts: [brokenAccount]).isEmpty,
    "a still-broken account says nothing further"
)
require(
    HealthLog.transitions(previous: ["work": true], accounts: [healthyAccount])
        == [.recovered(accountID: "work")],
    "recovery gets its own quiet all-clear"
)
require(
    HealthLog.transitions(previous: [:], accounts: [brokenAccount]).isEmpty,
    "an account seen for the first time has not crossed anything"
)
// The snapshot a later round compares against has to be the shape `previous`
// takes, or every round reads as "first seen" and nothing ever notifies.
require(
    HealthLog.transitions(
        previous: HealthLog.brokenness(accounts: [healthyAccount]),
        accounts: [brokenAccount]
    ) == [.broke(accountID: "work", reason: "credentials rejected: NO")],
    "brokenness feeds straight back in, reason carried through to the notification"
)

// MARK: - What the monitor does between checks

/// Stands in for the real check. Counts what was asked for and how much of it
/// happened at once, and takes as long as it is told to — a real check can hold
/// the monitor for half a minute, and every decision worth pinning here is
/// about what the monitor does while one is in flight.
final class CheckSpy: @unchecked Sendable {
    private let lock = NSLock()
    private var asked: [String] = []
    private var live = 0
    private var peak = 0
    private let duration: TimeInterval

    init(duration: TimeInterval) {
        self.duration = duration
    }

    func check(_ accountID: String) -> AccountCheckResult {
        lock.withLock {
            asked.append(accountID)
            live += 1
            peak = max(peak, live)
        }
        if duration > 0 {
            Thread.sleep(forTimeInterval: duration)
        }
        lock.withLock { live -= 1 }
        return AccountCheckResult(outcome: .ok, detail: "")
    }

    var accountsAsked: [String] { lock.withLock { asked } }
    var mostAtOnce: Int { lock.withLock { peak } }
}

func contractLog(_ name: String) -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(name)
    try? FileManager.default.removeItem(at: url)
    return url
}

// A pass is one login at a time, and every account it reaches lands in the log
// as a periodic record — the file the dots are derived from, so a check nobody
// wrote down changed nothing.
let serialSpy = CheckSpy(duration: 0.05)
let serialLog = contractLog("torromail-contract-monitor-serial.jsonl")
let serialMonitor = AccountHealthMonitor(
    interval: 0.2,
    logURL: serialLog,
    check: { accountID, _ in serialSpy.check(accountID) }
)
serialMonitor.update(accountIDs: ["work", "personal"], executableName: "torromail-mcp")
serialMonitor.start()
// Waits for the pass rather than for a fixed 0.3 s: two 50 ms checks fit in
// that on a quiet machine and do not on a loaded CI runner, where this failed
// now and then for reasons that had nothing to do with the monitor.
let serialDeadline = Date().addingTimeInterval(5)
while Set(HealthLog.load(from: serialLog).map(\.accountID)).count < 2, Date() < serialDeadline {
    Thread.sleep(forTimeInterval: 0.05)
}
serialMonitor.stop()
require(
    serialSpy.mostAtOnce == 1,
    "accounts are checked one after another, never as a burst of simultaneous logins"
)
let serialRecords = HealthLog.load(from: serialLog)
require(
    Set(serialRecords.map(\.accountID)) == ["work", "personal"],
    "every account the pass reached is written to the health log"
)
require(
    serialRecords.allSatisfy { $0.source == "periodic" && $0.outcome == .ok },
    "and written as what it was: a periodic check that succeeded"
)

// `cancel()` cannot interrupt a check already in flight, so a stopped monitor
// has to notice between accounts. Without that, quitting the app leaves a pass
// grinding through a list of half-minute timeouts nobody is waiting for.
let stoppedSpy = CheckSpy(duration: 0.3)
let stoppedMonitor = AccountHealthMonitor(
    interval: 60,
    logURL: contractLog("torromail-contract-monitor-stopped.jsonl"),
    check: { accountID, _ in stoppedSpy.check(accountID) }
)
stoppedMonitor.update(accountIDs: ["one", "two", "three"], executableName: "torromail-mcp")
stoppedMonitor.start()
Thread.sleep(forTimeInterval: 0.1)
stoppedMonitor.stop()
Thread.sleep(forTimeInterval: 0.6)
require(
    stoppedSpy.accountsAsked == ["one"],
    "a stopped pass abandons the accounts it has not reached yet"
)

// An empty executable name is a cleared text field, not a broken mailbox: it
// must produce no record at all, and it must not be the end of the cadence
// either, or configuring the binary later would never start the checks again.
let configuredSpy = CheckSpy(duration: 0)
let configuredLog = contractLog("torromail-contract-monitor-configured.jsonl")
let configuredMonitor = AccountHealthMonitor(
    interval: 0.1,
    logURL: configuredLog,
    check: { accountID, _ in configuredSpy.check(accountID) }
)
configuredMonitor.update(accountIDs: ["work"], executableName: "")
configuredMonitor.start()
Thread.sleep(forTimeInterval: 0.25)
require(
    configuredSpy.accountsAsked.isEmpty && HealthLog.load(from: configuredLog).isEmpty,
    "with no executable configured nothing is checked and nothing is blamed on the account"
)
configuredMonitor.update(accountIDs: ["work"], executableName: "torromail-mcp")
Thread.sleep(forTimeInterval: 0.3)
configuredMonitor.stop()
require(
    !configuredSpy.accountsAsked.isEmpty,
    "a pass with nothing it could check still schedules the next one"
)

// MARK: - What the monitor is allowed to ask about

// An account the wizard never finished is a setup gap, not a health problem.
// The wire has three words — `ok`, `rejected`, `unreachable` — and none of them
// means "nothing to report", so `--check-account` answers `unreachable` with
// "has no connection configured". Asked every fifteen minutes, three of those
// elapse the grace period and the app raises an alarm about an account the user
// has simply not finished creating: the disease this whole feature exists to
// cure, coming back through the one route the server cannot close. Not asking
// is the only cure, so it is pinned here.
let unfinishedAccount = MailAccount(
    id: "unfinished",
    name: "Unfinished",
    email: "unfinished@example.com",
    provider: .imapSmtp,
    loginMethod: .password
)
let hostOnlyAccount = MailAccount(
    id: "host-only",
    name: "Host only",
    email: "host@example.com",
    provider: .imapSmtp,
    loginMethod: .password,
    imapHost: "imap.example.com"
)
let checkableAccount = MailAccount(
    id: "checkable",
    name: "Checkable",
    email: "checkable@example.com",
    provider: .imapSmtp,
    loginMethod: .password,
    imapHost: "imap.example.com",
    username: "checkable@example.com",
    connectionState: .connected
)
let checkableModel = TorroMailModel(
    accounts: [unfinishedAccount, hostOnlyAccount, checkableAccount],
    selectedSidebarItem: .dashboard,
    generalSettings: GeneralSettings(),
    audit: []
)
require(
    checkableModel.checkableAccountIDs == ["checkable"],
    "only an account the server has connection facts for is ever checked"
)
// The line is the policy document's, not a second rule invented here: half a
// connection is no connection to the server, so half a connection must not be
// asked about either.
require(
    !unfinishedAccount.hasIMAPConnection && !hostOnlyAccount.hasIMAPConnection,
    "an empty shell and a half-filled one are both unconfigured"
)
require(
    checkableAccount.hasIMAPConnection,
    "host and username together are what the server can open"
)

// MARK: - The model follows the log

// The wire the whole feature hangs from: what the log says has to reach the
// dots, and what it does not say has to leave them alone.
let followLog = contractLog("torromail-contract-follow.jsonl")
let followModel = TorroMailModel(
    accounts: [checkableAccount],
    selectedSidebarItem: .dashboard,
    generalSettings: GeneralSettings(),
    audit: []
)
followModel.applyHealth(from: followLog)
require(
    followModel.accounts[0].connectionState == .connected
        && followModel.lastHealthCheck.isEmpty,
    "an empty log changes no dot and claims no check"
)
HealthLog.append(
    HealthRecord(
        accountID: "checkable",
        at: Date(timeIntervalSince1970: 1_000),
        outcome: .rejected,
        source: "periodic",
        detail: "credentials rejected: NO"
    ),
    to: followLog
)
followModel.applyHealth(from: followLog)
require(
    followModel.accounts[0].connectionState == .failed("credentials rejected: NO"),
    "a rejection in the log turns the stored green dot red, reason and all"
)
require(
    followModel.lastHealthCheck["checkable"] == Date(timeIntervalSince1970: 1_000),
    "and the account detail can say when that was"
)


// Cache settings: the single level replaces the four switches, and a state
// file written by the previous build still decodes to the right level.
let legacyCacheJSON = Data("""
{"localCacheEnabled":true,"cacheMode":"fullText","indexBodies":true,"indexAttachments":false,"storage":"12 MB"}
""".utf8)
require(
    (try? JSONDecoder().decode(SearchCacheSettings.self, from: legacyCacheJSON))?.level == .bodies,
    "legacy fullText cache decodes to the bodies level"
)
let disabledCacheJSON = Data("""
{"localCacheEnabled":false,"cacheMode":"metadata","indexBodies":false,"indexAttachments":false,"storage":"0 MB"}
""".utf8)
require(
    (try? JSONDecoder().decode(SearchCacheSettings.self, from: disabledCacheJSON))?.level == .off,
    "a disabled legacy cache decodes to off"
)
require(
    (try? JSONDecoder().decode(SearchCacheSettings.self, from: Data(#"{"level":"attachments"}"#.utf8)))?.level == .attachments,
    "the new shape decodes directly"
)
require(
    CacheLevel.ceiling(for: .headers) == .headers && CacheLevel.ceiling(for: .fullMessage) == .bodies
        && CacheLevel.ceiling(for: .withAttachments) == .attachments && CacheLevel.ceiling(for: ReadAccess.none) == .off,
    "the read permission caps the cache level"
)
let cachePublishAccount = MailAccount(
    id: "cache-contract",
    name: "Cache Contract",
    email: "cache@example.com",
    provider: .imapSmtp,
    loginMethod: .password,
    searchCache: SearchCacheSettings(level: .attachments)
)
let cachePolicyData = (try? PolicyDocument.data(for: [cachePublishAccount], clients: [])) ?? Data()
let cachePolicyText = String(data: cachePolicyData, encoding: .utf8) ?? ""
let cacheAccounts = ((try? JSONSerialization.jsonObject(with: cachePolicyData)) as? [String: Any])?["accounts"] as? [[String: Any]] ?? []
let cacheBlock = cacheAccounts.first?["cache"] as? [String: Any]
require(
    cacheBlock?["level"] as? String == "attachments" && !cachePolicyText.contains("local_cache_enabled"),
    "the published cache block carries the level and nothing legacy"
)

// Provider detection. The address says firma.de; only DNS says whether the
// mail is really at Google or Microsoft, and getting that wrong hands the user
// a password field no tenant will accept (issue #1).
require(
    ProviderCatalog.fromMXHost("staude-net.mail.protection.outlook.com")?.provider == .microsoft,
    "a Microsoft 365 MX is recognised as Microsoft"
)
require(
    ProviderCatalog.fromMXHost("aspmx.l.google.com")?.provider == .gmail
        && ProviderCatalog.fromMXHost("smtp.google.com")?.provider == .gmail,
    "both of Google's MX shapes are recognised"
)
require(
    ProviderCatalog.fromMXHost("mx01.kundenserver.de") == nil,
    "a hoster's own MX claims nothing"
)
require(
    ProviderCatalog.fromSPF("v=spf1 include:spf.protection.outlook.com -all")?.provider == .microsoft,
    "SPF finds a tenant whose MX hides behind a filtering gateway"
)
require(
    ProviderCatalog.fromSPF("v=spf1 include:_spf.google.com ~all")?.provider == .gmail,
    "SPF finds a Workspace domain the same way"
)
require(
    ProviderCatalog.fromSPF("v=spf1 include:_spf.mailingservice.de -all") == nil
        && ProviderCatalog.fromSPF("google-site-verification=abc") == nil,
    "only SPF records, and only the two hyperscalers, are read out of TXT"
)

// The Plesk default: every domain on a shared hoster gets this file, and it
// keeps advertising the webspace's IMAP long after the mailboxes moved. It has
// to parse — and it has to lose to the MX record, which is why the chain asks
// DNS first.
let pleskAutoconfig = Data(#"""
<clientConfig version="1.1">
  <emailProvider id="staude.net">
    <domain>staude.net</domain>
    <displayName>staude.net</displayName>
    <incomingServer type="imap">
      <hostname>staude.net</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>staude.net</hostname>
      <port>465</port>
      <socketType>SSL</socketType>
      <authentication>password-cleartext</authentication>
    </outgoingServer>
  </emailProvider>
</clientConfig>
"""#.utf8)
let parsedPlesk = AutoconfigParser.parse(pleskAutoconfig, source: "autoconfig")
require(
    parsedPlesk?.imapHost == "staude.net" && parsedPlesk?.auth == .password,
    "the hoster's generated autoconfig still parses"
)
require(
    ProviderCatalog.fromMXHost("staude-net.mail.protection.outlook.com")?.auth == .oauth(.microsoft),
    "and the MX record outranks it with a token path"
)


// Shared accounts persist independently of client keys. Migration runs only
// once; a newly paired client cannot acquire all accounts implicitly.
do {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("torromail-account-access-\(UUID().uuidString)")
    let url = directory.appendingPathComponent("grants.json")
    defer { try? FileManager.default.removeItem(at: directory) }
    let migrated = try ClientAccountAccessStore.load(from: url, legacyClientIDs: ["existing"])
    require(migrated["existing"] == .all, "existing clients preserve account access on migration")
    var grants = try ClientAccountAccessStore.load(from: url, legacyClientIDs: ["existing", "new"])
    require(grants["new"] == nil, "migration never implicitly grants later clients all accounts")
    grants["existing"] = .selected(["work", "shared"])
    grants["other"] = .selected(["personal", "shared"])
    grants["empty"] = .selected([])
    try ClientAccountAccessStore.save(grants, to: url)
    let restored = try ClientAccountAccessStore.load(from: url)
    require(restored == grants, "overlapping and empty selections survive reloading")

    let pairings = ["existing", "other", "empty", "new", MCPClientKeyStore.appClientID].map {
        MCPClientKeyStore.Pairing(clientID: $0, name: $0, tokenSHA256: "rotated-key-hash")
    }
    let applied = ClientAccountAccessStore.applying(restored, to: pairings)
    require(applied[0].accountAccess == .selected(["work", "shared"]), "key renewal preserves selected accounts")
    require(applied[1].accountAccess == .selected(["personal", "shared"]), "accounts may be shared with multiple clients")
    require(applied[2].accountAccess == .selected([]) && applied[3].accountAccess == .selected([]), "empty selections and new clients grant nothing")
    require(applied[4].accountAccess == .all, "the app can still check and manage every account")
    let data = try PolicyDocument.data(for: model.accounts, clients: applied)
    let object = try JSONSerialization.jsonObject(with: data) as! [String: Any]
    let clients = object["clients"] as! [[String: Any]]
    let selection = clients[0]["account_access"] as! [String: Any]
    require(selection["mode"] as? String == "selected" && selection["account_ids"] as? [String] == ["shared", "work"], "the policy publishes the exact selected IDs in stable order")
    let all = clients[4]["account_access"] as! [String: Any]
    require(all["mode"] as? String == "all" && all["account_ids"] == nil, "all accounts is an explicit mode, not a snapshot")

    let invalid = Data(#"{"existing":{"mode":"selected"}}"#.utf8)
    try invalid.write(to: url)
    do {
        _ = try ClientAccountAccessStore.load(from: url, legacyClientIDs: ["existing"])
        require(false, "malformed access must fail instead of migrating to all accounts")
    } catch { }
    let unchanged = try Data(contentsOf: url)
    require(unchanged == invalid, "malformed grants are not silently overwritten")
} catch {
    require(false, "account access persistence: \(error)")
}

// MARK: - The policy document, shared with Rust

// `contracts/policy-document.json` holds accounts in the shape `state.json`
// stores them, clients, and the document both must publish. The Rust writer
// (crates/torromail-control) runs the same file, so this `PolicyDocument` and
// that one cannot come to disagree without one of the two suites going red.
//
// apps/TorroMailApp/Tests/TorroMailKitContract/main.swift → the repository
// root is five levels up.
let policyCasesURL = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .deletingLastPathComponent()
    .deletingLastPathComponent()
    .deletingLastPathComponent()
    .deletingLastPathComponent()
    .appendingPathComponent("contracts/policy-document.json")
// The file names the OAuth client ids by placeholder, because they belong to
// the build: swap in the ones this build carries before reading a single case.
let policyCasesText = ((try? String(contentsOf: policyCasesURL, encoding: .utf8)) ?? "")
    .replacingOccurrences(of: "$GOOGLE_CLIENT_ID", with: OAuthIssuer.google.clientID)
    .replacingOccurrences(of: "$MICROSOFT_CLIENT_ID", with: OAuthIssuer.microsoft.clientID)
let policyCasesFile = (try? JSONSerialization.jsonObject(with: Data(policyCasesText.utf8))) as? [String: Any]
require(policyCasesFile != nil, "the shared policy cases are readable at \(policyCasesURL.path)")
require(
    policyCasesFile?["secret_ref_prefix"] as? String == KeychainStore.secretReference(forAccount: ""),
    "the shared cases and the app agree on where secrets are referenced"
)
let policyCases = policyCasesFile?["cases"] as? [[String: Any]] ?? []
require(!policyCases.isEmpty, "the shared policy cases are not empty")
for policyCase in policyCases {
    let caseName = policyCase["name"] as? String ?? "unnamed"
    do {
        let accountsData = try JSONSerialization.data(withJSONObject: policyCase["accounts"] ?? [])
        let caseAccounts = try JSONDecoder().decode([MailAccount].self, from: accountsData)
        let casePairings = try (policyCase["clients"] as? [[String: Any]] ?? []).map { client in
            MCPClientKeyStore.Pairing(
                clientID: client["id"] as? String ?? "",
                name: client["name"] as? String ?? "",
                tokenSHA256: client["token_sha256"] as? String ?? "",
                accountAccess: try JSONDecoder().decode(
                    ClientAccountAccess.self,
                    from: JSONSerialization.data(withJSONObject: client["account_access"] ?? [:])
                )
            )
        }
        let published = try JSONSerialization.jsonObject(
            with: PolicyDocument.data(for: caseAccounts, clients: casePairings)
        ) as? NSDictionary
        let expected = policyCase["expected"] as? NSDictionary
        if published != expected {
            // Both documents, so a red CI run says what differs without anyone
            // needing a Mac to find out.
            let shown = (try? JSONSerialization.data(
                withJSONObject: published ?? [:],
                options: [.sortedKeys, .prettyPrinted]
            )).flatMap { String(data: $0, encoding: .utf8) } ?? "unprintable"
            FileHandle.standardError.write(Data("Swift published for “\(caseName)”:\n\(shown)\n".utf8))
        }
        require(published == expected, "shared policy case: \(caseName)")

        // The same accounts must survive this app's own save and load, or the
        // two surfaces could not share a state file.
        let reread = try JSONDecoder().decode([MailAccount].self, from: JSONEncoder().encode(caseAccounts))
        require(reread == caseAccounts, "shared policy case round-trips through state.json: \(caseName)")
    } catch {
        require(false, "shared policy case “\(caseName)” could not be read: \(error)")
    }
}

// MARK: - Client configs and snippets, shared with Rust

// `contracts/client-config.json`: how TorroMail writes itself into a client's
// JSON config, and the snippets it offers for pasting. The Rust side
// (crates/torromail-control/tests/client_setup.rs) runs the same file, so the
// terminal surface and this app leave a client's config in the same state.
let clientCasesURL = policyCasesURL.deletingLastPathComponent().appendingPathComponent("client-config.json")
let clientCasesFile = (try? Data(contentsOf: clientCasesURL))
    .flatMap { try? JSONSerialization.jsonObject(with: $0) } as? [String: Any]
require(clientCasesFile != nil, "the shared client cases are readable at \(clientCasesURL.path)")
let sharedCommandPath = clientCasesFile?["command_path"] as? String ?? ""
let sharedToken = clientCasesFile?["token"] as? String ?? ""
let mergeCases = clientCasesFile?["merges"] as? [[String: Any]] ?? []
require(!mergeCases.isEmpty, "the shared merge cases are not empty")
for (index, mergeCase) in mergeCases.enumerated() {
    let caseName = mergeCase["name"] as? String ?? "unnamed"
    let configURL = FileManager.default.temporaryDirectory
        .appendingPathComponent("torromail-contract-merge-\(index)", isDirectory: true)
        .appendingPathComponent("config.json")
    try? FileManager.default.removeItem(at: configURL.deletingLastPathComponent())
    let existing = mergeCase["existing"] as? String
    if let existing {
        try? FileManager.default.createDirectory(
            at: configURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try? Data(existing.utf8).write(to: configURL)
    }
    let sharedClient = MCPClient(
        id: "cursor",
        displayName: "Cursor",
        setup: (mergeCase["root_key"] as? String) == "servers"
            ? .serversJSON(configURL: configURL)
            : .mcpServersJSON(configURL: configURL)
    )
    let removing = (mergeCase["action"] as? String) == "remove"
    var failed = false
    do {
        if removing {
            try MCPClientSetup.remove(from: sharedClient)
        } else {
            try MCPClientSetup.add(to: sharedClient, commandPath: sharedCommandPath, token: sharedToken)
        }
    } catch {
        failed = true
    }
    if mergeCase["error"] as? Bool == true {
        require(failed, "shared client case fails as written: \(caseName)")
        require(
            (try? String(contentsOf: configURL, encoding: .utf8)) == existing,
            "shared client case leaves the file untouched: \(caseName)"
        )
        continue
    }
    require(!failed, "shared client case succeeds: \(caseName)")
    let written = (try? Data(contentsOf: configURL))
        .flatMap { try? JSONSerialization.jsonObject(with: $0) } as? NSDictionary
    require(written == mergeCase["expected"] as? NSDictionary, "shared client case: \(caseName)")
    require(
        MCPClientSetup.isConfigured(sharedClient) == !removing,
        "shared client case reads back as \(removing ? "not configured" : "configured"): \(caseName)"
    )
}
let sharedSnippets = clientCasesFile?["snippets"] as? [String: String] ?? [:]
let snippetFormats: [(String, MCPClientDescriptor.SnippetFormat)] = [
    ("mcp_servers_json", .mcpServersJSON),
    ("servers_json", .serversJSON),
    ("hermes_yaml", .hermesYAML),
    ("openclaw_json", .openClawJSON)
]
for (formatName, format) in snippetFormats {
    require(
        MCPClientSetup.configSnippet(commandPath: sharedCommandPath, format: format, token: sharedToken)
            == sharedSnippets[formatName],
        "shared snippet: \(formatName)"
    )
}
// Every client Rust knows, this app knows under the same id — access keys are
// stored by id, so a renamed client would lose its key on the other surface.
require(
    Set(MCPClientRegistry.catalog.map(\.id))
        == ["claude-desktop", "claude-code", "chatgpt", "gemini-cli", "cursor", "lm-studio",
            "vscode", "windsurf", "clawbot", "hermes", "other"],
    "the client catalog carries the ids the Rust catalog carries"
)

// MARK: - Provider discovery, shared with Rust

// `contracts/provider-discovery.json`: what an address implies about where to
// connect. The Rust side (crates/torromail-control/tests/provider_discovery.rs)
// runs the same file, so the wizard here and the one in the terminal lead an
// address to the same servers and the same login path.
func sharedAuthObject(_ auth: AuthPath?) -> Any {
    switch auth {
    case nil: return NSNull()
    case .oauth(let issuer): return ["kind": "oauth", "issuer": issuer.rawValue]
    case .password: return ["kind": "password"]
    case .appPassword(let setupURL): return ["kind": "app_password", "setup_url": setupURL.absoluteString]
    }
}

func sharedConfigObject(_ config: DiscoveredConfig?) -> Any {
    guard let config else { return NSNull() }
    return [
        "imap_host": config.imapHost,
        "imap_port": config.imapPort,
        "imap_security": config.imapSecurity.rawValue,
        "smtp_host": config.smtpHost,
        "smtp_port": config.smtpPort,
        "smtp_security": config.smtpSecurity.rawValue,
        "auth": sharedAuthObject(config.auth),
        "provider_label": config.providerLabel,
        "provider": config.provider.rawValue,
        "source": config.source
    ] as [String: Any]
}

let discoveryCasesURL = policyCasesURL.deletingLastPathComponent().appendingPathComponent("provider-discovery.json")
let discoveryCasesFile = (try? Data(contentsOf: discoveryCasesURL))
    .flatMap { try? JSONSerialization.jsonObject(with: $0) } as? [String: Any]
require(discoveryCasesFile != nil, "the shared discovery cases are readable at \(discoveryCasesURL.path)")

let discoverySections: [(String, ([String: Any]) -> Any)] = [
    ("domains", { sharedConfigObject(ProviderCatalog.lookup(domain: $0["input"] as? String ?? "")) }),
    ("mx", { sharedConfigObject(ProviderCatalog.fromMXHost($0["input"] as? String ?? "")) }),
    ("spf", { sharedConfigObject(ProviderCatalog.fromSPF($0["input"] as? String ?? "")) }),
    ("app_password_fallback", { discoveryCase in
        sharedAuthObject(
            ProviderCatalog.lookup(domain: discoveryCase["input"] as? String ?? "")
                .flatMap { ProviderCatalog.appPasswordFallback(for: $0) }
        )
    }),
    ("email_domains", { Autodiscovery.domain(of: $0["input"] as? String ?? "") as Any? ?? NSNull() }),
    ("autoconfig", { discoveryCase in
        sharedConfigObject(AutoconfigParser.parse(
            Data((discoveryCase["xml"] as? String ?? "").utf8),
            source: discoveryCase["source"] as? String ?? ""
        ))
    }),
    ("mailbox_names", { AccountMailboxList.displayName($0["input"] as? String ?? "") })
]
for (section, resolve) in discoverySections {
    let sectionCases = discoveryCasesFile?[section] as? [[String: Any]] ?? []
    require(!sectionCases.isEmpty, "the shared discovery section “\(section)” is not empty")
    for discoveryCase in sectionCases {
        let label = (discoveryCase["name"] ?? discoveryCase["input"]) as? String ?? "unnamed"
        let resolved = resolve(discoveryCase) as? NSObject
        let expected = discoveryCase["expected"] as? NSObject
        if resolved != expected {
            FileHandle.standardError.write(Data("Swift resolved “\(label)” to: \(String(describing: resolved))\n".utf8))
        }
        require(resolved == expected, "shared discovery case (\(section)): \(label)")
    }
}

// The writer is the server binary now. Without it there is no document — and
// "no document" must be an error the caller sees, never an empty or partial
// file the server would then read its rights from.
var missingWriterFailed = false
do {
    _ = try PolicyDocument.data(for: [], clients: [], executableName: "/nonexistent/torromail-mcp")
} catch {
    missingWriterFailed = true
}
require(missingWriterFailed, "a missing server binary fails the publication instead of inventing a document")

print("TorroMailKit control-surface contract passed")
