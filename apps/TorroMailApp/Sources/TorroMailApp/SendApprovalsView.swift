import SwiftUI
import TorroMailKit

/// An approval preview is operational consent, not a general mail reader.
struct SendApprovalsView: View {
    @EnvironmentObject private var store: SendActionStore
    @EnvironmentObject private var model: TorroMailModel
    var accountID: String? = nil
    @State private var selected: SendApproval?
    private var actions: [SendApproval] { store.actions.filter { accountID == nil || $0.accountID == accountID } }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let notice = store.notice {
                Text(L(notice)).foregroundStyle(.secondary)
                Button(L("Dismiss")) { store.notice = nil }
            }
            ForEach(actions) { action in
                HStack {
                    VStack(alignment: .leading) {
                        Text(action.subject).fontWeight(.medium)
                        Text(action.recipients.joined(separator: ", ")).foregroundStyle(.secondary)
                        if !action.needsApproval { Text(statusText(action)).foregroundStyle(.orange) }
                    }
                    Spacer()
                    if action.needsApproval {
                        Button(L("Reject"), role: .destructive) { decide(action, approve: false) }
                        Button(L("Review and approve")) { selected = action }
                    }
                }
                .disabled(store.busy)
            }
        }
        .sheet(item: $selected) { action in
            VStack(alignment: .leading, spacing: 16) {
                Text(L("Review send")).font(.title2)
                ScrollView {
                    VStack(alignment: .leading, spacing: 12) {
                        LabeledContent(L("From"), value: action.from)
                        LabeledContent(L("Recipients"), value: action.recipients.joined(separator: ", "))
                        LabeledContent(L("Subject"), value: action.subject)
                        Text(action.body).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                        ForEach(action.attachments.indices, id: \.self) { index in
                            let attachment = action.attachments[index]
                            Text(verbatim: "\(attachment.filename) · \(attachment.size_bytes) bytes · \(attachment.media_type)")
                        }
                    }
                }
                Text(L("SMTP acceptance does not confirm delivery to the recipient."))
                    .font(.footnote).foregroundStyle(.secondary)
                HStack {
                    Spacer()
                    Button(L("Cancel")) { selected = nil }
                    Button(L("Approve")) { selected = nil; decide(action, approve: true) }.keyboardShortcut(.defaultAction)
                }
            }.padding(24).frame(width: 620, height: 480)
        }
    }
    private func decide(_ action: SendApproval, approve: Bool) {
        Task { await store.decide(action, approve: approve, executableName: model.generalSettings.mcpExecutable) }
    }
    private func statusText(_ action: SendApproval) -> String {
        switch action.submissionStatus {
        case "accepted": L("Accepted by SMTP; Sent copy pending.")
        case "submitting": L("Submission is in progress.")
        case "not_accepted": L("SMTP did not accept the message.")
        default: L("SMTP acceptance is unknown. TorroMail will not send again automatically.")
        }
    }
}
