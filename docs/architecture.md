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

## MCP Client Compatibility

Automatic client setup is limited to assistants that execute local MCP servers
over stdio and publish a stable, user-owned configuration file or CLI. TorroMail
merges only its own entry into those configurations and never treats a file edit
as proof of a connection.

Google Antigravity is an automatic client with its own pairing and account
grants. Its current CLI (`agy`), desktop and IDE surfaces use the global
`~/.gemini/config/mcp_config.json`, with `command`, `args` and `env` under
`mcpServers`. TorroMail detects the Antigravity profile or CLI and merges only
`torromail` into this file. It does not change Gemini CLI's legacy
`~/.gemini/settings.json` or copy that client's key or grants. After connecting,
restart Antigravity and check `/mcp` in the CLI, or **Settings → Customizations →
Installed MCP Servers** in the desktop app. This targets the current configuration
documented in Google's [MCP configuration guide](https://antigravity.google/docs/mcp/)
and [Gemini CLI migration guide](https://antigravity.google/docs/cli/gcli-migration/).

OpenClaw is preferably configured through its native `openclaw mcp` registry.
OpenClaw 2.0 can install the desktop app without making the separate CLI
launcher visible to another GUI process, so TorroMail also has a file fallback:
it resolves `OPENCLAW_STATE_DIR` / `OPENCLAW_CONFIG_PATH` and surgically changes
only `mcp.servers.torromail` in `openclaw.json`. The JSON5 editor preserves
comments, trailing commas, existing servers, and unrelated settings; an
unreadable structure fails without being overwritten. When the CLI is present,
`mcp doctor torromail --probe --json` supplies the live transport check. In both
paths, the server's connection log remains the proof that OpenClaw accepted the
configured key.

Hermes is automatically configured in the native app and terminal control
surface. The terminal's single-client setup honors `HERMES_HOME` and otherwise
follows `~/.hermes/active_profile`. The native app's Hermes settings enumerate
the standard bot and every named profile independently of that selector. Users
select bots and share all or selected mail accounts with each one. New bots
start with no accounts; existing legacy grants migrate without being widened.
Each selected bot gets its own pairing identity, derived from the canonical
Hermes installation root and profile name, and its own key. Repeated setup keeps
readable keys stable. Disconnect revokes only that bot; legacy shared identities
are retained while older profiles may still use them.

The app reads Hermes Desktop's saved connection facts. An active SSH connection
is used directly; a URL backend is matched to a saved SSH host by hostname or
resolved address only when that match is unique. An unmatched remote backend
requires an explicit connection choice, never a silent local fallback. The
signed `TorroMailHermesControl` executable ships beside the MCP server and runs
on the selected computer, locally or over batch-mode SSH. Both computers need
an updated TorroMail app; this setup currently supports macOS targets. SSH uses
the existing user's key. The remote wrapper runs the signed worker through a
short-lived launch agent in the same user's existing macOS GUI session, because
an SSH security session cannot necessarily create login-keychain items. It
does not unlock the keychain or enable dialogs; a missing login session is an
actionable error. The agent and private request files are removed on completion.
Requests carry profile IDs and grants as bounded JSON
over stdin, not shell command text. Desktop login tokens are not imported, and
bot keys never return to the controlling computer. Account choices, keys and
the policy all belong to the selected TorroMail instance; local-only mail
accounts are not copied to a remote instance.

Setup changes `mcp_servers.torromail`, preserving custom tool filters and extra
environment variables while replacing stale transport facts and enabling the
server. The policy override environment variable is removed so the entry uses
this instance's policy. Reads parse YAML and compare the key inside that exact
entry, rather than finding an unrelated key elsewhere in the file. Ordinary block YAML edits
preserve the surrounding settings, server definitions, and comments; flow
mappings and aliases use a semantic YAML rewrite, which can change formatting
and comments. Invalid YAML, duplicate keys, and an invalid server map fail
without overwriting the file. Writes are atomic, private, and follow config
symlinks. The interactive discovery and selection prompts in `hermes mcp add`
are why setup uses the [documented YAML configuration](https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp).
The helper serializes setup with native policy publication using a control
lock. It updates the separate account-grant store, policy allowlist and an
existing terminal pairing file together; failure restores the affected files
and newly changed key. A connection test performs `initialize` and an
authenticated `mail_list_accounts` call with the configured key, rather than
treating open `tools/list` discovery as proof of access. A failed test rolls
setup back. Batch results remain per bot, so a damaged profile cannot overwrite
its siblings. Neither listing bots nor testing access runs a Hermes agent.
Hermes can load watched configuration changes automatically. A running session
that still uses the previous key needs `/reload-mcp` or a fresh session; the
setup probe verifies the server configuration, not a live Hermes conversation.
Key changes persist a per-bot reload notice. Only a subsequent attributed
client handshake clears it; setup probes and repeated setup keep it visible.

The official Grok Bot desktop app is intentionally not listed as a TorroMail
client. Grok Bot runs its tools on a persistent cloud computer and accepts
custom MCP connectors by public HTTP URL; it does not load a local stdio server
from the desktop app's Application Support directory. Exposing TorroMail through
a tunnel would cross the local-only mail boundary and require a new authenticated
Streamable HTTP transport, lifecycle, and consent design. Until that exists, an
"automatic" Grok Bot card would claim a connection that cannot work.

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

## Shared Accounts Per Client

The client settings expose **Shared accounts** (German: **Freigegebene
Konten**): **All accounts** includes accounts added later; **Selected accounts**
is an explicit set of account IDs. Selections may overlap between clients, and
an empty selection grants no access. Account read, write and send permissions
still apply on top of these grants.

The app persists the choices in `client-account-access.json` beside the policy
document, independently of access keys, so reconnecting or renewing a key
preserves the selection. On the first migration, existing readable client
pairings receive all accounts. New clients start with an empty selection.
The app's own pairing retains all accounts for setup and connection checks.
Malformed sharing settings are reported rather than replaced with all access.

Each published client carries one of these explicit policy fields:

```json
{"account_access": {"mode": "all"}}
```

```json
{"account_access": {"mode": "selected", "account_ids": ["work", "shared"]}}
```

A legacy client entry without `account_access` retains all accounts. A present
but invalid value fails closed; `null` does not mean all accounts. Pairing
itself remains unchanged, including support for hand-managed documents without
`clients`.

The server identifies the client by its token hash and filters accounts before
metadata, cache, credentials or connection access. `mail_list_accounts` and
aggregate cache status show only shared accounts. Requests for unshared IDs
receive the same account-not-found error as unknown IDs. Every subsequent tool
call reads current grants, including cached reads, pooled connections, search
refinement and execution of prepared actions. Account checks, cache rebuilds
and startup health sweeps use the same filter. Changing account sharing needs
no client restart. It does not retract content already returned to a client.

## Search And Cache

Each account has one cache decision, spoken in the language of the read
permissions: `off`, `headers` (Betreff & Absender), `bodies` (Ganze
E-Mails), or `attachments` (E-Mails & Anhänge). The effective level is
capped by the account's read permission — nothing rests on disk that no
assistant may read — and lowering either triggers `--trim-cache`. There are
no separate index switches: the FTS index always covers exactly what is
stored (subjects and senders always, body text and attachment *filenames*
from `bodies` up; file contents never).

The store is one SQLite database per account (`cache/<account>.sqlite`
beside the policy document, WAL mode — every MCP client spawns its own
server process) with an FTS5 index; retained attachment files live in the
`attachments/` tree from the download path. It fills write-through: what
search and read tools fetch anyway is kept at the effective level, and rows
are only valid under the UIDVALIDITY they were written with. Cached rows
serve repeated reads (`from_cache: true`); `mail_search` merges local index
hits into the live answer (`source: "live"`), and when the server is
unreachable the index answers alone (`source: "cache_only"`). The empty
browse query never consults the cache — recency is the server's to answer.
Only ids of the `mailbox/uid` shape are cached, which keeps fixture data
out by construction.

`torromail-mcp --rebuild-cache <account>` wipes and prefetches (newest 500
header rows per readable mailbox in one batched `UID FETCH` per chunk, the
newest 100 INBOX bodies when the level allows), emitting one JSON progress
line per batch for the app's progress bar. `mail_get_cache_status` reports
the chosen and effective level plus real counts and bytes. Every search
creates a reusable `result_set_id`; refinements operate on the prior set
before broadening back to provider search.

## Risky Actions

Send, move, and delete flows create pending actions with a preview, affected message count, account identity, tool call name, and expiration. Confirmation is single-use and expires by TTL.

Sending re-checks the effective account policy when the prepared action is
confirmed. A session draft is bound to the account that created it, so it
cannot be submitted through another account. Send previews expose attachment
names, media types, and byte sizes, but never their encoded content.

## Outgoing Attachments

`mail_create_draft` composes the text body and all attachments atomically, then
appends the complete RFC 5322 message with the `\Draft` flag. The destination
comes from the server's `\Drafts` mailbox attribute in `LIST`,
`LIST (SPECIAL-USE)`, or `XLIST`. If no attribute is reported, TorroMail uses
an existing drafts folder with a common or localized name, using the server's
hierarchy delimiter. With separate per-account `allow_create_drafts_mailbox`
consent (default false), it creates a missing Drafts folder under the personal
IMAP namespace, using CREATE-SPECIAL-USE when offered, then verifies LIST and
SELECT before use. Without NAMESPACE it uses the empty-name LIST hierarchy root;
unknown namespaces fail with an actionable setup error. With
attachments the message is `multipart/mixed`; every binary
part uses base64 transfer encoding and an RFC 2231 UTF-8 filename.

The same lookup applies to Sent, Archive, Junk, and Trash using their IMAP
special-use attributes and existing common folder names. An account can
override each of these five roles in the setup app by selecting an existing,
selectable folder from the server's live list. The server checks that exact
choice again when the action runs. `INBOX` is reserved by IMAP and has no
manual mapping. Automatic Drafts and Sent creation each have separate account
consent (`allow_create_drafts_mailbox` and `allow_create_sent_mailbox`, both false
by default); Archive, Junk and Trash require existing folders. A broken explicit mapping is reported,
never replaced automatically.

Draft operations use an optional `idempotency_key`, scoped to account and paired
client. Repeating it with different contents is rejected. Without a key,
identical MIME and Bcc recipients derive the same identity; intentional identical
new drafts need a fresh key. Before APPEND, private metadata under `draft-state/`
records a content hash and destination. An account file lock serializes concurrent
MCP processes; synced atomic checkpoints survive a process crash. The GUI-owned
policy is never rewritten by the server. Successful automatic folder mappings
persist there too and are validated on the next connection.

Every real draft result requires a Message-ID search, a single matching UID,
the Draft flag, and a full MIME fetch identical to the composed message. Retries
of uncertain attempts reconcile by identity and cannot APPEND again. If the
message remains invisible, TorroMail reports `draft_storage_uncertain`; the user
must inspect the server before deliberately starting a fresh operation. A
positively refused APPEND permits retry after quota or permission repair.
JSON-RPC errors retain code -32000 and add `error.data.reason` for draft setup
and storage failures. Draft audit entries contain counts, never recipients,
subject, filenames, body, or attachment bytes.

`mail_create_draft` defaults to `storage: "imap"` for compatibility. With
`storage: "local"` it composes and persists the exact MIME, envelope (including
Bcc), body and attachment summaries without opening IMAP or creating a server
folder. Both modes require the existing draft permission. Draft IDs and
`idempotency_key` ownership survive MCP restarts and are scoped to account and
client. A changed payload under the same key is rejected. The local snapshot
is immutable: editing an IMAP draft does not edit the payload TorroMail will
submit; use a new draft and obtain a new approval for changed content.

SMTP and Sent-copy storage are separate outcomes. Submission checks the current
client's account grant, the originating client's grant for GUI approval, the
send permission, approval code/expiry, sender and content fingerprint, and SMTP
configuration. It performs **no IMAP operation before SMTP**. The per-operation
OS lock remains held through submission. A synced atomic journal checkpoint is
written before the attempt and again immediately after the final DATA result.
`submission_status` is `accepted`, `not_accepted` or `unknown` after an attempt;
acceptance means SMTP's final 250 after DATA, **not recipient delivery**. Repeated
confirmation returns the same operation's outcome. It cannot send again. A
process that dies with `submitting` on disk recovers as `unknown`, even if the
server accepted DATA: SMTP has no general exactly-once guarantee, and Message-ID
alone does not prevent delivery twice. Unknown operations never retry SMTP.
Inspect the provider before explicitly creating a new draft with a new key.

The GUI and MCP both use `send_service`; the app's `--send-action` bridge lists,
rejects and confirms the same persistent operations. The GUI preview shows the
sender, full envelope, subject, body and attachment summaries. Send approvals
retain the five-minute TTL. Move/delete preparations remain session-local.

Each account chooses `sent_copy_strategy`: `imap` (the migration/default),
`provider` (the provider files its own copy) or `none` (no extra copy).
`sent_copy_status` is `saved`, `pending`, `provider` or `skipped` after acceptance;
before acceptance it is `not_applicable`. IMAP connection, lookup, creation and
APPEND failures affect only the copy. A missing automatically selected Sent
folder may be created using the shared special-folder algorithm and separate
account consent. A broken explicit mapping is never silently replaced.

Private `send-state/<policy-hash>/` files (directories 0700, files 0600 on Unix)
keep the exact MIME and envelope; no credentials are stored. This send spool is
separate from the permission-capped read cache and is retained until explicitly
removed. Audit logs contain only operation IDs, submission/copy outcomes and
counts. SMTP acceptance is committed before copy work. An `appending` checkpoint
records the original mailbox before APPEND; a retry verifies Message-ID **plus
exact MIME and the Seen flag** there. A tagged refusal can retry APPEND. A lost
APPEND acknowledgement can only reconcile; an invisible ambiguous copy stays
pending rather than risking a duplicate. A failure before APPEND's first search
is safe to retry. Automatic recovery runs at MCP startup and every 60 seconds
while a server is running. `torromail-mcp --retry-sent-copies` uses the same
pairing/account gates for an explicit copy-only recovery. Recovery never calls
SMTP, and revoked grants/permissions stop it.

Archive and Junk can be selected as `target_role` in `mail_prepare_move`;
soft delete resolves Trash automatically.

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

`mail_get_message` accepts `include_headers: true` to fetch every top-level
message header as an ordered `headers` array of `{ "name", "value" }` pairs.
Repeated fields (including `Received`) and folded value lines are preserved.
The array is absent by default and requires the "Message and attachments"
read level, including any mailbox-specific restriction. Full headers are read
live on demand and are not retained in the search cache.

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
