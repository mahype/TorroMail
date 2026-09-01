# Microsoft 365 Sign-In: What Microsoft Actually Requires

Status: current. Written 2026-09-01. Splits the second half out of issue #1.

## Why this cannot wait

Basic authentication for IMAP and POP in Exchange Online is **already gone**,
in every tenant, and cannot be brought back. Microsoft's own words: "Basic
authentication is now disabled in all tenants […] Now no one (you or Microsoft
support) can re-enable Basic authentication in your tenant." The switch-off ran
from 2021 through early 2023.

So for a Microsoft 365 mailbox there is no password TorroMail could ask for.
The fallback the wizard now offers after the detection fix — server fields and
a password — is an honest way out of a dead end, but for a real tenant it will
fail at the IMAP login every time. **OAuth is not the better path for Microsoft
365; it is the only path.**

Two more dates matter and are not yet passed:

- **SMTP AUTH** still accepts basic auth today. Microsoft's updated timeline
  disables it by default for existing tenants at the end of December 2026,
  makes it unavailable for tenants created after that, and announces a final
  removal date in H2 2027.
- **Personal accounts** (outlook.com, hotmail.de, live.com) went the same way
  ahead of the tenants; Microsoft's support article now only says basic auth is
  being removed from Outlook.com and points third-party clients at OAuth. Field
  reports put the cut-off at 16 September 2024, and a fresh consumer account
  today refuses basic auth outright.

Treat every Microsoft address — tenant or personal — as OAuth-only.

## What TorroMail already gets right

Checked line by line against Microsoft's protocol documentation:

| Requirement | Our code | Verdict |
| --- | --- | --- |
| IMAP scope `https://outlook.office.com/IMAP.AccessAsUser.All` | `OAuthIssuer.scopes` | exact match |
| SMTP scope `https://outlook.office.com/SMTP.Send` | same | exact match |
| `offline_access` for refresh tokens | same | present |
| `base64("user=" + user + "^Aauth=Bearer " + token + "^A^A")` | `imap_provider.rs`, `smtp.rs` | byte-for-byte |
| `AUTHENTICATE XOAUTH2 <blob>` / `AUTH XOAUTH2 <blob>` | both | correct, including the `+`/`334` continuation a rejected token triggers |
| Refresh-token rotation on every renewal | `torromail-oauth::refresh` | handled |
| `outlook.office365.com:993` / `smtp.office365.com:587` | `ProviderCatalog.microsoft` | correct |

Nothing in the protocol layer needs to change. What is missing is entirely on
the registration and setup side.

## What Microsoft requires that we do not have

### 1. The app registration

- Public client (mobile & desktop platform), **"Allow public client flows" =
  Yes**. No client secret; PKCE carries the exchange, which is what
  `OAuthService` already does.
- Supported account types: *any organizational directory **and** personal
  Microsoft accounts*. One registration then serves both a tenant mailbox and
  an outlook.com address through the `/common` endpoints already in
  `OAuthIssuer` — Microsoft states OAuth2 for IMAP/POP/SMTP is available for
  Microsoft 365 and Outlook.com alike.
- Redirect URI `msauth.com.torromail.app://auth`, registered under the
  iOS/macOS platform. Note the bundle id mismatch tracked in issue #1:
  `Info.plist` says `com.torromail.app`, `ProviderCatalog.bundleIdentifier`
  falls back to `de.torromail.app`.
- Delegated permissions under *Office 365 Exchange Online*:
  `IMAP.AccessAsUser.All`, `SMTP.Send`, plus `offline_access`, `openid`,
  `email`, `profile`.

### 2. Publisher verification — the real gate

This is the finding that changes the plan. Since November 2020, risk-based
step-up consent means **users cannot consent to a multi-tenant app that is not
publisher-verified**, when the app was registered after 8 November 2020 and
asks for anything beyond basic sign-in. `IMAP.AccessAsUser.All` is well beyond
basic sign-in. Without verification, every user outside our own tenant sees an
unverified-publisher warning and, in tenants using the recommended consent
policy, is blocked outright — each customer's admin would have to consent for
the whole organisation before anyone can add their mailbox.

Verification is free and quick *once the prerequisites exist*, and the
prerequisites are the slow part:

- A **Microsoft AI Cloud Partner Program** account (formerly MPN) with a
  verified Partner One ID, held as the organisation's partner global account.
- The app must be **registered from an Entra work or school account**. An app
  registered with a personal Microsoft account can never be publisher-verified
  — so the registration has to be created in the right tenant from the start,
  or redone later.
- A **publisher domain** on the app that matches the domain of the email used
  to verify the partner account, and it cannot be `*.onmicrosoft.com`.
- The person verifying needs Application Administrator (or Cloud Application
  Administrator) in Entra and Partner Admin or Account Admin in Partner
  Center, with MFA.

Practically: register the app from a `torromail.de` (or equivalent) Entra
tenant, not from a private Microsoft account, and start the partner account
now. Google's CASA assessment and this run in parallel, and both gate a
release that claims one-click sign-in.

### 3. Tenant-side prerequisites we do not control — but must explain

Even with a correct token, two admin settings can refuse the connection. Both
produce errors that look like our bug and are not:

- **IMAP disabled for the mailbox** (`Set-CASMailbox -ImapEnabled $true`).
- **SMTP AUTH disabled**, at tenant or mailbox level. This gates SMTP AUTH
  regardless of authentication type — an OAuth `AUTH XOAUTH2` fails just the
  same, with `535 5.7.139 Authentication unsuccessful, SmtpClientAuthentication
  is disabled for the Mailbox`. It bites personal accounts too: fresh
  outlook.com accounts have been reported with working OAuth IMAP and SMTP
  refusing on exactly this.
- **User consent disabled in the tenant**, which surfaces as `AADSTS65001` and
  needs an administrator, not another attempt.

These three deserve named errors in the app rather than "the server did not
accept these details". The wizard's current message sends the user hunting for
a wrong password when the actual fix is one PowerShell line by their admin.

## The order of work

1. Register the Entra app from a work/school tenant, wire the client id into
   `Info.plist`, add `CFBundleURLTypes`, fix the bundle-id fallback. Sign-in
   works immediately for our own tenant and for personal accounts.
2. Start publisher verification (partner account first — that is the long pole).
   Until it lands, tenant users may need an admin's consent.
3. Name the three failure modes above in the wizard and in the health check, in
   words that point at the admin setting rather than at the password.
4. Only then consider retiring the password fallback for Microsoft addresses.
   Today it is the escape hatch for a wrongly detected tenant (a domain whose
   MX is at Microsoft purely for inbound filtering); once sign-in works, an
   address that resolves to a real tenant should not be offered a password
   field at all.

## The alternative worth naming

Microsoft's own guidance nudges away from IMAP: "Move away from these protocols
as they don't enable full features." Microsoft Graph needs no SMTP AUTH, no
per-mailbox IMAP switch, and is not on a deprecation path. `docs/architecture.md`
already anticipates provider-native APIs behind the provider profile, and the
MCP contract would not change.

Not now — it is a second mail backend, and OAuth is a prerequisite either way.
But if the SMTP AUTH retirement in late 2026 turns into a support burden, Graph
is the exit, and the registration built above is the same one it would use.

## Sources

- [Deprecation of Basic authentication in Exchange Online](https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/deprecation-of-basic-authentication-exchange-online)
- [Authenticate an IMAP, POP or SMTP connection using OAuth](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth)
- [Updated Exchange Online SMTP AUTH Basic Authentication Deprecation Timeline](https://techcommunity.microsoft.com/blog/exchange/updated-exchange-online-smtp-auth-basic-authentication-deprecation-timeline/4489835)
- [Publisher verification overview](https://learn.microsoft.com/en-us/entra/identity-platform/publisher-verification-overview)
- [Outlook and other apps are unable to connect to Outlook.com when using Basic authentication](https://support.microsoft.com/en-us/office/outlook-and-other-apps-are-unable-to-connect-to-outlook-com-when-using-basic-authentication-f4202ebf-89c6-4a8a-bec3-3d60cf7deaef)
