# ADR 0060 — Connector strategy: SimpleFIN first, in the Launch gate, manual stays first-class

- **Status:** Accepted (owner decision, 2026-08-21)
- **Date:** 2026-08-21
- **Bead:** `personal-cfo-uf3a` (decision record); amends gate `personal-cfo-2owr`, epic `personal-cfo-pxi`
- **Amends:** plan §5.4 (connectors were MVP 3 / post-launch), Launch gate AC (`2owr`)
- **Builds on:** ADR 0004 (relay boundary), ADR 0008 (staged ingestion), ADR 0014 (Money
  Inbox), docs/research/simplefin-feasibility.md (`s558`, GO)

## Context

Onboarding is the weakest step of the product story: today a new user must export files from
every institution and re-import them on a cadence, forever. The owner reviewed Securo (an
AGPL-3.0 self-hosted finance app) and asked what direct institution linking would take. A
research pass (2026-08-21) established: SimpleFIN Bridge is the only US aggregator whose
whole flow — claim and fetch — runs client-side with no project registration, no embedded
secret, and no relay; it costs the user ~$15/yr, covers ~16k institutions via MX (including
investment holdings), and is the proven choice of Actual Budget and Firefly III. Teller
requires per-developer mTLS credentials (viable only as bring-your-own-account), Plaid still
assumes a confidential server client, MX/Finicity direct are enterprise-only, and Section
1033/FDX is enjoined and being rewritten — no individual access path in 2026.

The plan (§5.4) had deferred all connectors to post-launch MVP 3 on the manual-first
principle (§1.5). The counterweight is that manual-only onboarding is a real adoption
barrier for a public launch.

## Decision

### 1. SimpleFIN is the first connector, via the user-token tier

No relay, no project credentials: paste setup token → client-side claim → access URL stored
as a vault secret (so backup/restore round-trips the connection), with a first-class re-link
flow (claim a fresh setup token) for stale access URLs and single-use-token 403s — part of
`w3gh`/`ul5d`, not an error dead end. Constraints are accepted: the daily-refresh cadence
must be visible as freshness copy ("updated daily", per plan §8.5 patterns); the ~24 req/day
budget and 90-day query windows constrain the sync engine (chunked backfill, debounced
refresh), not the copy. Feasibility, limits, and risks:
docs/research/simplefin-feasibility.md.

### 2. Connectors enter the Launch gate — a deliberate reversal of §5.4's deferral

Owner decision 2026-08-21. The Launch gate (`2owr`) gains: **a working SimpleFIN sync ships
before public launch.** Launch-gated scope is exactly four gate-tracked beads: `x6dr`
(connector-core + `ConnectorAdapter` trait — the `1s2b` mock is folded under its AC), `w3gh`
(SimpleFIN adapter), `ul5d` (connection health surface), `zfyo` (connector-error Money Inbox
item). Gate verification: an end-to-end run against the SimpleFIN demo token claims a setup
token, syncs accounts and transactions through staged ingestion (ADR 0008), and commits via
Money Inbox — proven by an integration test or scripted drill named in `w3gh`'s AC. Teller
(BYO account, `fhz8`/`79mk`), Plaid-class relay work (`nizb` et al.), and
institution-specific importers stay post-launch. The known cost — launch moves out by
roughly the length of one focused arc (~3–4 PRs) — was weighed and accepted.

### 3. Manual import remains a first-class path, now with a cadence

Sync is opt-in convenience; every feature keeps working manual-only (§1.5 unchanged, and the
gate scope above must not make sync load-bearing). For manual users the app takes over the
ritual's memory: per-institution **import-freshness reminders** — a Money Inbox item keyed to
the last committed `source_batch` per account ("It's been 21 days since your last Chase
import"), mirroring the shipped `stale_balance` mechanism, with per-institution cadence
settings (`o7w0`); an OS-notification trigger follows once notification infrastructure
(`3yc7`/`9ii`) ships (`k3hd`). Import dialogs grow per-institution export guidance so the
ritual is documented, not tribal knowledge (`rfsc`). These are not launch-gated.

### 4. Sync scheduling reuses the on-vault-open pattern

Sync runs on vault open (like forecast persist/actualize/backtest per ADR 0026) plus a
manual refresh button — no job runtime, no background daemon. This keeps typical use far
below the ~24 req/day cap; sync debounces (skipped when the last success is recent), and a
rate-limit response is a *healthy* connection state per the §5 error taxonomy, so hitting
the cap degrades gracefully. Connector output enters the standard staged-ingestion pipeline
(ADR 0008): staged rows, layered dedupe against prior file imports, Money Inbox triage,
idempotent commit, full provenance. No SimpleFIN concept leaks into the core schema (§8.3).

### 5. Learn Securo's hard-won reconciliation behavior — as a clean-room reimplementation

Reimplement in Rust using Securo (AGPL-3.0) as a *reference*, with attribution as courtesy
in a NOTICE entry — **no Securo code is copied**. Two reasons this is a rule, not a
preference: today's repo LICENSE is still proprietary (AGPL text lands at `fkt5` fork time),
so verbatim AGPL code cannot enter pre-fork builds; and ADR 0043's dual-licensing position
rests on the owner being sole copyright holder of every line — third-party AGPL code would
erode that permanently. The behaviors worth reimplementing: the typed provider-error
taxonomy (expired / needs-user-action / rate-limited-is-healthy), 3-pass reconciliation
(exact provider-id with pending→posted promotion; fuzzy-merge of synced transactions into
pre-existing manual entries; pending/posted twin collapse), and the 14-day incremental
rewind. Do **not** copy its credential posture (app-global derived key, silent decrypt
failure) — credentials live in the encrypted vault per ADR 0004, redacted per §6.6, failing
loudly.

## Rejected alternatives

- **Teller first** — per-user developer accounts + mTLS key handling is onboarding friction,
  and no investment data. Kept as a post-launch power-user option.
- **Plaid first (BYO keys, 2026 Trial tier)** — still architecturally server-shaped, 10-item
  cap, heaviest privacy optics for a privacy-first brand. Relay-tier work, post-launch, only
  on demonstrated demand.
- **Wait for Section 1033 / FDX** — rule enjoined and under rewrite; no 2026 path for an OSS
  project. Watchlist only (`gqmx`).
- **No aggregation at all (Ghostfolio's stance)** — was the de-facto plan; rejected by the
  owner because manual-only onboarding is a real adoption barrier for launch.
- **Keep connectors post-launch (the plan §5.4 status quo)** — the prior plan default and
  this planning session's initial recommendation; overridden by the owner 2026-08-21:
  onboarding value justifies the launch delay.

## Consequences

- Launch gate AC (`2owr`) gains item (8) — SimpleFIN sync — with `tracks` edges to `x6dr`,
  `w3gh`, `ul5d`, `zfyo`; all four bumped to P1. `w3gh`'s AC gains the demo-token
  end-to-end verification and its token storage is reconciled to the vault-secret model
  (was: OS keychain), per §1.
- ADR 0004 (relay boundary) written and `p8x` closed; SimpleFIN research
  (docs/research/simplefin-feasibility.md) closes `s558` with GO.
- New manual-path beads: `o7w0` (import-freshness reminders, P2), `k3hd` (OS-notification
  trigger, P3), `rfsc` (export guidance, P3) — not launch-gated.
- `79mk` (Teller) re-scoped to BYO-credential posture per ADR 0004 §2.
- Launch moves out by roughly one focused arc (~3–4 PRs), accepted by the owner.

## Revisit if

- SimpleFIN Bridge shuts down, reprices materially, or loses its MX relationship → fall back
  to manual-first launch scope; re-rank Teller BYO.
- 1033/FDX yields an individual-access path → consider a direct FDX adapter tier.
- The SimpleFIN arc materially threatens the launch timeline → the owner re-decides the gate
  scope with data.

## Addendum (2026-09-02, personal-cfo-kdw6): the onboarding fork

Owner decision (2026-09-01 feedback): first-run setup opens with a choice —
**connect banks through the SimpleFIN Bridge** or **enter and import by
hand** — and branches into tailored flows. Recorded here because it fixes
the shape of every later onboarding child (5fp6):

- The fork is step 2 of the first-run guide, after the currency step; the
  choice persists per viewer (`localStorage`, `pcfo.onboardingPath`) and is
  revisitable — the guide can be rerun from Settings, and both branches keep
  the other path one click away (a connected user can add an account by
  hand; a manual user can link later from Settings).
- The connected branch shows the **disclosure panel before any token is
  pasted**, stating in plain language that the Bridge is independent and
  unaffiliated, that it handles the user's bank credentials as a separate
  service under its own terms, that it costs money (paid to them), and that
  it is optional — then reuses the Settings Connections card verbatim (link,
  map, create-an-account-from-mapping), so there is exactly one link/map
  surface in the app.
- The Bridge URL is rendered as selectable plain text, not a link: the app
  ships no opener/shell capability (ADR 0010), and adding one for a single
  URL is not worth widening the WebView's reach.
- Accounts remain the gate for advancing (the forecast needs a starting
  balance); a link whose provider has no accounts ready yet (k025) is told
  how to continue by hand.
