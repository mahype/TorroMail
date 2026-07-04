# Account-Centered UI Implementation Plan

Status: Superseded historical plan.

This plan predates the current product clarification. DonnyMail is an MCP-first
local mail access layer, not a general-purpose mail client. The native macOS app
is limited to setup, consent, MCP lifecycle, status, audit, and diagnostics. Do
not use this plan to justify adding an inbox, message reader, thread browser, or
daily-use mail UI.

> **Historical note:** This was once an implementation plan for an
> account-centered configuration UI. Do not execute it as current work without a
> fresh product decision and replacement plan.

**Goal:** Refactor the macOS SwiftUI app so DonnyMail starts from email accounts, keeps per-account settings inside each account, and treats the MCP server as app-supervised.

**Architecture:** Extract testable app state into `DonnyMailKit`, keep the executable target as a thin SwiftUI shell, and render one account-centered navigation model. The Rust core and MCP catalog remain unchanged except for final verification.

**Tech Stack:** SwiftPM, SwiftUI, executable Swift contract runner, existing Rust workspace.

---

### Task 1: Add Testable Swift Navigation Model

**Files:**
- Modify: `apps/DonnyMailApp/Package.swift`
- Create: `apps/DonnyMailApp/Sources/DonnyMailKit/DonnyMailKit.swift`
- Create: `apps/DonnyMailApp/Tests/DonnyMailKitContract/main.swift`

- [ ] **Step 1: Write the failing tests**

```swift
import DonnyMailKit
import Foundation

func require(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() {
        FileHandle.standardError.write(Data("Contract failed: \(message)\n".utf8))
        Foundation.exit(1)
    }
}

let model = DonnyMailModel.preview()
require(model.sidebarItems.map(\.label) == ["Work", "Personal", "AI Clients", "General Settings", "Audit Log", "Diagnostics"], "accounts first")
require(model.selectedSidebarItem == .account("work"), "first account selected")
require(DonnyMailAccountSection.allCases.map(\.label) == ["Overview", "Connection", "Permissions", "Search & Cache", "Mailboxes", "Pending Actions", "Advanced"], "account sections")
require(model.generalSettings.startMcpServerWithApp, "MCP starts with app")
require(model.selectedAccount.pendingActions.count == 1, "account owns pending actions")
```

- [ ] **Step 2: Run test to verify it fails**

Run: `swift run --package-path apps/DonnyMailApp --scratch-path apps/DonnyMailApp/.build DonnyMailKitContract`

Expected: FAIL because `DonnyMailModel`, `DonnyMailAccountSection`, and related model types are not implemented in `DonnyMailKit`.

- [ ] **Step 3: Implement minimal model**

Create public/internal model types for sidebar items, account detail sections, accounts, permissions, search/cache, pending actions, AI clients, and general settings.

- [ ] **Step 4: Run test to verify it passes**

Run: `swift run --package-path apps/DonnyMailApp --scratch-path apps/DonnyMailApp/.build DonnyMailKitContract`

Expected: PASS.

### Task 2: Refactor SwiftUI Shell Around Account-Centered Navigation

**Files:**
- Modify: `apps/DonnyMailApp/Sources/DonnyMailApp/DonnyMailApp.swift`

- [ ] **Step 1: Replace global section sidebar**

Use `NavigationSplitView` with `model.sidebarItems`, where accounts appear first and global items appear below.

- [ ] **Step 2: Add account detail tabs**

Render `DonnyMailAccountSection.allCases` as a segmented control in the detail view and show Overview, Connection, Permissions, Search & Cache, Mailboxes, Pending Actions, and Advanced views inside the selected account.

- [ ] **Step 3: Add global views**

Render AI Clients, General Settings, Audit Log, and Diagnostics as separate global detail views.

- [ ] **Step 4: Run Swift build**

Run: `swift build --package-path apps/DonnyMailApp --scratch-path apps/DonnyMailApp/.build`

Expected: PASS.

### Task 3: App-Supervised MCP Lifecycle

**Files:**
- Modify: `apps/DonnyMailApp/Sources/DonnyMailKit/DonnyMailKit.swift`
- Modify: `apps/DonnyMailApp/Sources/DonnyMailApp/DonnyMailApp.swift`
- Test: `apps/DonnyMailApp/Tests/DonnyMailKitContract/main.swift`

- [ ] **Step 1: Extend the contract runner**

Add checks for `MCPExecutableLocator` candidate path resolution and `MCPServerStatus` labels.

- [ ] **Step 2: Implement the supervisor**

Add `MCPServerStatus`, `MCPExecutableLocator`, and `MCPServerSupervisor` in `DonnyMailKit`. The supervisor resolves `target/debug/donnymail-mcp` before falling back to `PATH`, starts the process when `startMcpServerWithApp` is enabled, and exposes status for the GUI.

- [ ] **Step 3: Wire the supervisor into the app**

Create `MCPServerSupervisor` as a `StateObject`, call `startIfNeeded` from the root task, and show status plus Start/Stop controls in General Settings.

- [ ] **Step 4: Run the Swift contract**

Run: `swift run --package-path apps/DonnyMailApp --scratch-path apps/DonnyMailApp/.build DonnyMailKitContract`

Expected: PASS.

### Task 4: Verify Repository

**Files:**
- No new files.

- [ ] **Step 1: Format Rust**

Run: `cargo fmt --all --check`

Expected: PASS.

- [ ] **Step 2: Test Rust**

Run: `cargo test`

Expected: PASS.

- [ ] **Step 3: Lint Rust**

Run: `cargo clippy --all-targets --all-features`

Expected: PASS without warnings.

- [ ] **Step 4: Run Swift contract**

Run: `swift run --package-path apps/DonnyMailApp --scratch-path apps/DonnyMailApp/.build DonnyMailKitContract`

Expected: PASS.

- [ ] **Step 5: Build Swift app**

Run: `swift build --package-path apps/DonnyMailApp --scratch-path apps/DonnyMailApp/.build`

Expected: PASS.
