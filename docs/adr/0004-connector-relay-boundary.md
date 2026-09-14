# ADR 0004 — Connector relay boundary: no provider secrets in the desktop app

- **Status:** Accepted
- **Date:** 2026-08-21 (slot reserved 2026-05-03; written when connector work became real)
- **Bead:** `personal-cfo-p8x`
- **Enables:** `personal-cfo-x6dr` (connector-core), `personal-cfo-w3gh` (SimpleFIN adapter),
  `personal-cfo-nizb` (self-hosted relay design)
- **Related:** ADR 0002 (local encrypted vault), ADR 0003 (trust boundary), ADR 0008 (staged
  ingestion), ADR 0022 (parser isolation), ADR 0060 (connector strategy + sequencing)

## Context

Automated bank connections tempt an app toward two things this project must never do: ship a
provider API secret inside a distributed binary, and render a bank's auth flow inside the
app's own WebView. This app is open source — anything in the binary or the repo is public.
And provider guidance (Plaid explicitly) deprecates embedded-WebView link flows in favor of
hosted/system-browser auth.

Providers differ in *whose* secret the integration needs:

- **SimpleFIN-style user-token providers** need no developer registration at all. The user
  obtains a setup token themselves; the app claims it client-side and receives a read-only
  access URL. There is no project-owned secret anywhere.
- **BYO-credential providers** (Teller's per-developer mTLS certificate, Plaid's per-account
  keys under its 2026 Trial tier) have secrets — but they can be the *user's own*, created
  under the user's own provider account and stored only on the user's machine.
- **Server-secret providers** (Plaid/MX/Mastercard in their normal operating mode) require a
  confidential client: OAuth redirect handling, token exchange, webhooks — a server.

## Decision

### 1. Project-owned provider credentials never exist

The project registers for no aggregator accounts and holds no API keys, certificates, or
client secrets. Nothing in the repo, the binary, or the build pipeline embeds a provider
credential. This is stronger than "don't leak secrets": there are no project secrets to leak,
and no per-user metering liability for the project.

### 2. User-owned credentials live only in the user's encrypted vault or OS keychain

Setup tokens, access URLs, and any BYO credential (e.g. a user's own Teller mTLS cert) are
entered by the user, stored in the encrypted vault (or the OS keychain where the credential
is a private key), redacted from all logs per §6.6, and revocable from Settings. (Plaid's
2026 Trial tier makes user-owned Plaid keys technically possible, but per ADR 0060 Plaid is
planned via the relay tier; BYO-Plaid is not a committed path.)

This *renegotiates two clauses of the original bead AC* (`p8x`): (1) "provider API
credentials never live in the desktop app" becomes "**project-owned** provider credentials
never exist; **user-owned** credentials live only in the user's vault or OS keychain";
(2) Teller is reclassified from relay-required to BYO — research
(docs/research/simplefin-feasibility.md and ADR 0060) showed its credential is
per-developer and can be user-owned, making it a BYO variant of the user-token tier rather
than a relay-only provider. The boundary principle is unchanged — whose secret it is, not
which provider — so the correction is recorded here rather than inventing a relay
requirement the provider does not have.

### 3. Server-secret providers go through a relay the user controls

Three operating modes, each an explicit, separate opt-in; the app defaults to the first:

1. **Manual-only** — no connector, no relay. File import and manual entry. Always fully
   functional; sync is never load-bearing (§1.5).
2. **Self-hosted relay** — the user deploys the relay (`services/connector-relay`); it holds
   provider secrets, handles OAuth redirects/token exchange/webhooks, and returns only
   normalized, signed batches (design: `personal-cfo-nizb` and plan §8.4).
3. **Managed relay** — a future convenience deployment of the same relay. Nothing about its
   API surface may differ from self-hosted; it must remain swappable.

The relay's API surface stays minimal — link-session creation, account list, transaction
list, OAuth callback handling — and the relay never sees the vault key, the vault, or any
derived financial state. It is a fetch proxy, not a database.

"Provider secrets" here means provider **app** credentials (client id/secret), which are
necessarily relay-side. Whether per-item **access tokens** persist relay-side or live in the
desktop vault and are forwarded opaquely (the stateless variant sketched in
`personal-cfo-pxi.1`) is decided in the `nizb` relay design — this ADR constrains only that
no such secret is project-owned or app-embedded.

### 4. Provider auth renders in the system browser, never the app WebView

Hosted-Link-style flows open in the user's default browser; the app receives only the
resulting token via paste or local callback. The app WebView never hosts a bank login.

## Rejected alternatives

- **Bundle provider secrets with the app** — rejected: an open-source app exposes them to
  extraction the day it ships, and the project would carry usage liability for every fork.
- **WebView-embedded link flows** — rejected: deprecated by Plaid, and a credential-phishing
  surface inside our own window that contradicts ADR 0003's trust boundary.
- **Project-operated relay as the default sync path** — rejected at this stage: it creates a
  central service, an entity/liability requirement, and a privacy honeypot; it may return
  later as the *managed* deployment of the same self-hosted relay (mode 3), never as the only
  path.

## Consequences

- `personal-cfo-x6dr` (connector-core) is unblocked; the trait encodes this boundary (no
  provider concept leaks into the core schema, all output staged per ADR 0008).
- The relay design (`nizb`) inherits the app-credential/access-token distinction in §3 and
  must resolve the stateless-variant question explicitly.
- Bead `79mk` (Teller adapter) is re-scoped from relay-required to BYO-credential posture.
- The plan appendix (§21.2/§23) historically reserved different filenames for the 0003/0004
  slots; the bead graph (`p8x`) is authoritative and this file occupies the reserved 0004
  slot.

## Revisit if

- A provider critical to users offers **neither** a user-token/BYO flow **nor** a
  relay-compatible server flow.
- Open-banking regulation (Section 1033 / FDX) produces a direct consumer-credential path
  that makes the relay tier unnecessary for major providers.
- A BYO-tier provider stops offering user-ownable credentials (e.g., Teller requires
  organizational developer accounts) → that provider reverts to the relay tier and its beads
  are re-scoped.
