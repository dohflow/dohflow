# DOHFLOW — trademark clearance dossier

**Bead:** `personal-cfo-8ad7` (W0-5) · **Prepared:** 2026-09-03 · **Status:** draft for attorney review
**Confidentiality:** private until the fork-day flip (`personal-cfo-fkt5.11` privacy review governs).

---

## 0. What this document is, and is not

This is a **briefing packet to hand a trademark attorney** so the engagement starts
from facts rather than from a blank page. It exists to shorten the attorney's
intake, not to substitute for their work.

**It is not a clearance opinion, and nothing in it is legal advice.** In
particular:

- **No USPTO database search was performed.** The USPTO's search endpoints are not
  programmatically accessible without credentials, and the authoritative searches —
  `tmsearch.uspto.gov`, the 50 state registers, and common-law/trade-name sources —
  are the attorney's deliverable, not ours.
- Where this document says a fact is **verified**, it means verified by direct
  query against a public registry (RDAP, DNS, a package registry) on the date
  stated, and the method is recorded in §9 so it can be re-run.
- Where it says **unverified**, treat it as a lead, not a finding.

The single output we are buying is a **written clearance opinion within three
weeks of engagement**, because that opinion is the head of the critical path to
the public launch.

---

## 1. Engagement summary

| Item | Value |
|---|---|
| Mark | **DOHFLOW** — standard characters, no design element |
| Applicant | Christopher Bustos, individual (US) |
| Filing basis | **§1(b) intent-to-use** |
| Classes | **9** and **42** (see §4 — including whether 42 is wise) |
| Geography | **US only at launch** |
| Design mark | Deferred until the logo finalizes (`personal-cfo-n76x.3.3`) |
| Requested turnaround | Written opinion **≤ 3 weeks** from engagement |

**Why the applicant is an individual and not an entity.** LLC formation is
deliberately deferred (`personal-cfo-915.3`); the owner is in California, where an
LLC costs ~$890 in year one and $800/yr thereafter regardless of revenue, against a
free local-only app with no data custody and no payment processing. If an entity is
later formed, the mark is assigned via the USPTO Assignment Center
(`personal-cfo-dyobc` carries that obligation). **Question for counsel:** does
filing individually now and assigning later create any avoidable problem versus
waiting?

**Scope decision already taken.** US federal + state + common-law clearance only.
Madrid and EUIPO are deliberately deferred and re-evaluated inside the **six-month
Paris Convention priority window** measured from the ITU filing date. A calendar
entry for that window is required (`personal-cfo-n76x.22`).

---

## 2. The product, in the terms a trademark examiner would care about

DohFlow is a **downloadable macOS desktop application** for personal financial
management. It is local-first: the user's financial data lives in an encrypted
vault on their own machine. There is no telemetry, no account, and no server-side
processing of user data. It is distributed free under the AGPL, with donations via
GitHub Sponsors and Ko-fi.

Functionally: manual and imported transaction entry, account and balance tracking,
recurring-bill and income modelling, cash-flow forecasting with scenarios,
categorization, and debt/payoff views. Bank connectivity is planned via a
third-party aggregator (SimpleFIN), where **the user contracts with the aggregator
directly** — DohFlow does not intermediate the financial relationship.

That last point is the crux of the Class 36 question in §4.

---

## 3. Risk register

Ordered by what most needs an answer. Each item states what we know, how we know
it, and the specific question for counsel.

### R1 — DOUGHFLOW, and an active payments company behind it · **highest priority**

`DOUGH` and `DOH` are, for practical purposes, **phonetic equivalents** (/doʊ/).
A phonetically identical mark in an overlapping field is the classic likelihood-of-
confusion fact pattern, and this one has a live commercial actor attached.

**Verified 2026-09-03:**
- `doughflow.com` — registered **2002-09-19**, expires **2027-09-19**, registrar
  **Cloudflare**. It issues an **HTTP 301 to `premiercashier.com`**.
- `premiercashier.com` resolves and is live (HTTP 200). Public descriptions
  characterize Premier Cashier as a **payment-orchestration platform** for
  merchants, handling crypto payments since 2014.
- `doughflow.app` — registered **2025-11-23**, expires **2026-11-23**, registrar
  **Namecheap**. A *different* holder from the .com.

**Unverified:** whether DOUGHFLOW is registered or applied-for at the USPTO or any
state register; whether Premier Cashier has ever used "DoughFlow" as a product,
brand, or trade name; whether the 2002 .com registration reflects continuous use or
is simply a long-held defensive asset. A redirect is **not**, by itself, trademark
use — but a 24-year-old domain held by an operating fintech is not nothing.

**Questions for counsel:**
1. Is DOUGHFLOW registered, applied-for, or in common-law use in Classes 9, 36 or 42?
2. Does the fintech-adjacent field overlap create a likelihood-of-confusion problem
   for DOHFLOW in Classes 9 and 42 specifically?
3. Does `doughflow.app` in a third party's hands (expiring 2026-11-23) change anything?
4. Is a consent agreement or coexistence agreement worth contemplating, or is this
   clean enough to ignore?

### R2 — `dohflow.com` is held by someone else, with a placeholder

**Verified 2026-09-03:** registered **2026-02-03**, expires **2027-02-03**,
registrar **Squarespace Domains**, Google Domains nameservers. The site returns
HTTP 200 and serves a **Squarespace "coming soon" placeholder** — "under
construction," no business name, no products, no services, no industry identified.

**Reading:** a placeholder identifying no goods or services is thin ground for
common-law rights. But the exact-match .com was taken **seven months ago** on a
one-year registration, which is consistent with either speculation or an
undisclosed plan. It is being drop-watched (`personal-cfo-n76x.22`).

**Question for counsel:** does an unbranded coming-soon page create any priority or
common-law exposure we must account for, and does it affect the strength of a
registration we obtain?

### R3 — Search both spellings, and the near neighbours

Given R1, the search should not be limited to the literal string. At minimum:
**DOHFLOW, DOUGHFLOW, DOE FLOW, DOFLOW, D'OH FLOW**, and the two-word and
hyphenated forms of each.

### R4 — "FLOW" is crowded in financial software

"Flow" and "cashflow" are heavily used in fintech naming. A crowded field usually
means a **narrower scope of protection** even when registration succeeds.

**Questions for counsel:** is DOHFLOW distinctive enough to register without a
descriptiveness refusal? And realistically, what would we be able to enforce
against — identical marks only, or the broader phonetic family in R3?

### R5 — Class 36 is deliberately excluded. Confirm that.

We propose **9 and 42 only**, on the reasoning that DohFlow supplies *software*,
not *financial services*: no funds are held, moved, advised on, or intermediated,
and the SimpleFIN relationship is between the user and the aggregator.

**Question for counsel:** is excluding Class 36 correct, and does forecasting and
budgeting functionality risk being characterized as financial services anyway?

### R6 — Class 42 may expire before we ever use it · **cost risk**

Class 42 covers SaaS/non-downloadable software. **DohFlow is downloadable-only
today** — Class 9 covers what actually exists. Class 42 is a bet on the future
hosted tier, which has **no committed date** (the site's `/pricing` page says
"future hosted plans" with no dates, deliberately).

Under §1(b), a Statement of Use is due within 6 months of the Notice of Allowance,
extendable five times by 6 months each — roughly **36 months maximum**. If the
hosted tier has not launched inside that window, **Class 42 dies and its fees are
sunk** (~$350 filing + up to $625 in extension fees for that class alone).

**Questions for counsel:**
1. Do we have a bona fide intent to use in Class 42 sufficient to support the filing?
2. Is filing 42 now worth the risk, or better to file Class 9 now and add 42 as a
   fresh application once the hosted tier is real?

### R7 — The name is already public, before the opinion

By deliberate decision (recorded in `8ad7`), the name is publicly visible **now**:
`dohflow.app` is registered, `github.com/dohflow` is reserved, and a coming-soon
placeholder will be live shortly. Accepted sunk cost ≈ $60 plus placeholder work.
Mitigations: the GitHub org description, avatar, and Sponsors bio are all kept
**name-free** until the opinion lands, and the LICENSE PR, TRADEMARK.md, and the
public repo flip all remain gated on a favourable opinion.

**Question for counsel:** does pre-filing public use create any problem — priority,
or otherwise — or is it simply irrelevant to a §1(b) application?

---

## 3A. DIY federal knock-out search — results (2026-09-04)

Run by the owner's agent against `tmsearch.uspto.gov` (the USPTO's own wordmark
search), all status filters checked so both **live and dead** records were in
scope. **This is a knock-out screen, not a clearance opinion** — see the limits
below before relying on it.

| Query | Results | Notes |
|---|---|---|
| `dohflow` | **0** | No record, live or dead |
| `doughflow` | **0** | No record, live or dead |
| `doflow` | **0** | — |
| `doeflow` | **0** | — |
| `doh` | 167 | **None in Class 9, 36 or 42 for financial software.** Hits cluster in toys (028), apparel (025), leather (018), hospitality (043). PLAY-DOH (Hasbro) is the notable holder; its Class 042 record is for computer *game* programs and is DEAD |
| `quicken` *(control)* | 79 | Control query, confirming the search mechanism returns results correctly |

**The control matters.** A "no results" screen is worthless if the query silently
failed, so `quicken` was run through the identical path and returned 79 records
with live/dead status, classes, serials, and owners. The zeros above are real.

### What this changes about R1

R1 was built on *domain* evidence — `doughflow.com` held since 2002 and redirecting
to an operating payments company. That evidence stands. But **there is no federal
trademark filing for DOUGHFLOW at all**, live or dead. So the risk is no longer
"an existing registration blocks us"; it narrows to the much smaller question of
**unregistered common-law rights** arising from actual commercial use.

That is a materially cheaper question to put to an attorney: *no federal
registration exists — assess common-law use only.*

### Limits of this screen — read before relying on it

This search does **not** substitute for the attorney work, and specifically did
not cover:

- **State trademark registers** (50 of them)
- **Common-law and trade-name use** — rights can exist with no USPTO filing at all,
  which is exactly the open question on Premier Cashier
- **Design marks** and stylized forms that carry no matching text
- **Phonetic neighbours that share no letters** with the queried string
- **Foreign registrations**, including anything reaching the US via Madrid
- **Likelihood-of-confusion analysis**, which is legal judgment and not a database query

A "contains this word" search also depends on how the engine tokenizes: `dough flow`
as two words returned 7,769 largely irrelevant hits (cookie dough, toys, apparel),
confirming that multi-word input is ORed rather than phrase-matched. The
single-token forms in the table are the meaningful test.

**Nothing here is legal advice, and none of it is a clearance opinion.**

---

## 4. Proposed identifications of goods and services

Draft wording for counsel to correct against the current USPTO ID Manual.

**Class 9 — downloadable software** *(this covers the product as it exists today)*

> Downloadable computer software for personal financial management, namely, for
> budgeting, cash-flow forecasting, tracking financial accounts and transactions,
> categorizing expenditures, and managing recurring bills and income.

**Class 42 — SaaS** *(intent-to-use; see R6 before filing)*

> Software as a service (SAAS) services featuring computer software for personal
> financial management, budgeting, cash-flow forecasting, and tracking financial
> accounts and transactions.

Prefer pre-approved ID Manual wording wherever it exists — it reduces the odds of
an identification-based office action and is cheaper than arguing.

---

## 5. Specimen plan

**Class 9.** Per TMEP 904.03(e), a **web page displaying the mark together with a
working download link** is an acceptable specimen for downloadable software. The
`/download` page on `dohflow.app` is built for exactly this
(`personal-cfo-n76x.15`): the mark appears in the header and adjacent to the DMG
download and its SHA-256. The Statement of Use should be timed to follow the
v0.1.0 public release, when that page is live and the download works.

**Class 42.** Requires actual SaaS use — a signed-in service screen or comparable
evidence showing the mark. **Not available until the hosted tier ships**, which is
the substance of R6.

**Fee note:** SOU is $150/class; extension requests are $125/class and may be filed
five times. Both need calendar entries the moment a Notice of Allowance arrives.

---

## 6. Filing parameters and expected timeline

| Item | Value | Source / note |
|---|---|---|
| Application fee | **$350 per class** → $700 for two | Confirm current fee schedule at filing |
| Statement of Use | $150 per class | Due 6 months after Notice of Allowance |
| Extensions | $125 per class, ×5 max | ~36 months total runway |
| First office action | ~4.2 months | USPTO pendency, as of Aug 2026 |
| Disposal | ~9.7 months | USPTO pendency, as of Aug 2026 |
| Attorney fee | $1,000–2,500 flat expected | Solo practitioners quoted ~$500–600 |
| Paris window | 6 months from filing | Madrid/EUIPO decision point |

**DIY fallback:** if the opinion is clean and the owner elects to file without
counsel, filing goes through the USPTO Trademark Center, which requires ID.me
verification or a notarized paper alternative.

---

## 7. Namespace availability

Reserved and verified as of 2026-09-03. The package and social reservations are
gated on the attorney's **knock-out result** (`personal-cfo-n76x.8`) — which is a
fast first pass, days rather than the full three weeks. **Ask for the knock-out as
a separate early deliverable** so these are not stuck behind the written opinion.

| Namespace | Status | Verified how |
|---|---|---|
| `dohflow.app` | **Owned** — 2026-09-04 → 2028-09-04, Cloudflare Registrar | RDAP |
| `github.com/dohflow` | **Reserved** 2026-09-04, 2FA required, profile blank | GitHub API |
| crates.io `dohflow` | **Available** | crates.io API — "does not exist" |
| npm `dohflow` | **Available** | registry.npmjs.org — 404 |
| PyPI `dohflow` | **Available** | pypi.org — 404 |
| X / Bluesky / Mastodon / YouTube / Reddit | **Not yet checked** | `personal-cfo-n76x.8` |

---

## 8. Domain evidence table

All rows verified by RDAP on **2026-09-03** unless marked otherwise.

| Domain | Registered | Expires | Registrar | Notes |
|---|---|---|---|---|
| `dohflow.app` | 2026-09-04 | 2028-09-04 | Cloudflare | **Ours.** Transfer lock on |
| `dohflow.com` | 2026-02-03 | 2027-02-03 | Squarespace Domains | Third party. Squarespace "coming soon" placeholder, no goods/services identified. Drop-watch |
| `dohflow.io` | — | — | — | **RDAP inconclusive.** Earlier note: available at ~$50/yr, skipped on cost |
| `doughflow.com` | 2002-09-19 | 2027-09-19 | Cloudflare | **301 → premiercashier.com** (payment orchestration). See R1 |
| `doughflow.app` | 2025-11-23 | 2026-11-23 | Namecheap | Third party, different holder from the .com |
| `doughflow.io` | — | — | — | **RDAP inconclusive.** Earlier note: GoDaddy, to 2027-04-25 |

No defensive `dough*` registrations were made, deliberately — buying them would
neither create nor strengthen rights in DOHFLOW.

---

## 9. Verification log

Everything marked *verified* above was produced by these methods on **2026-09-03**,
and each is re-runnable:

- **Domain facts** — RDAP: `https://rdap.org/domain/<name>`, and for `.com`
  directly against `https://rdap.verisign.com/com/v1/domain/<name>`. Note that the
  rdap.org bootstrap returned an empty result for `dohflow.com` while the Verisign
  endpoint returned a full record — **prefer the registry endpoint** for `.com`.
- **`doughflow.com` redirect** — `curl -I -L`, observed 301 → `https://premiercashier.com/`.
- **`dohflow.com` content** — page fetch; characterized as a Squarespace
  coming-soon placeholder.
- **Package namespaces** — crates.io API, `registry.npmjs.org`, `pypi.org` JSON API.
- **GitHub org** — `gh api orgs/dohflow`.

**Explicitly NOT done:** any USPTO, state-register, or common-law trademark search.
Web searches for "DoughFlow trademark" surfaced no public record, which is **weak
negative evidence and must not be relied on**. The authoritative search is the
attorney's.

---

## 10. Consolidated question list for counsel

1. Is DOHFLOW clear to register in Classes 9 and 42, US federal, state, and common law?
2. **DOUGHFLOW / Premier Cashier** (R1) — registered, applied-for, or in common-law
   use? Does it block us? Consent agreement worth pursuing?
3. Does the `dohflow.com` placeholder (R2) create priority or common-law exposure?
4. Is DOHFLOW distinctive enough to avoid a descriptiveness refusal in a field
   crowded with "flow" marks (R4)?
5. Is excluding **Class 36** correct for local-only software with no funds handling (R5)?
6. Should **Class 42** be filed now on intent-to-use, or deferred until the hosted
   tier is real, given the ~36-month SOU ceiling (R6)?
7. Does filing as an individual with a later assignment to an LLC create any problem (§1)?
8. Does pre-filing public use of the name (R7) matter at all for a §1(b) filing?
9. Can you provide the **knock-out result separately and early**, ahead of the full
   written opinion, so namespace reservations can proceed (§7)?
10. What is your recommendation on Madrid/EUIPO within the six-month Paris window?

---

## 11. Bundled documents for the same review

Per `personal-cfo-rv2s`, these are reviewed in the same engagement to avoid paying
twice for context (expected +$250–500):

- In-app disclaimer copy (not financial advice; no warranty)
- `CLA.md` — contributor licence agreement
- `TRADEMARK.md` — name and logo explicitly **not** licensed by the AGPL
- Website privacy policy, terms of service, accessibility statement
- App-side privacy and data-retention documentation

---

## 12. Change log

| Date | Change |
|---|---|
| 2026-09-03 | Initial dossier. Domain, namespace, and redirect facts verified by direct registry query; USPTO search explicitly out of scope and left to counsel. |
| 2026-09-04 | Added §3A: DIY federal knock-out screen run against tmsearch.uspto.gov with a control query. DOHFLOW, DOUGHFLOW, DOFLOW, DOEFLOW all return zero records live or dead. R1 narrows from "an existing registration blocks us" to "assess common-law use only". |
