# SimpleFIN adapter feasibility (spike output for `personal-cfo-s558`)

- **Date:** 2026-08-21
- **Decision: GO** — proceed with `personal-cfo-w3gh` (SimpleFIN adapter) behind the
  `ConnectorAdapter` trait (`personal-cfo-x6dr`).
- **Method:** web research against current SimpleFIN Bridge docs and community sources, a
  code-level read of a shipping AGPL-3.0 implementation (Securo,
  github.com/securo-finance/securo, `backend/app/providers/simplefin.py` and its sync
  layer), and the Actual Budget / Firefly III integration docs. Time-boxed well under the
  bead's 1-week cap.

## Protocol and API surface

SimpleFIN is an open, read-only protocol; the **Bridge** (beta-bridge.simplefin.org, operated
on top of **MX** aggregation) is the hosted implementation users pay for. Surface is tiny:

1. **Claim:** the user generates a one-time **Setup Token** (base64 of a claim URL) in the
   Bridge UI and pastes it into the app. The app POSTs to the claim URL once and receives an
   **Access URL** with embedded HTTP Basic credentials. The token is single-use — a second
   claim returns 403 (surface as "setup token already used; generate a new one").
2. **Fetch:** `GET {access_url}/accounts?version=2` with `start-date`/`end-date` returns
   accounts, balances, and transactions (and holdings where the institution provides them).
   Responses carry an `errlist` of per-connection messages (`gen.auth`/`con.auth` mean the
   user must revisit the Bridge to reauthorize an institution).

That is the whole integration: no OAuth dance, no webhooks, no developer registration, no
project-owned secret. The claim and every fetch can run entirely inside the desktop app,
which is what makes SimpleFIN the only aggregator compatible with ADR 0004's user-token
tier (no project-owned secret anywhere, no relay).

## Token model and storage

The Access URL **is** the credential (basic-auth in the URL). It is stored as a vault secret,
never logged (§6.6 redaction; add its shape to the redaction corpus), shown nowhere in the
UI, and revocable locally (forget the URL); the Bridge's app management should also allow
revoking the connection server-side — confirm during `w3gh` implementation. Securo's
v0.13.9/v0.13.10 release notes show access URLs can go stale in practice (cause
unconfirmed) — design the "claim a fresh token" re-link path as a first-class flow, not an
error dead end.

## Coverage, limits, cost

- **Institutions:** ~16,000 US institutions via MX, including investment accounts with
  holdings (symbol/shares/market value confirmed in example data). Long-tail credit unions
  are MX-grade — better than Teller, comparable to Plaid.
- **Freshness:** data refreshes roughly **once per 24h** per institution (bank-dependent).
  Not real-time; the UI must say so ("updated daily") rather than implying live balances.
- **Rate limit:** ~**24 requests/day** per access URL. An on-vault-open sync plus a manual
  refresh button fits comfortably; a background poller would not be needed and must budget
  against this cap if ever added.
- **Query windows:** ≤**90 days** per request — historical backfill walks date ranges in
  90-day chunks. History depth at first link is institution-dependent (~2–6 months);
  deeper backfill arrives via the existing file importers, which the staged-ingestion dedupe
  layer reconciles against synced rows.
- **Cost:** paid by the **user**, not the project: $1.50/mo or **$15/yr** (up to 25
  institutions, 25 apps). A free demo token exists for development and CI-adjacent testing.
  The Bridge software is open source and self-hostable, which softens (not removes) the
  hosted-service dependency.

## ToS / ecosystem fit

The Bridge is explicitly built for third-party apps; Actual Budget and Firefly III both ship
SimpleFIN integrations under this model, and every OSS finance app offering US sync without
central infrastructure has converged on it. AGPL distribution poses no conflict: the app
ships no Bridge credentials and each user brings their own token.

## Risks / blockers

- **Single small operator.** The Bridge is a tiny operation with real bus-factor risk, and
  its MX relationship is load-bearing. Mitigation: the adapter sits behind the provider-
  neutral trait (x6dr), sync is never load-bearing (§1.5), and file import remains the
  universal fallback. This is an accepted risk, not a blocker.
- **Beta hostname.** Current canonical host is `beta-bridge.simplefin.org`; make the base
  URL a configurable constant so a host migration is a settings change, not a release.
- **Daily-batch expectations.** Users coming from Plaid-based apps expect real-time; copy
  and freshness badges must set the daily expectation explicitly (plan §8.5 patterns).
- No ToS, technical, or licensing blocker found. **GO.**

## Implementation notes for `w3gh` (from prior art)

Securo's AGPL-3.0 implementation is a working reference to *reimplement* (clean-room, per
ADR 0060 §5 — no code copied). From the adapter itself (`backend/app/providers/simplefin.py`):
single-use-token 403 handling, 90-day chunked walks with seen-id dedupe, and `errlist`
parsing (`gen.auth`/`con.auth` → needs-user-action), plus stale-access-URL recovery added in
their v0.13.9/v0.13.10. From their provider base and sync layer (`providers/base.py`,
`services/connection_service.py`, sync tasks): the typed error taxonomy (expired /
needs-user-action / rate-limited), the rate-limited-connections-stay-*healthy* status
machine, the 14-day incremental rewind, and 3-pass fuzzy reconciliation against manual
entries. Their credential handling (app-global Fernet key, silent decrypt-to-None) is the
anti-pattern our vault already avoids.
