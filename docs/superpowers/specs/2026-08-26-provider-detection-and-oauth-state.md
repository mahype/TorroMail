# Provider Detection and the State of Google/Microsoft Sign-In

Status: current. Written 2026-08-26; fixes 1 and 3 implemented the same day,
fix 2 outstanding. Context: issue #1
("Microsoft 365 wird nicht automatisch erkannt", frank@staude.net).

## The problem

Two separate defects sit between "type an address" and "the account is
connected". They compound, which is why the reported symptom (a password and
server fields for a Microsoft 365 mailbox) looks like one bug and is two.

### 1. A hoster's default autoconfig outranks the MX record

`Autodiscovery.chain` runs Thunderbird's order: catalog, the domain's own
autoconfig, ISPDB, MX, SRV, probe. Step 2/3 wins before step 5 ever runs.

For `staude.net` that is exactly wrong. The web space is on Plesk, and Plesk
serves a generated client-config for every hosted domain:

    https://staude.net/.well-known/autoconfig/mail/config-v1.1.xml  → 200
    incomingServer imap → hostname staude.net, port 993,
                          authentication password-cleartext

The mail, however, is not there:

    dig MX staude.net → 10 staude-net.mail.protection.outlook.com

`ProviderCatalog.fromMXHost` already recognises that suffix and would return
the Microsoft entry with `.oauth(.microsoft)` — it is simply never reached.
The autoconfig hit is authoritative-looking and wrong, and the user gets a
password field that no Microsoft 365 tenant will ever accept.

This is not a one-domain quirk: Plesk, cPanel and most shared hosters publish
that file for every domain on the box, including the many whose mailboxes were
moved to Microsoft 365 or Google Workspace years ago.

### 2. No build can do OAuth at all

`ProviderCatalog` still carries placeholders:

    placeholderGoogleClientID    = "UNCONFIGURED.apps.googleusercontent.com"
    placeholderMicrosoftClientID = "UNCONFIGURED-microsoft-client-id"

`Info.plist` sets neither `TorroMailGoogleClientID` nor
`TorroMailMicrosoftClientID`, so `isConfigured` is false for both issuers in
every build shipped so far. What follows from that:

- **Gmail** degrades to the app-password path (`appPasswordFallback`), which
  works but demands 2FA and is not the "just enter your address" flow.
- **Microsoft has no fallback.** `appPasswordFallback` only answers for
  `.gmail`, so `effectiveAuth` stays `.oauth(.microsoft)`, the OAuth step
  renders "This build has no Microsoft client registered", `canAttemptLogin`
  is false, and the step shows no `manualDetails` disclosure. The wizard
  dead-ends: no sign-in, no manual escape, only Cancel.

So even once detection is fixed, `frank@staude.net` walks into a wall instead
of a password prompt. Detection must be fixed *and* a client registered, or
the user is worse off than today.

## What is already done

Worth stating, because the remaining work is smaller than it looks. The whole
token path exists and is tested:

- `OAuthService` — ASWebAuthenticationSession, PKCE (RFC 7636/8252), code
  exchange, `access_type=offline` + `prompt=consent` for Google, userinfo
  lookup that corrects the address and sender name, refresh-token-or-fail.
- `OAuthTokens.keychainPayload()` ↔ `torromail_oauth::TokenSet` — one agreed
  JSON shape, round-trip covered by tests on both sides.
- `torromail-oauth::refresh` — public-client refresh, Google's "no new refresh
  token" and Microsoft's rotation both handled, revoked grants told apart from
  network failures.
- `torromail-core` — `AUTHENTICATE XOAUTH2` for IMAP and `AUTH XOAUTH2` for
  SMTP.
- `TorroMailKit` writes `auth: xoauth2`, `token_endpoint` and `client_id` into
  the policy document for both IMAP and SMTP, and `torromail-mcp` parses them
  and renews tokens on its own while the app is closed.

Missing is the front of the funnel: recognising the provider, and having a
client id to sign in with.

## Fix 1: MX decides before a generic autoconfig — done

The chain now reads:

1. `ProviderCatalog.lookup(domain:)` — unchanged, no network.
2. **MX → `fromMXHost`**, moved up from step 5. Already implemented; it was
   only ever unreachable. Where the mail arrives is the domain owner's own
   statement, and a stale hoster XML does not get to overrule it.
3./4. The domain's own autoconfig, then ISPDB — as before, but now only for
   domains no hyperscaler claimed.
5. **SPF** (new): a `TXT` record containing `include:spf.protection.outlook.com`
   → Microsoft 365, `include:_spf.google.com` → Google Workspace. This catches
   tenants whose MX points at a filtering gateway (Proofpoint, Hornetsecurity,
   Mimecast), which the MX check silently misses.
6. SRV, then the 993 probe — unchanged.

The one non-obvious placement is SPF, and it is deliberate: it sits *below*
the autoconfig rather than next to the MX check. SPF says who may **send** for
a domain, which is a weaker claim than where mail arrives — a company relaying
its newsletters through Google is not a Workspace mailbox. So SPF only ever
rescues a domain that published nothing of its own, while the MX, which is a
statement about the mailbox itself, outranks everything.

Verified live against the reported address after the change:

    frank@staude.net → Microsoft 365 | imap outlook.office365.com:993
                     | auth oauth(microsoft) | source mx

`DNSResolver` gained a TXT query (type 16) alongside MX (15) and SRV (33) —
same `query` helper, but the rdata is a run of length-prefixed strings that has
to be concatenated, because a long SPF record is split at 255 bytes
(RFC 1035 §3.3.14).

Guard rail: a domain whose MX is at Microsoft only for inbound filtering
(Exchange Online Protection in front of a self-hosted server) is now offered
OAuth wrongly. No lookup can tell that case apart — which is why fix 3 is part
of this and not optional.

**Not implemented, and deliberately so: Microsoft's Autodiscover v2.**
`https://autodiscover-s.outlook.com/autodiscover/autodiscover.json/v1.0/<email>?Protocol=ActiveSync&RedirectCount=3`
answers `{"Protocol":"ActiveSync","Url":"https://outlook.office365.com/..."}`
for a tenant mailbox — verified against the issue's address — and an error
body otherwise. `Protocol=IMAP` is *not* supported there; ActiveSync is the
probe that works. It would confirm a tenant that neither MX nor SPF reveals,
but it sends the full address to Microsoft before the user has chosen a
provider. Worth adding only behind an MX/SPF hint, and only if a real case
turns up that the two records miss.

## Fix 2: register the two OAuth clients

Neither is a code change; both are blocking.

**Microsoft (Entra ID app registration)**
- Account types: "Accounts in any organizational directory and personal
  Microsoft accounts" — matches the `/common` endpoints already in
  `OAuthIssuer`.
- Platform: mobile & desktop, public client, no secret.
- Redirect URI: `msauth.com.torromail.app://auth` — note the bundle id in
  `Info.plist` is `com.torromail.app`, while `ProviderCatalog.bundleIdentifier`
  falls back to `de.torromail.app`. Align the fallback to the real id so a
  unit-test host cannot compute a redirect the registration does not know.
- Delegated permissions: `IMAP.AccessAsUser.All`, `SMTP.Send`,
  `offline_access`, `openid`, `email`, `profile` — exactly the scope list
  already in `OAuthIssuer.scopes`.

**Google (Cloud Console OAuth client)**
- Client type: iOS/desktop public client; redirect is the reversed client id,
  which `redirectScheme` already derives.
- Scope `https://mail.google.com/` is a *restricted* scope: app verification
  plus a CASA security assessment before the client leaves testing mode
  (100 test users cap, plus the "unverified app" interstitial the wizard
  already warns about). Long lead time — start it before it blocks a release.

Both client ids then go into `Info.plist` as `TorroMailGoogleClientID` /
`TorroMailMicrosoftClientID` (the override hook exists; only the keys are
missing). Add `CFBundleURLTypes` for both schemes as well: the callback works
without it because `ASWebAuthenticationSession` intercepts the scheme itself,
but an unregistered scheme is an unclaimed one, and any other app may take it.

Client ids are not secrets — PKCE is what protects the exchange — so they may
live in the repository the way every open-source mail client ships them.

## Fix 3: the wizard must never dead-end — done

- `effectiveAuth` now falls back for *any* unconfigured issuer: to an app
  password where the provider issues them (Google), and to the plain server
  fields otherwise. Microsoft issues none for a tenant with modern auth, so
  there was nothing to fall back to and the step simply had no way forward.
  The discovered `outlook.office365.com` / `smtp.office365.com` settings are
  already seeded into those fields.
- The OAuth step carries an "Enter the server details instead" button that
  sets `overrodeOAuth` and switches to the manual path. This is the escape
  hatch for the EOP case above: detection reads DNS, and only the person
  setting the account up knows whether the tenant is real.
- `isOAuthFallback` now also covers the password step, which says why it is
  asking — with the caveat that a Microsoft password only works while the
  tenant still allows basic auth.
- German strings added in `Resources/de.lproj/Localizable.strings`.

## Tests

There were none for `Autodiscovery` or `ProviderCatalog` — the contract target
did not mention them. `TorroMailKitContract` now covers:

- `fromMXHost` against `staude-net.mail.protection.outlook.com`,
  `aspmx.l.google.com`, `smtp.google.com`, and a hoster's own MX (which must
  claim nothing).
- `fromSPF` against the exact record the issue's domain publishes
  (`v=spf1 include:spf.protection.outlook.com -all`), its Google counterpart,
  a third party's include, and a non-SPF TXT record.
- `AutoconfigParser` against the Plesk XML above: it must parse *and* lose to
  a Microsoft MX.

`fromMXHost` and `fromSPF` had to become `public` for the contract target to
see them. The network-dependent steps stay out of CI; the classification
functions they feed are what gets tested.
