# TorroMail Architecture

TorroMail is a local-first mail access layer for agents. It retrieves mail from
configured accounts and exposes policy-controlled capabilities through MCP. The
primary interface is the open MCP contract, not an end-user mail reader.

TorroMail is not a full mail client. The project must not add a human-facing
inbox, message reading surface, thread browser, or daily mail workflow UI unless
that product boundary is explicitly changed. The native macOS app is a
configuration and control surface: users manage accounts, permissions, cache,
OAuth, MCP clients, pending actions, audit logs, diagnostics, and MCP lifecycle
there. Files may exist internally, but they are not the user-facing
configuration path.

## Boundaries

- Rust core owns account state, provider abstractions, policy checks, cache policy, pending actions, search sessions, and MCP-facing behavior.
- SwiftUI app owns the macOS setup, review, status, and diagnostics flow. It is
  not responsible for browsing mail content.
- MCP owns assistant-facing tools for mail work, but not account administration.
- Secrets are referenced by account identity and stored through the platform secret store, not passed through tool parameters.

## Non-Goals

- No inbox UI for people to read mail.
- No message timeline, conversation view, or manual mail triage workflow.
- No general mail-client feature set such as rich mailbox management, manual
  compose workflows, contact management, calendar UI, or newsletter reading.
- No MCP tools that mutate account setup, secrets, OAuth state, or permissions.
- No hidden configuration paths that bypass the native setup and consent model.

## GUI-Only Operations

These operations must remain unavailable as MCP tools:

- Add, edit, or remove mail accounts.
- Add, rotate, or reveal credentials.
- Start or complete OAuth setup.
- Change account permissions.
- Enable local body cache, attachment cache, or full text indexing.

## MCP Surface

Read and search tools:

- `mail_search`
- `mail_refine_search`
- `mail_get_message`
- `mail_get_attachment`
- `mail_get_thread`
- `mail_list_mailboxes`

Direct write tools (no approval, gated by the mark permission):

- `mail_mark`

Prepared action tools:

- `mail_create_draft` — accepts optional MIME attachments as standard padded
  base64, never as a local path or URL.
- `mail_prepare_send`
- `mail_prepare_move`
- `mail_prepare_delete`
- `mail_confirm_action`

Read-only admin tools:

- `mail_list_accounts`
- `mail_get_policy`
- `mail_get_cache_status`

## Service Boundary

`torromail-core` owns the policy-checked mail path. `MailProvider` is the
trait fixture-backed tests and real IMAP retrieval share; `MailAccessService`
is the doorway MCP tools go through. Every read is authorized twice —
account-wide and again for the mailbox the data actually lives in — so
search results never contain hits from folders the permission rules block.
`torromail-mcp` routes JSON-RPC `tools/call` into that service. All catalog
tools are implemented. The mailbox listing only ever names folders the policy
grants something on.

The real IMAP path exists as a protocol core behind `ImapTransport`:
`ImapClient` speaks the smallest useful IMAP4rev1 subset (LOGIN, LIST,
SELECT, UID SEARCH/FETCH/STORE, literal parsing, PEEK-only body reads), and
`ImapMailProvider` implements the same `MailProvider` trait the fixtures use
— fully covered by scripted-transport tests without a network. Transports
share one generic `StreamImapTransport` over any duplex stream: plaintext
TCP for local development, and implicit TLS from `torromail-imap-tls`
(rustls with the bundled Mozilla roots; custom CAs arrive with platform
verification).

Secrets close the chain: the app stores passwords in the macOS keychain
under service `TorroMail`; the policy document carries connection facts and
the reference (`keychain://TorroMail/{account}`) — never the password. The
server resolves the reference itself via the Security framework, which asks
the user once to allow `torromail-mcp` until code signing makes it
promptless. Accounts with connection facts get a real TLS session per tool
call (no pooling yet); accounts without stay on fixture data. The app's
"Test Connection" button runs `torromail-mcp --check-account <id>` — the
same resolution, TLS and LOGIN the tools use, so a green dot means the real
path works. `SecretRef` redacts itself in any Debug output.

The permissions set in the app reach the server through an internal policy
document: the app publishes `~/Library/Application Support/TorroMail/policy.json`
(override: `TORROMAIL_POLICY_PATH`) whenever accounts change, and every
`torromail-mcp` instance — whether the app's supervisor or an MCP client
spawned it — reloads the document on each tool call, so a switch flipped in
the UI applies immediately. A corrupt document fails closed, accounts missing
from it are refused, and only when no document exists yet does the server fall
back to the product default (read and drafts).

## Search And Cache

The default cache policy persists metadata and headers only. Body cache, body indexing, and attachment indexing are opt-in per account or mailbox. Every search creates a reusable `result_set_id`; refinements operate on the prior set before broadening back to provider search.

## Risky Actions

Send, move, and delete flows create pending actions with a preview, affected message count, account identity, tool call name, and expiration. Confirmation is single-use and expires by TTL.

Sending re-checks the effective account policy when the prepared action is
confirmed. A session draft is bound to the account that created it, so it
cannot be submitted through another account. Send previews expose attachment
names, media types, and byte sizes, but never their encoded content.

## Outgoing Attachments

`mail_create_draft` composes the text body and all attachments atomically, then
appends the complete RFC 5322 message to the account's Drafts mailbox with the
`\Draft` flag. With attachments the message is `multipart/mixed`; every binary
part uses base64 transfer encoding and an RFC 2231 UTF-8 filename.

The MCP contract accepts `filename`, optional `media_type`, and
`content_base64`. It deliberately accepts neither filesystem paths nor URLs:
the paired client supplies bytes it can already access, while TorroMail does
not gain ambient file or network-reading authority. A draft may contain at
most 20 attachments, and the finished MIME message may contain at most 20 MiB.

```json
{
  "attachments": [
    {
      "filename": "angebot.pdf",
      "media_type": "application/pdf",
      "content_base64": "JVBERi0xLjQK..."
    }
  ]
}
```

## Incoming Attachments

`mail_get_message` lists a message's attachments (id, filename, media type,
decoded size, inline flag) from the "Full message" read level; the bytes need
"Message and attachments". `mail_get_attachment` writes the decoded file to
`attachments/<account>/<message>/<id>-<filename>` under the shared Application
Support directory and answers with the absolute path — clients never pass
destination paths, mirroring the rule on the outgoing side.
`include_content: true` additionally inlines base64 up to 2 MiB; downloads are
capped at 50 MiB. Files older than 24 hours are swept on server start until
the cache (spec phase 2) starts retaining them.

## Provider Roadmap

The initial adapter model targets IMAP/SMTP. Gmail, Microsoft, and JMAP are represented as provider profiles so OAuth/XOAUTH2 and provider-native APIs can be added without changing the GUI or MCP contract.
