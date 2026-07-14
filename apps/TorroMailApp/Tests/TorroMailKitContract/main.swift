import TorroMailKit
import Foundation

func require(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() {
        FileHandle.standardError.write(Data("Contract failed: \(message)\n".utf8))
        Foundation.exit(1)
    }
}

let model = TorroMailModel.preview()

require(
    model.sidebarItems.map(\.label)
        == ["Work", "Personal", "AI Clients", "General Settings", "Audit Log", "Diagnostics"],
    "accounts must be primary navigation before global settings"
)
require(
    model.sidebarGroups.map(\.label) == ["Accounts", "General"],
    "sidebar should visually separate mail accounts from general settings"
)
require(
    model.sidebarGroups[0].items.map(\.label) == ["Work", "Personal"],
    "account rows should live in their own sidebar group"
)
require(
    model.sidebarGroups[1].items.map(\.label) == ["AI Clients", "General Settings", "Audit Log", "Diagnostics"],
    "global settings should live below the account group"
)
require(
    model.selectedSidebarItem == .account("work"),
    "first account should be selected by default"
)
require(
    TorroMailAccountSection.allCases.map(\.label)
        == ["Overview", "Connection", "Permissions", "Search & Cache", "Mailboxes", "Pending Actions", "Advanced"],
    "account-specific settings must stay inside account detail sections"
)
require(
    model.generalSettings.startMcpServerWithApp,
    "MCP server should start with TorroMail by default"
)
require(
    model.generalSettings.mcpLifecycleSummary == "Starts with TorroMail",
    "MCP lifecycle summary should be user-facing"
)

let account = model.selectedAccount
require(
    account.permissions.summary == "Read body, Drafts, Mark, Move",
    "selected account should own permission state"
)
require(
    account.searchCache.summary == "Headers cached, body index off",
    "selected account should own search/cache state"
)
require(account.pendingActions.count == 1, "selected account should own pending actions")

let locator = MCPExecutableLocator(
    executableName: "torromail-mcp",
    workspaceRoot: "/repo"
)
require(
    locator.candidatePaths == ["/repo/target/debug/torromail-mcp", "torromail-mcp"],
    "MCP executable should be resolved from the dev workspace before PATH"
)
require(
    MCPServerStatus.notFound("torromail-mcp").label == "Executable not found",
    "MCP server status should expose readable lifecycle state"
)

print("TorroMailKit navigation contract passed")
