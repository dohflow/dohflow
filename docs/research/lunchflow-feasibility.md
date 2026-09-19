# LunchFlow adapter feasibility (spike output for `personal-cfo-hdk50`)

- **Date:** 2026-09-18
- **Decision: GO** for `personal-cfo-r2pow` (LunchFlow adapter, user-token tier) via
  LunchFlow's **Personal API** — with a zero-code fallback (LunchFlow's own SimpleFIN
  Bridge re-exposure) and a second zero-code fallback (CSV/OFX export) both already
  available today, independent of whether `r2pow` ships.
- **Method:** web research against LunchFlow's public marketing site (lunchflow.app),
  its public docs (lunchflow.app/docs, mirrored at lunchflow.mintlify.app), and its
  `llms.txt` doc index. Every claim below is a public-page fetch with the URL and the
  date fetched (2026-09-18 unless noted). **Tier: Public per ADR 0082** — the affiliate
  program's economics and the demand-test count live in
  `dohflow/internal`'s `docs/research/lunchflow-affiliate.md` instead of here; see that
  file for §(e).
- **What this doc does NOT establish:** several claims below need a real (owner-only)
  LunchFlow account, a live API key, and a connected bank to verify — those are called
  out explicitly rather than guessed at. See "Open items for the owner" at the end.

## (a) Auth model — classified against ADR 0004

LunchFlow exposes **two separate APIs**, not one:

1. **Personal API** (lunchflow.app/docs/api/personal-api-overview, fetched
   2026-09-18) — a user creates an "API destination" in their own LunchFlow
   dashboard, retrieves an API key from that destination's settings, and sends it as
   an `x-api-key` header on every request. No app registration, no OAuth, no
   project-owned client secret. This is exactly **ADR 0004 §2's user-token tier** —
   the same shape as the shipped SimpleFIN adapter (paste a credential the user
   generated themselves).
2. **Platform API** (lunchflow.app/docs/api/platform-api-overview, fetched
   2026-09-18) — a developer registers an application in a "Developer Dashboard" to
   get a `client_id`/`client_secret`, then redirects individual end users through an
   OAuth authorize flow to obtain per-user bearer tokens. This is the **relay tier**
   (ADR 0004 §10.3's paid-platform-or-nothing branch) — it requires DohFlow to hold a
   project-owned secret and register as a platform, which the free app's invariants
   (no project-owned secret, no relay) rule out.

**Verdict on this axis: the Personal API is the one to build against.** It matches
the user-token tier exactly, same as SimpleFIN. The Platform API exists and is
documented, but nothing about LunchFlow forces its use — a regular user can generate
a Personal API key without ever touching the Platform/OAuth flow. `r2pow`'s adapter
should use the Personal API only, and the doc/help copy should never mention the
Platform API (it would misleadingly suggest DohFlow needs a registered app, which it
does not).

**Unresolved from public pages, owner-verify:** which plan tier(s) actually expose the
"create an API destination" option in the live dashboard. LunchFlow's own website
(lunchflow.app, fetched 2026-09-18) describes the Individual plan ($34.99/yr) as
"sync to unlimited destinations" without naming API access explicitly, while a
separate marketing page (lunchflow.app/features/api-integration) states "all plans
include full API access with generous rate limits" and a third page implies the
"RESTful API with OAuth connect flow" is a Developer/Team-plan feature. These three
public pages do not agree, and none of them distinguish Personal API from Platform
API when making that claim. **This resolves only by creating a trial account and
checking the dashboard directly** — see "Open items for the owner."

## (b) ToS / ecosystem fit

**Resolved.** lunchflow.app/terms and lunchflow.app/acceptable-use both render their
substantive legal text client-side, so this session's automated fetches only returned
page navigation (2026-09-18, three attempts each). The owner read both in a browser
and provided the full text for review (2026-09-18, "Last updated 24 August 2026" per
both documents' own headers).

**No clause blocks a third-party client using a user's own self-service API key.**
Neither document contains a "Third-Party Applications" or "API Access" section at
all. The clauses that could plausibly apply, read precisely:

- "Transfer, distribute, or 'mirror' any part of our Services' **Materials**... without
  explicit authorisation" and "decompile, or reverse engineer any Materials, software,
  or content" — the Terms define **"Materials"** narrowly as "content provided,
  generated, or made available for or in relation to our Services," i.e. LunchFlow's
  own site/software/content. A user's own transaction data flowing through a
  documented, self-service API endpoint is not "LunchFlow's Materials," and using a
  published API as documented is not "reverse engineering" it.
- "Use automated scripts or technologies... to access, scrape, or extract data from
  our Services **without explicit consent from us**" — LunchFlow's own docs publicly
  document the Personal API specifically for programmatic access; publishing a
  self-service API is the explicit consent this clause requires. This targets
  unauthorized scraping of the website/app, not documented API use.
- The Acceptable Use Policy's "Fair use" clause ("business as usual... if your use is
  considered excessive, additional fees may be charged or capacity restricted") is a
  soft rate-limit, consistent with §(c)'s finding that no hard numeric rate limit is
  published anywhere. `r2pow`'s adapter should poll conservatively (on-vault-open +
  manual refresh, matching SimpleFIN's own pattern) rather than assume any specific
  request budget.
- The brand-use clause ("you may refer to our company name and brand in a factual and
  truthful manner... must not use our name, logo, trademarks... in any way that
  implies endorsement, sponsorship, or affiliation... without prior written consent")
  is a real constraint for future site/help copy (`personal-cfo-wk0iv`): naming
  LunchFlow factually ("connect via LunchFlow") is fine; using their logo is not,
  without separately checking the affiliate program's brand-asset permissions.

**Data processing location:** not stated in either document. The company is **Zen Labs
LTD**, UK-registered (company no. 16061160, VAT ID EU372096652), trading as Lunch
Flow — UK governing law and dispute resolution apply to the LunchFlow-customer
relationship, but this does not state where the aggregated financial data itself is
processed or stored. Still unresolved; not blocking, since DohFlow's own posture (no
DohFlow server, connector providers are independent controllers — the same framing
`personal-cfo-s3keh`'s privacy-page work already uses for SimpleFIN) does not depend
on knowing LunchFlow's specific processing region.

**Notable, non-blocking oddity:** the live Terms contain two unresolved template
placeholders where real values should be — the liability cap reads "the greater of
(a) the total amounts paid by you to us... or (b) `mpkwali0-x9idrhsjr8a`" and the
pre-litigation negotiation window reads "within `mp6oadoc-a52vai8iv24` days." Both
look like an unfilled legal-document-generator variable, not real contract terms.
Not something that affects DohFlow's integration (DohFlow is not a party to this
contract — the user is), but worth knowing: this is evidence of a small operation
running templated legal docs, in the same spirit as SimpleFIN's own "single small
operator" risk noted below, not a reason to distrust the service technically.

**Ecosystem fit, otherwise confirmed:** LunchFlow explicitly documents itself as a
sync/aggregation layer for third-party destinations — its own docs list SimpleFIN
Bridge, Actual Budget, Firefly III, Lunch Money, YNAB, Google Sheets, and CSV/OFX
files as first-class "destinations" (lunchflow.app/integrations, fetched
2026-09-18). Nothing in the read Terms or Acceptable Use Policy contradicts this
being the intended use of the Personal API.

## (c) Data shape

Confirmed endpoints (Personal API, `lunchflow.app/docs/llms.txt` index, fetched
2026-09-18):

| Entity | Endpoint | Stable id | Currency | Date fields | Pending/posted | Pagination | Rate limit |
|---|---|---|---|---|---|---|---|
| Accounts | `GET /accounts` | not documented publicly | not documented publicly | n/a | n/a | not documented | not documented |
| Transactions | `GET /accounts/{id}/transactions` | not documented publicly | not documented publicly | `from`/`to` date-range filter confirmed; no field names for posted/effective date given | confirmed distinct: `include_pending=true` toggles inclusion of "pending (unposted)" transactions — the API distinguishes the two states | no cursor/limit parameter documented; `from`/`to` is the only filter confirmed | not documented |
| Balances | `GET /accounts/{id}/balance` | n/a | not documented publicly | as-of timestamp not documented | n/a | n/a | not documented |
| Holdings | `GET /accounts/{id}/holdings` | not documented publicly | not documented publicly | not documented | n/a | not documented | not documented |

Holdings are explicitly scoped: "only available for accounts from providers that
support holdings (SnapTrade, MX, Finicity, Pluggy)" (llms.txt, fetched 2026-09-18) —
not every connected account will have a holdings endpoint response, matching
SimpleFIN's own institution-dependent holdings support.

**Everything marked "not documented publicly" in the table above requires a live API
key against a real connected account to observe** — LunchFlow's public docs describe
endpoints and high-level behavior but not full response schemas, stable-id field
names, amount sign convention, per-transaction currency, exact rate limits, or
pagination beyond the `from`/`to` filter. This is the single largest gap in this
doc and the reason `r2pow`'s implementation notes below flag "confirm against a live
response" repeatedly rather than asserting field names from documentation that does
not exist publicly.

## (d) Cost

- **Individual plan:** $2.92/month, or $34.99/year billed annually (LunchFlow's own
  page frames this as a "47% discount," though $2.92 × 12 = $35.04 — effectively flat,
  not a real discount; noted for accuracy, not a claim DohFlow repeats anywhere).
  Includes 2 connections; additional connections cost **$10.00 each** beyond the
  included pair (lunchflow.app, fetched 2026-09-18).
- **[U] resolved partially, one clause still open:** the plan §22 open question was
  "the period of '$10.00 per extra connection.'" Every public page found (the LunchFlow
  homepage, marketing copy, three independent web searches) states the figure as "$10.00 per
  extra connection" with **no time unit attached anywhere** — not "$10/month" or
  "$10/year." Given the base plan itself is quoted as an annual figure ($34.99/yr) with
  a monthly-equivalent shown alongside it, and the extra-connection charge is not shown
  with a separate cadence, the most likely reading is that $10.00 is **also an annual,
  per-connection add-on** (i.e., $34.99 + $10.00 × extra connections, billed yearly) —
  but this is an inference, not a citation, and the doc must not assert it as fact.
  **Unresolvable from public pages; the actual checkout flow (trial account, adding a
  third connection) is the only way to see the real billing cadence** — owner action.
- **7-day free trial** confirmed (lunchflow.app, fetched 2026-09-18).
- **Developer/Team plan:** no published self-serve cost, a contact-sales model — not needed, since
  the Personal API (§a) does not require this tier. No further research spent here.
- **Currency-account datum for C.7a:** LunchFlow's own website itself lists its cost
  in GBP, USD, or EUR (fetched 2026-09-18) and the service aggregates via US-market
  aggregators (SnapTrade, MX, Finicity) among its regional set — consistent with a US
  household on US institutions getting USD-denominated accounts through LunchFlow, the
  same way they would through SimpleFIN directly. This keeps `M-LunchFlow` at size M
  (multi-currency handling stays a real requirement for non-US households, not a
  blocker for the US case C.7a scopes).

## (e) Affiliate program

**Moved to `dohflow/internal`'s `docs/research/lunchflow-affiliate.md` per ADR 0082**
(this bead's own tier-split instruction, `personal-cfo-0uxwt`). The public
lunchflow.affonso.io landing page (fetched 2026-09-18) renders no terms without
logging in — it is built on Affonso, a third-party affiliate-program SaaS, and shows
only a language selector and a "Powered by Affonso" footer. Commission rate, cookie
window, payout terms, and disclosure requirements are genuinely not public; reading
them requires the owner's login. See the internal doc for the drafted D15
recommendation and the FTC disclosure sentence once those terms are read.

## (f) OFX/CSV export — the zero-adapter path

Confirmed via LunchFlow's own docs (lunchflow.app/docs, "CSV / OFX Files" destination,
fetched 2026-09-18):

- **CSV** fields confirmed by name: `Date, Amount, Merchant, Account Name, Currency,
  Balance, Transaction Type`. This already covers everything DohFlow's shipped CSV
  importer (`personal-cfo-cu8`) needs for a manual import.
- **OFX** is described only as "industry-standard... compatible with QuickBooks, Xero"
  and "preserves financial metadata and account structure in a standardised format" —
  **whether it carries `FITID` (the stable transaction id DohFlow's OFX importer,
  `personal-cfo-fr79`, relies on for dedupe) is not stated on any public page.** OFX
  is a standardized format that includes `FITID` by spec in the overwhelming majority
  of real-world exporters, so this is likely fine, but "likely" is not a citation.
- **Export cadence:** on-demand, no stated limit ("download as often as you like — no
  limits," fetched 2026-09-18) — this is materially better than a rate-limited API for
  a low-frequency manual workflow.
- **No publicly downloadable sample file exists** (confirmed by direct fetch attempts
  and three targeted web searches, 2026-09-18) — getting a real export to test against
  DohFlow's importers requires an actual account with at least one connected account,
  which is owner-only (see below). AC #5's "vendor's published sample" alternative is
  not available for LunchFlow; a real export is the only path.

### Bonus path found during this research: LunchFlow as a SimpleFIN Bridge

Not one of the original (a)–(g) questions, but the single most consequential finding
in this doc. LunchFlow's docs (lunchflow.app/docs/guides/destinations/simplefin-bridge,
fetched 2026-09-18) describe LunchFlow **acting as a SimpleFIN Bridge server** for its
own aggregated accounts:

1. The user creates a "SimpleFIN" destination inside their LunchFlow dashboard and
   picks which LunchFlow-connected accounts to expose.
2. LunchFlow generates a standard SimpleFIN **setup token** (base64 claim URL, valid 7
   days, single-use) — the exact same token shape SimpleFIN Bridge itself issues.
3. That token is pasted into any SimpleFIN-compatible client. Quoting the docs
   directly: *"LunchFlow acts as the SimpleFIN bridge server. Your app doesn't know or
   care that the data comes from LunchFlow — it just sees a standard SimpleFIN API."*

**This means DohFlow's already-shipped SimpleFIN adapter (`personal-cfo-w3gh`,
`crates/connectors/simplefin-adapter`) can connect to a user's LunchFlow-aggregated
accounts today, with zero new code**, the same way it connects to SimpleFIN Bridge
directly. A user who wants LunchFlow's broader international bank coverage (SimpleFIN
Bridge itself is US-only via MX) can link LunchFlow as a SimpleFIN destination and
paste that token into DohFlow's existing "Connect via SimpleFIN" flow — no adapter,
no registry entry, no new crate.

This changes the shape of the VERDICT below: LunchFlow support does not have to wait
on `r2pow` shipping. The SimpleFIN-Bridge-re-exposure path is a documentation-only
change (a help article + `/help/known-limitations` line saying "non-US banks: connect
via LunchFlow's SimpleFIN Bridge destination"), available immediately, independent of
whether a dedicated Personal-API adapter (`r2pow`) is ever built.

## (g) Verdict

**GO for `personal-cfo-r2pow`**, sized **M** as the plan estimated, via the Personal
API (§a) — user-token tier, no relay, no project-owned secret, same shape as the
shipped SimpleFIN adapter. Assumptions `r2pow` inherits from this doc, all flagged
"confirm against a live response" rather than asserted:

- Stable id field names for accounts/transactions/holdings (§c) — unknown until a real
  API response is inspected.
- Amount sign convention and per-transaction currency field (§c) — unknown.
- Rate limits and pagination beyond `from`/`to` date filtering (§c) — unknown; the
  adapter's polling cadence cannot be finalized until this is confirmed.
- Which plan tier(s) genuinely expose Personal API access in the live dashboard (§a) —
  unknown; affects the cost line DohFlow's help copy quotes.

**Independent of `r2pow`'s timeline, two zero-code paths exist today** and should be
documented immediately (Part 2 of this bead):

1. **LunchFlow as a SimpleFIN Bridge** (§f bonus finding) — works right now through
   DohFlow's existing SimpleFIN adapter, no new code.
2. **LunchFlow's CSV/OFX export** (§f) — works right now through DohFlow's existing
   file importers, no new code, pending the real-export test in "Open items."

Neither zero-code path removes the case for `r2pow`: both require the user to leave
DohFlow to generate a fresh token/export periodically (SimpleFIN Bridge re-exposure
tokens expire after 7 days per LunchFlow's own docs; CSV/OFX is fully manual), while a
native Personal API adapter gives the same on-vault-open sync UX SimpleFIN already
has. But neither path should wait on `r2pow` — they cost nothing to document today.

## Open items for the owner

These cannot be completed by an agent session — no account creation, no credential
entry, no reading of logged-in-only pages:

1. **Create a LunchFlow trial account** (7-day free trial, no card required per
   LunchFlow's own website) and confirm whether the Individual plan's dashboard actually shows
   the "create an API destination" option, or whether it's gated to Developer/Team.
   Resolves §a's open question.
2. **Connect at least one bank/brokerage account** in the trial, then:
   - Generate a Personal API key and make one real request to each of the four
     endpoints in §c's table; record the actual JSON field names for stable ids,
     currency, amount sign, and pending/posted status, and any rate-limit response
     headers.
   - Generate a CSV and an OFX export, and run the OFX file through DohFlow's file
     importer on a **test vault** (never a real vault) to confirm it parses — record
     the parser result (parsed rows, warnings, dedupe behavior on a second import of
     the same file) in this doc's §f. File a bug bead and link it here if it fails to
     parse.
   - Add a third connection and check the actual $10 charge's billing cadence
     (monthly vs. annual) shown at checkout — resolves §d's remaining [U].
3. ~~Read `lunchflow.app/terms` and `/acceptable-use` in an actual browser~~ — **done,
   2026-09-18.** No blocking clause found; see §(b) above.
4. ~~Log into `lunchflow.affonso.io`'s affiliate application~~ — **done, 2026-09-18.**
   See `dohflow/internal`'s `docs/research/lunchflow-affiliate.md` for the answers and
   the finalized D15 decision.
5. **Choose the demand-test instrument** (a GitHub Discussion thread on
   `dohflow/dohflow`, and/or an issue-label prompt) once Part 2's help content is
   merged, and record the link + baseline count + date on `personal-cfo-hdk50`'s notes.
