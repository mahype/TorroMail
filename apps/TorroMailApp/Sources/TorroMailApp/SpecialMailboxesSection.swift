import SwiftUI
import TorroMailKit

/// Account setup only: the app chooses exact existing IMAP folders and the
/// server enforces them later. Assistants never receive a write destination
/// parameter for Drafts, Sent, or Trash.
struct SpecialMailboxesSection: View {
    @Binding var account: MailAccount
    let executableName: String

    @State private var mailboxes: [String] = []
    @State private var loading = false
    @State private var loadFailed = false
    @State private var requestVersion = 0

    var body: some View {
        Section {
            Toggle(L("Choose special folders manually"), isOn: $account.specialMailboxes.manual)
            if account.specialMailboxes.manual {
                if loading {
                    ProgressView(L("Loading folders…"))
                } else if loadFailed {
                    HStack {
                        Text(L("Could not load folders. Check the connection and try again."))
                            .foregroundStyle(.secondary)
                        Spacer()
                        Button(L("Retry")) { loadMailboxes() }
                    }
                }
                mailboxPicker(L("Drafts"), selection: $account.specialMailboxes.drafts)
                mailboxPicker(L("Sent"), selection: $account.specialMailboxes.sent)
                mailboxPicker(L("Archive"), selection: $account.specialMailboxes.archive)
                mailboxPicker(L("Junk"), selection: $account.specialMailboxes.junk)
                mailboxPicker(L("Trash"), selection: $account.specialMailboxes.trash)
                if !loading && !loadFailed && hasMissingSelection {
                    Label(L("A selected folder is no longer available on this server."), systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                }
            }
        } header: {
            Text(L("Special folders"))
        } footer: {
            Text(L("TorroMail recognizes these folders automatically. Choose an existing folder only when needed."))
        }
        .onAppear {
            if account.specialMailboxes.manual { loadMailboxes() }
        }
        .onChange(of: account.specialMailboxes.manual) {
            if account.specialMailboxes.manual { loadMailboxes() }
        }
        .onChange(of: account.id) {
            requestVersion += 1
            loading = false
            mailboxes = []
            if account.specialMailboxes.manual { loadMailboxes() }
        }
    }

    private var hasMissingSelection: Bool {
        let selected = account.specialMailboxes
        return [selected.drafts, selected.sent, selected.archive, selected.junk, selected.trash]
            .contains { !$0.isEmpty && !mailboxes.contains($0) }
    }

    private func mailboxPicker(_ title: String, selection: Binding<String>) -> some View {
        Picker(title, selection: selection) {
            Text(L("Automatic")).tag("")
            ForEach(pickerNames(including: selection.wrappedValue), id: \.self) { name in
                Text(AccountMailboxList.displayName(name)).tag(name)
            }
        }
    }

    private func pickerNames(including chosen: String) -> [String] {
        var names = mailboxes
        if !chosen.isEmpty && !names.contains(chosen) {
            names.insert(chosen, at: 0)
        }
        return names
    }

    private func loadMailboxes() {
        guard !loading else { return }
        loading = true
        loadFailed = false
        requestVersion += 1
        let version = requestVersion
        let accountID = account.id
        let executable = executableName
        Task.detached(priority: .userInitiated) {
            let result = AccountMailboxList.load(accountID: accountID, executableName: executable)
            await MainActor.run {
                guard version == requestVersion else { return }
                switch result {
                case let .success(names):
                    mailboxes = names
                    loadFailed = false
                case .failure:
                    loadFailed = true
                }
                loading = false
            }
        }
    }
}
