# TorroMail 0.13.0 — draft folder verification

Verified locally on 2026-10-09. The version is recorded in the workspace
Cargo.toml and injected into the application and MCP initialization response.
Release notes are in [0.13.0.md](../releases/0.13.0.md).

## Result

An account without Drafts can create a verified draft, including an attachment,
after explicit per-account consent in **Special folders → Create a missing
drafts folder** (**Spezialordner → Fehlenden Entwurfsordner anlegen**).
New and migrated accounts default to no creation consent. Draft and send
permissions remain separate. The existing confirmation handshake is unchanged.

The implementation resolves explicit mappings and selectable special-use
folders, then known names using the actual hierarchy delimiter. Creation uses
the personal namespace, optionally CREATE-SPECIAL-USE, followed by LIST/SELECT
verification and best-effort subscription. Its account mapping persists outside
the GUI-owned policy document.

An account lock and durable attempt checkpoint coordinate parallel MCP
processes. Repeated operations use a stable Message-ID, scoped to account and
client identity, and verify the exact stored MIME plus the Draft flag. This
also survives MCP restart and renewal of the same client's access key.
An explicit idempotency key reused for different contents is rejected.

Draft error responses retain JSON-RPC code -32000 and include a structured
reason with setup or retry instructions. Local state contains hashes and
mailbox names; draft audit entries contain counts. Neither contains message
contents, attachment data or credentials.

## Commands actually executed

| Check | Result |
| --- | --- |
| `cargo test -q` | 391 passed, 0 failed, 4 opt-in tests ignored |
| `cargo build -p torromail-mcp` | Passed |
| `swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract` | Passed |
| `swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build` | Passed |
| `scripts/test-drafts-dovecot.sh` | Three successful real-server integration runs |
| `scripts/build-macos-app.sh` with version 0.13.0 and Developer ID signing | Passed, universal application |
| `scripts/build-dmg.sh`, then Developer ID signing of the image | Passed |
| Mounted-image smoke checks | Passed |
| `git diff --check` and shell syntax check of the integration runner | Passed |

Swift builds used `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`
(Xcode 26.5). The initially selected Command Line Tools lacked the SwiftUI
macro implementation required by their SDK; selecting full Xcode resolved that
build failure without changing application code.

The ordinary Rust run intentionally skipped the public-network TLS test, the
public DNS test, the real user keyring test, and the opt-in Dovecot test. The
Dovecot test was then run explicitly against all three configurations. No real
user keychain was modified by the integration tests.

## Acceptance coverage

| Scenario | Automated evidence |
| --- | --- |
| Existing selectable Drafts special-use folder | IMAP protocol contract |
| Explicit account mapping | IMAP protocol contract; shared policy contract |
| Known localized name without special-use | IMAP protocol contract, including `:` delimiter |
| Missing folder with and without creation consent | IMAP protocol contract and real Dovecot |
| Server refuses CREATE | IMAP protocol contract with tagged NOPERM refusal |
| Root, dot, colon and flat namespaces | Scripted protocol cases; real Dovecot for `/`, `INBOX.` and `Personal:` |
| Noselect candidate or newly created Noselect folder | IMAP protocol contracts |
| Concurrent CREATE attempts | Scripted refusal reconciliation and actual concurrent Dovecot sessions |
| Parallel MCP operations | Independent MCP servers sharing the account journal, against Dovecot |
| Attachment and unchanged To/Cc/Bcc recipients | Exact MIME verification; decoded attachment bytes `[0,1,2,3,255]`; SMTP sink envelope comparison |
| Lost APPEND acknowledgement and retry after restart | Fault injected only after actual Dovecot acceptance; exactly one stored draft |
| Default deduplication and intentional identical second draft | Real Dovecot message counts with absent and distinct keys |
| Renewed client access key | Same client id and draft id after key renewal; one Dovecot message |
| Saving refused or verification incomplete | Protocol tests for OVERQUOTA, absent UID, changed MIME and missing Draft flag |
| Sending still requires confirmation | Existing MCP contracts and real-server integration; prepare sends nothing, wrong code fails, correct code submits once |

The real IMAP server was the official Dovecot 2.4.2 image pinned to
`sha256:3fe299453447826694a42f7bb04b8ea12bc36bbc353fac154df05a0e7d6eaf48`.
The image supports Linux amd64 and arm64. Each test instance used disposable
storage and a loopback-only port. SMTP delivery ended in an in-process local
sink with reserved `example.invalid` recipients. No external message delivery
was attempted. The test prepared an existing Sent folder as a fixture for the
unchanged send prerequisite.

The integration runner is reproducible and is now included in GitHub CI. That
CI job was added locally; no remote CI run or published release is claimed.
See [the test](../../crates/torromail-mcp/tests/drafts_dovecot.rs) and
[the runner](../../scripts/test-drafts-dovecot.sh). Protocol behavior follows
[RFC 6154](https://www.rfc-editor.org/rfc/rfc6154.html); the server image is
specified in [Dovecot's Docker documentation](https://doc.dovecot.org/2.4.3/installation/docker.html).

## Artifact

- DMG: `dist/TorroMail-0.13.0.dmg` (approximately 16 MiB).
- Application: `dist/TorroMail.app`.
- Checksum file: `dist/SHA256SUMS.txt`.
- SHA-256: `cd16ee88bd8f1ade137ab9ca7cb3988c16a4dab5f8b70331ec1a707270a55e6b`.
- Test and build logs: `dist/verification/`.

The image and application are signed with the existing Developer ID, team
`2GA7DQ3P3Z`. Mounted-image checks verified the signature, version, German
consent string, Sparkle public key, and arm64/x86_64 architecture for the GUI,
CLI, MCP server and Hermes helper. The MCP catalog and initialization ran with
an isolated empty policy. Both CLI programs also started under x86_64.
The Hermes helper rejected an invalid request without accessing accounts.

## Remaining limits

1. The local artifact is signed but **not notarized**. No Apple notarization
   credentials were configured for this build; no stapled ticket is present.
   The full distribution smoke test requiring notarization therefore remains
   outstanding. The artifact has not been published or installed over the
   user's existing app.
2. Only Drafts may be created automatically. Sent, Archive, Junk and Trash
   still require existing selectable folders. For the reported account with
   only INBOX and INBOX.Trash, a Sent folder or explicit suitable mapping is
   also needed before the existing SMTP flow can succeed. No permission to
   create that folder was inferred from sending or drafts consent.
3. If an uncertain attempt has no visible message, the tool fails closed
   instead of risking a duplicate. The caller can retry the same key to
   reconcile; starting a fresh operation requires first inspecting the server.
4. Confirmation behavior was tested through the existing MCP handshake and
   local SMTP sink. A manual click-through of the running GUI and testing
   against the reported production account were not performed. Production
   TLS and credential handling were not bypassed; integration transport seams
   apply only to the explicit test harness.
5. Local Linux and Windows builds were not executed. Their existing CI checks
   remain responsible for platform validation. Durable metadata is retained
   to preserve idempotency; it does not currently have an automatic retention
   policy.
