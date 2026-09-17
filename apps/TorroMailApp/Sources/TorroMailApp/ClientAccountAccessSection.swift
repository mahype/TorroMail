import SwiftUI
import TorroMailKit

struct ClientAccountAccessSection: View {
    @EnvironmentObject private var model: TorroMailModel
    let clientID: String

    @State private var access: ClientAccountAccess = .selected([])
    @State private var loaded = false
    @State private var failed = false
    @State private var pendingAccess: ClientAccountAccess?

    var body: some View {
        Section {
            Picker(L("Account access"), selection: Binding(
                get: { access == .all },
                set: { save($0 ? .all : .selected(Set(model.accounts.map(\.id)))) }
            )) {
                Text(L("All accounts")).tag(true)
                Text(L("Selected accounts")).tag(false)
            }
            .disabled(!loaded)

            if case .selected(let ids) = access, loaded {
                ForEach(model.accounts) { account in
                    Toggle(isOn: Binding(
                        get: { ids.contains(account.id) },
                        set: { allowed in
                            var selected = ids
                            if allowed { selected.insert(account.id) }
                            else { selected.remove(account.id) }
                            save(.selected(selected))
                        }
                    )) {
                        VStack(alignment: .leading) {
                            Text(verbatim: account.name)
                            Text(verbatim: account.email).foregroundStyle(.secondary).scaledFont(.caption)
                        }
                    }
                    .toggleStyle(.checkbox)
                }
                if ids.intersection(Set(model.accounts.map(\.id))).isEmpty {
                    Text(L("No accounts are shared with this client."))
                        .foregroundStyle(.secondary)
                }
            }

            if failed {
                Text(L("Could not apply account sharing. Please try again."))
                    .foregroundStyle(.red)
                Button(L("Try again")) {
                    if let pendingAccess { save(pendingAccess) }
                    else { load() }
                }
            }
        } header: {
            Text(L("Shared accounts"))
        } footer: {
            Text(access == .all
                ? L("This client can access all accounts, including accounts added later. Account permissions still apply.")
                : L("Only selected accounts are shared. New accounts require a separate selection. Account permissions still apply."))
        }
        .onAppear { load() }
        .onChange(of: clientID) { _, _ in load() }
    }

    private func load() {
        do {
            let grants = try ClientAccountAccessStore.load(legacyClientIDs: MCPClientKeyStore.pairings().map(\.clientID))
            access = grants[clientID] ?? .selected([])
            loaded = true
            failed = false
            pendingAccess = nil
        } catch {
            loaded = false
            failed = true
            NSLog("TorroMail: account sharing read failed: %@", error.localizedDescription)
        }
    }

    private func save(_ updated: ClientAccountAccess) {
        do {
            let pairings = MCPClientKeyStore.pairings()
            let previous = try ClientAccountAccessStore.load(legacyClientIDs: pairings.map(\.clientID))
            var grants = previous
            grants[clientID] = updated
            try ClientAccountAccessStore.save(grants)
            do {
                try PolicyDocument.publish(accounts: model.accounts, clients: ClientAccountAccessStore.applying(grants, to: pairings))
            } catch {
                // A failed policy write must not look like a successful grant
                // change. Restore persisted settings before reporting failure.
                try ClientAccountAccessStore.save(previous)
                throw error
            }
            access = updated
            failed = false
            pendingAccess = nil
        } catch {
            pendingAccess = updated
            failed = true
            NSLog("TorroMail: account sharing update failed: %@", error.localizedDescription)
        }
    }
}
