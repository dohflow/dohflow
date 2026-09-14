# ADR 0030: Transaction categorization — taxonomy CRUD + assignment

- **Status:** Accepted
- **Date:** 2026-06-26
- **Deciders:** Project owner
- **Beads:** [`personal-cfo-bac`](../../.beads/issues.jsonl) (CRUD), `personal-cfo-d3p` (schema)
- **Builds on:** ADR 0006 (Finance Kernel command boundary), ADR 0007 (ledger/posting
  model — categories annotate, they don't post), ADR 0009 (materialized read models),
  ADR 0011 (op-log), ADR 0014 (the Money Inbox surfaces categorization work)

## Context

R2 turns a list of bare transactions into an organized ledger. The category schema
(`categories` + `category_aliases` + a re-parent-cycle trigger, seeded with the §9.6
taxonomy at vault create) shipped with `d3p`, but nothing **manages** it (no CRUD) and
nothing **assigns** a category to a transaction. The `transaction_display` read model
already reserves `primary_category_id` / `category_confidence_bps` / `category_source`
for the result. This ADR settles the two open decisions: how categories are edited, and
where a transaction's category lives.

## Decision

### 1. The taxonomy is hierarchical, typed, and partly system-owned

`categories` are a tree (`parent_id`), each with a `type`
(`income`/`expense`/`transfer`/`adjustment`) and a `forecast_behavior` the forecast
consumes. The §9.6 defaults are seeded with `is_system = 1`. Re-parenting cycles are
rejected by the existing DB trigger (`categories_no_parent_cycle`); the kernel relies on
it rather than re-deriving the check.

### 2. CRUD is op-logged kernel commands; delete is soft (archive); system is protected

Category edits go through the command bus (ADR 0006), each writing the op-log (ADR 0011):
`CreateCategory`, `UpdateCategory` (name/icon/color), `MoveCategory` (re-parent),
`ArchiveCategory`, `ReinstateCategory`. **There is no hard delete** — non-destructive by
default (AGENTS.md §1); "delete" is archive (a new `archived_at` column; archived
categories hide from pickers but keep historical assignments valid).

**A system category's identity is immutable; its appearance is not.** A user cannot
rename, re-parent, or hard-delete a system category (its **identity** — name, type,
parent — is fixed), but **may customize its appearance (color + icon)**. Alias matching
keys on identity, not appearance, so recoloring or re-iconing a default is safe: it does
not diverge vaults or break import matching, and the canonical taxonomy stays stable across
app versions. A user who wants a different *name/structure* **creates their own** category.
Users' own categories are fully editable. (See the 2026-07-09 addendum; this refines `bac`'s
"users cannot delete system categories but can hide them.")

### 3. A transaction's category lives in a dedicated assignment store

A new canonical table **`transaction_categorizations`** holds one row per categorized
transaction: `transaction_id` (1:1), `category_id`, `source`
(`user` / `rule` / `model` / `import_alias`), `confidence_bps`, `assigned_at`.
**`RecategorizeTransaction`** is an op-logged kernel command that upserts it (latest
assignment wins — no silent re-tag, per the AC). A manual assignment is `source = user`,
`confidence_bps = 10000`. The `transaction_display` projection (and the transactions-list
read) join this table to fill `primary_category_id` / `category_confidence_bps` /
`category_source`. The category is **metadata on the transaction**, not a ledger posting —
ADR 0007's double-entry model is untouched (mirrors how `byxe`'s memo/counterparty sits
in a side table).

### 4. The Money Inbox is the categorization queue (later)

Uncategorized or low-confidence transactions become Money Inbox items (ADR 0014, bead
`uc95`) — a deterministic generator over `transaction_categorizations` vs. the ledger.
Automatic categorization (alias matching, rules, a local classifier — `user_rules` /
`r6o5`) feeds the same store with non-`user` sources and is **out of scope here**; this
ADR covers the taxonomy CRUD + manual assignment only.

## Consequences

- The taxonomy is user-manageable without risking the canonical defaults; every edit is
  auditable and reversible (archive, not delete).
- Categories project into the existing read-model fields, so the transactions list and
  future budget/forecast-by-category work read one place.
- Auto-categorization later writes the **same** assignment store with a different
  `source`, so the manual and automatic paths converge without a schema change.

## Addendum 2026-06-29 — Auto-categorization v1: merchant memory (`7yh0`/`5n4`)

The first non-`user` source lands: **merchant memory**. The user's own manual
categorizations are the training signal — when transactions sharing a normalized merchant
key (merchant normalization v1, `personal-cfo-7yh0`) have been categorized, that learned
category is applied to the household's still-**uncategorized** transactions of the same
merchant.

- **Source `rule`, confidence = agreement.** A learned merchant→category association is a
  `source = rule` assignment (distinct from the `user` manual layer and the future `model`
  classifier). `confidence_bps` is the agreement ratio — the share of that merchant's
  manual categorizations that chose the winning category. Applied only when agreement
  clears a threshold (v1: 60%); conflicted merchants are left for review.
- **Uncategorized-only; never a silent re-tag.** It fills only transactions with *no*
  assignment, so a user assignment is never overwritten (latest-user-wins is preserved); a
  later manual categorization overrides it.
- **User-triggered (opt-in).** v1 applies on an explicit user action (the IPC
  `apply_merchant_memory`), honoring §11's "overrides are training signals, not automatic
  universal rules, unless the user opts in." Auto-apply on import + the low-confidence
  review queue (`j5ij`) are follow-ons.

## Alternatives considered

- **A `category_id` column on `transaction_details`** (the `byxe` memo table). Rejected:
  conflates inherent import detail with a mutable, multi-source assignment that needs its
  own `source` + `confidence` + history semantics.
- **Hard-delete categories.** Rejected: orphans historical assignments and violates the
  non-destructive default; archive preserves history.
- **Rename/re-parent system categories in place.** Rejected: a renamed/moved default
  diverges every vault and breaks alias matching; users fork their own instead. (Note:
  this covers *identity* only — recoloring/re-iconing a default is allowed, since alias
  matching keys on identity, not appearance. See the 2026-07-09 addendum.)

## Addendum 2026-06-29 — Auto-apply merchant memory on import (`5n4.2`)

Merchant-memory apply (the 2026-06-29 addendum above) graduates from a purely manual action
to also running **automatically after an import**, so freshly imported transactions arrive
already categorized instead of waiting for the user to click *Auto-categorize*.

- **Setting-gated, default ON.** A new household setting `auto_categorize_on_import` (stored
  in the `settings` key-value table; `"true"`/`"false"`, **defaulting to `true` when unset**)
  controls it. This honors §11's "overrides are training signals, not automatic universal
  rules, unless the user opts in" by giving the user an explicit control, while defaulting to
  the helpful behavior — safe to default on because the apply is **uncategorized-only, never
  overwrites a user assignment, lands as `source = rule` with the visible provenance badge
  (`5n4.1`), and stays reviewable**. The user can turn it off in Settings; the manual
  *Auto-categorize* action remains regardless.
- **Where it runs.** In `finance-kernel::ingest_batch`, after the staged-commit loop and the
  terminal batch-state update, guarded on `committed > 0` **and** the setting. It reuses the
  existing `apply_merchant_memory` (idempotent, threshold-gated), so it is not import-scoped —
  it fills any still-uncategorized transaction the new import made learnable, which is the
  intended whole-household behavior.
- **Reported, not silent.** `BatchResult`/`BatchResultDto` gain an `auto_categorized` count so
  the import dialog can say how many rows were auto-categorized — auto-apply is surfaced, never
  a hidden mutation.

Deferred: the low-confidence review queue (`j5ij`) that triages sub-threshold/auto-applied
assignments remains the next follow-on.

## Addendum 2026-06-29 — Low-confidence review queue (`j5ij` / `uc95`)

Auto-categorization is allowed to be wrong, so the marginal calls must be reviewable. A
**low-confidence categorization** surfaces as a Money Inbox item the user can confirm or
correct; this closes the §11.5 loop ("auto-apply on high confidence; queue for review on the
rest").

- **What qualifies.** A committed, non-voided, **unreviewed** transaction whose categorization
  is from an **auto source** (`source IN (rule, model)`) with `confidence_bps` **below 7000**
  (70%). Manual (`source = user`, confidence 10000) and high-confidence auto assignments are
  trusted and never queued. With merchant memory applying at ≥60% agreement (the 2026-06-29
  addendum), the review band is the 6000–6999 bps slice — auto-applied but not yet trustworthy.
- **Computed on read, off canonical state.** Like the stale-balance and unreviewed-transaction
  items, the generator is **not materialized** — it reads `transaction_categorizations` +
  `transaction_reviews` at `money_inbox_list` time and merges in. No migration, no
  `transaction_display` projection wiring, no incremental-projection seam (the concern recorded
  in the `uc95` notes is sidestepped by driving off canonical tables, exactly as
  `unreviewed_transaction` does). Resolving an item changes canonical state, so the next read
  simply omits it.
- **One item per transaction.** A low-confidence-categorized row is also unreviewed, so it would
  otherwise match both generators. `money_inbox_list` suppresses the generic
  `unreviewed_transaction` item for any transaction that has a low-confidence item — the more
  actionable category card wins.
- **Accept = confirm in place (mark reviewed, keep `source = rule`).** Accepting does **not**
  rewrite the assignment to `source = user`; it marks the transaction reviewed, which drops it
  from the queue while preserving the honest provenance ("a rule guessed this and I waved it
  through" is not the same as "I chose this"). The category stays editable. Correcting instead
  routes through the existing `RecategorizeTransaction` (→ `source = user`), which also drops it.
- **Bulk accept.** "Accept all N" marks every currently-queued low-confidence transaction
  reviewed in one confirmed action — dispatched as one `MarkReviewed` command per row (the
  event-sourced path, like the import commit loop), not a bulk table write. Returns the count.

This is `uc95` (the inbox-item generator) and `j5ij` (the review/bulk surface) together; they
are two halves of one feature. The `j5ij → j99u` (user rule engine) dependency was dropped: the
real prerequisite is *an* auto source producing sub-threshold categories, which merchant memory
satisfies — the rule engine is one future source, not a gate.

## Addendum 2026-06-29 — Canonical merchant-identity entity layer (`zrpg`, Arc B foundation)

Merchant normalization v1 (`7yh0`) gives a deterministic *string* key per raw description
(`categorization::normalize_merchant`: `"AMZN MKTP US*1A2B"` → `"AMZN MKTP US"`). That key is
per-location and per-string by design — safe, but `"AMZN MKTP US"` and `"AMAZON.COM"` stay
*different* keys. The **entity layer** is what collapses them onto one real-world merchant, so a
category learned for one applies to all, and ambiguous merchants can be flagged. This addendum
defines the schema; the resolver/seeding/fuzzy-matching and read-model wiring are downstream
(`7yh0`-v2, the ambiguous-merchant beads).

**Two tables (migration 27), following the baseline conventions (BLOB UUID PKs, `REFERENCES`
without enforced FK pragmas, `CHECK`-constrained tokens, RFC-3339 `created_at`):**

- **`merchant_identities`** — the canonical merchant entity:
  - `id BLOB PRIMARY KEY`, `display_name TEXT NOT NULL` (e.g. `"Amazon"`).
  - `default_category_id BLOB` (nullable, `REFERENCES categories(id)`) — the entity's usual
    category, the seam auto-categorization reads. **Null for ambiguous merchants.**
  - `is_ambiguous INTEGER NOT NULL DEFAULT 0` — true for merchants that genuinely span
    categories (Amazon, Costco, Venmo/Zelle). Signals "do **not** auto-apply a single category;
    this needs disambiguation / a split template" — the hook the ambiguous-merchant beads
    (`6p6b`/`2a6r`/`9rtv`/`dr1b`/`x75h`) build on.
  - `source TEXT NOT NULL CHECK (source IN ('seed','user','auto'))` — provenance.
  - `created_at TEXT NOT NULL`.
- **`merchant_aliases`** — normalized key → identity (the many-to-one grouping):
  - `normalized_key TEXT PRIMARY KEY` — the `normalize_merchant` output; **PK because a key
    resolves to exactly one identity** (deterministic lookup). Both `"AMZN MKTP US"` and
    `"AMAZON COM"` are rows pointing at the one Amazon identity.
  - `merchant_identity_id BLOB NOT NULL REFERENCES merchant_identities(id)`.
  - `source TEXT NOT NULL CHECK (source IN ('seed','user','auto'))`, `confidence_bps INTEGER
    NOT NULL` (seed/user = 10000; auto/fuzzy < 10000 so a weak grouping is auditable).
  - `created_at TEXT NOT NULL`; index on `merchant_identity_id` for reverse lookup (an
    identity's aliases).

**Why a key→identity table and not a column on the transaction:** the same merchant recurs
across thousands of transactions; mapping the *normalized key* (a few hundred distinct strings)
keeps the grouping compact, inspectable, and editable in one place, and the existing
`transaction_display_rows_read_model.merchant_identity_id` forward-reference resolves through it.

**Scope of `zrpg`:** the migration + a typed db-worker seam (`upsert_identity` / `link_alias` /
`resolve`) with tests. **No** kernel/IPC surface, **no** read-model population, **no** seeded
identities or fuzzy matching — each arrives with its consumer so the seam is designed against
real use, not speculatively.

## Addendum 2026-06-30 — Identity-aware merchant-memory grouping (`7yh0`-v2, `5n4.3`)

With the merchant-entity schema live (`zrpg`), merchant-memory auto-categorization
(`merchant_memory::apply`) now groups by the resolved **identity** rather than the raw
normalized string. It loads the `merchant_aliases → merchant_identities` map and keys the
learning + fill on the identity (`id:<uuid>`) when an alias resolves the normalized
counterparty, falling back to the normalized string (`k:<key>`) otherwise.

- **Cross-alias learning.** A category the user assigns under one alias (`AMAZON COM`) now
  fills uncategorized transactions under another alias of the same identity (`AMZN MKTP
  US`) — grouping `normalize_merchant` alone could not do, since those are distinct keys.
- **Ambiguity guard.** An `is_ambiguous` identity (Amazon, Costco — merchants that span
  categories) is **excluded** from auto-fill: a single learned category must not propagate
  across an ambiguous merchant. Those defer to disambiguation (the ambiguous-merchant
  beads + split templates).
- **Backwards-compatible.** With no identities seeded the resolution map is empty and the
  key is the normalized string — behaviour identical to v1 (`5n4.1`).

Grouping comes from **seeded/user aliases via exact-key resolution** (zero false-merge
risk). Open-ended *auto*-discovery of same-merchant aliases without a seed is the separate,
ADR-gated `5n4.4`, where a conservative high-precision heuristic must be decided before
build.

## Addendum 2026-06-30 — Canonical-merchant seed: Amazon as ambiguous (`6p6b`)

Like the default category taxonomy, a set of **canonical merchants** is seeded into every
vault at init (`merchant_identity::ensure_seed_merchants`, run in the `open()` bootstrap
after `ensure_default_categories`). Each ambiguous-merchant bead extends the `SEED_MERCHANTS`
list; **Amazon** (`6p6b`) is the first: an `is_ambiguous` identity whose retail descriptors
(`AMZN MKTP US`, `AMAZON.COM`, `AMAZON MKTPL`, …) normalize to its aliases, so a real Amazon
transaction resolves to it and the merchant-memory **ambiguity guard** (7yh0-v2) keeps a
single learned category from mis-tagging the rest.

- **Idempotent.** Deterministic v5 ids (`merchant-seed:<name>`) + `INSERT OR IGNORE`: every
  open re-runs the seed harmlessly, and a user's own alias mapping for the same normalized
  key is never clobbered (the alias PK already exists).
- **Conservative aliasing.** Only the genuinely category-spanning retail descriptors are
  aliased to Amazon. Consistently-categorizable sub-brands (Prime, Audible, Whole Foods) are
  left out so they categorize normally rather than being suppressed by the guard.
- **Scope.** This addendum is the seed + guard. The richer Amazon disambiguation
  (subscription-amount detection, split behaviour, receipt linkage, category-distribution
  history — plan §11.6) is a follow-on that depends on split templates (`5mm8`).

## Addendum 2026-06-30 — Prefix aliases + the rest of the ambiguous merchants (`2a6r`/`9rtv`/`x75h`/`dr1b`)

Extends the canonical-merchant seed (`6p6b`) to the remaining ambiguous merchants, which
split by *how* their normalized key behaves:

- **Online, stable key (exact alias).** Apple (`9rtv`) bills as `APPLE.COM/BILL`,
  `ITUNES.COM` — seeded as exact aliases like Amazon.
- **Physical, per-location key (prefix alias).** `normalize_merchant` deliberately keeps the
  store city, so Costco/Target/Walmart (`2a6r`) produce `COSTCO WHSE SEATTLE` ≠
  `COSTCO WHSE PORTLAND`. A new **`match_type='prefix'`** alias (migration 28) resolves any
  key that begins with a brand token on a **word boundary** — `COSTCO` matches
  `COSTCO WHSE SEATTLE` but `APPLE` never matches `APPLEBEE'S`. Longest prefix wins. This is
  curated, high-precision grouping — distinct from the open-ended auto-fuzzy of `5n4.4`.
- **Already handled by normalization (no seed).** Processors (`dr1b`: Square/Stripe/PayPal)
  and P2P (`x75h`: Venmo/Zelle) are **prefix-stripped** by `normalize_merchant` (shipped in
  `7yh0`): `PAYPAL *SPOTIFY → SPOTIFY`, `VENMO PAYMENT … JOHN → JOHN`. The stripped result is
  the real merchant / counterparty and categorizes on its own. Only the **bare** P2P forms
  (`VENMO`, `ZELLE`, `CASH APP` with no counterparty) are genuinely uninformative, so those
  are seeded ambiguous. Seeding the *processor itself* as an identity would be wrong — the
  whole point of stripping it is to reveal what's underneath. The richer P2P signals
  (amount / recurrence / counterparty history) remain a follow-on (`x75h`).

## Addendum 2026-06-30 — Auto-fuzzy merchant grouping by anchored containment (`5n4.4`)

The open-ended half of merchant grouping: discover that several normalized keys are the same
multi-location chain *without* a curated seed — `RITUAL COFFEE SF` ≡ `RITUAL COFFEE OAKLAND`,
but `AMERICAN AIRLINES` ≢ `AMERICAN EAGLE`. A false merge cross-contaminates categorization,
so precision is paramount; recall is secondary. An adversarial design pass scored four
candidate heuristics against a trap fixture — three "drop the city, compare the remainder"
variants all had the **single-industry-noun hole** (`PIZZA OAKLAND` / `PIZZA DENVER` are
different shops but collapse to `PIZZA`). The chosen design is **anchored containment**:

- **Brand extraction** (`categorization::merchant_grouping`, pure). Peel a trailing location
  tail — a multi-token city, then single-token cities / full state names / metro abbrevs /
  area words (`DOWNTOWN`, `AIRPORT`, `MALL`) / 1–3-digit store residues — from a curated
  lexicon, never peeling the last token. The remaining spine is the **brand anchor** only if
  it is distinctive: **multi-token** (and not an all-generic head phrase), or a **single
  token ≥6 chars** that is neither a `GENERIC_HEAD` (`AMERICAN`, `UNITED`, `FIRST`, `BANK`…)
  nor a `GENERIC_INDUSTRY` noun (`PIZZA`, `BURGER`, `TACOS`, `COFFEE`…). Lexicon-classified
  peeling (not "drop any trailing token") is what avoids the `AMERICAN AIRLINES` trap; the
  industry-noun denylist closes the `PIZZA` trap.
- **Minting, not fusion** (db-worker `merchant_grouping::mint_anchors`). Never fuse two
  arbitrary keys. Among still-**unresolved** observed keys, when **≥2 distinct location-
  bearing keys** share one anchor (a chain seen at two locations), mint an `is_ambiguous =
  false` identity for the brand and link the brand as a **`prefix` alias** (`source = 'auto'`,
  `confidence_bps = 8000 < 10000` — auditable + reversible). Every current and future location
  then resolves through the existing prefix machinery (#199). The blast radius of any error
  is one identity — no transitive avalanche.
- **Where it runs.** `apply_merchant_memory` mints first, so learning groups the discovered
  chains; also exposed as `apply_merchant_grouping` for the import path. Idempotent
  (deterministic v5 ids + `INSERT OR IGNORE`; resolved keys skipped).

**Precision vs recall (honest).** The 24-pair fixture (incl. `PIZZA`/`BURGER`/`TACOS`/`NAILS`
traps) asserts **zero false merges**. Recall is bounded by the city lexicon (a starter set of
~150 populous US cities, pruned of brand-collision tokens like `MOBILE`/`PARIS`/`JACKSON`);
unknown cities are **safe misses**, and the lexicon grows monotonically at no precision cost.

**Deferred (a follow-on, not built here):** richer recall via a larger lexicon, and attaching
a *single-sighting* key to an established anchor (the quorum is ≥2 today). Both are pure recall
gains over the same zero-false-merge gates.

## Addendum 2026-07-09 — Default categories get customizable appearance (`kogu`)

The "system categories are immutable except archive" rule (§2 above) is refined to
distinguish **identity** from **appearance**:

- **Identity — immutable.** A system ("Default") category's name, type, parent, and
  existence stay fixed. `MoveCategory`, archive-as-delete, and hard-delete on a system
  row remain rejected (`apply_move_category` still calls `require_user_category`).
- **Appearance — user-customizable.** A system category's **color and icon (emoji)** are
  now editable. `apply_update_category` no longer rejects system rows; instead, for a
  system row it updates **color + icon only** and **preserves the canonical name** — any
  submitted name is ignored server-side, so the immutability boundary cannot be bypassed
  from the client. User categories are unchanged (full name + color + icon + reparent).
- **Why it's safe.** Alias/import matching keys on identity (name/type/parent), never on
  color or icon, so recoloring or re-iconing a default does not diverge vaults or break
  matching. This directly addresses the rejected "edit in place" concern, which was about
  *name/parent* divergence.

Alongside, **`CreateCategory` gains an optional `icon`** so a new (user) category can be
given its emoji + color at creation, not only via a follow-up edit. The frontend
Categories screen now (a) shows the inline **Edit** affordance on Default categories with
the name read-only and the parent picker hidden, and (b) gives the Add-category form the
same emoji field + native color picker as the editor.

## Addendum (2026-07-11, personal-cfo-4d8.25.21): "Credit Card Payment" is a transfer

Owner dogfooding 2026-07-09: "the default type for Credit Card Payment (under Debt) is
'expense', but isn't this a transfer? The expense was the charge on the credit card, but then
we transfer money from checking to the card to pay off those expenses." The owner is right —
a card payment is money movement (a liquid account → the card liability), not new spend. The
spend already happened as the original charges; typing the payment as `expense` double-counts
it in spend analytics and lets recurring detection surface it as a "bill."

**Decision (owner-confirmed 2026-07-11).** The seeded system category **"Credit Card Payment"
becomes a `transfer`** (`forecast_behavior = ignore_cashflow`) and **moves from the "Debt"
group to the "Transfers" group**, where it sits alongside the other money-movement categories.

This is a deliberate, **one-time** exception to this ADR's system-category
identity-immutability rule (name/type/parent immutable). It is justified because the original
seeding was simply wrong for this leaf, and the correction is safe:

- **Identity/reference stability:** the category **id is unchanged**, so every existing
  transaction categorized as "Credit Card Payment" keeps its assignment; only the category's
  `type`, `parent_id`, and `forecast_behavior` change.
- **Name/alias stability:** the **name is unchanged** ("Credit Card Payment"), so import-alias
  and merchant matching are unaffected.
- **Analytics/detection:** as a `transfer` it is excluded from spend totals and from
  recurring-bill detection (`recurring_detection` already excludes `type = 'transfer'`) — the
  card's statement forecast (ADR 0039) models the payment's cash effect, so counting the
  categorized transaction too would be a double-count.
- **Interaction with ADR 0035 §3:** consistent — a debt payment is an asymmetric asset→liability
  transfer; the category now agrees with the ledger model.

**Migration.** Migration v44 re-types + reparents the seeded leaf in existing vaults, guarded
so it only fires when the seeded expense-typed leaf is still under "Debt" **and** the
"Transfers" group exists (never orphans it). Fresh vaults seed it correctly from
`DEFAULT_TAXONOMY`. The migration rebuilds read models because the type + `forecast_behavior`
change affects spend rollups and forecast spend classification.

**Scope note.** Only "Credit Card Payment" is re-typed. The other "Debt" leaves (Student Loan,
Auto Loan, Personal Loan) stay `expense` for now — loan payments blend a transfer (principal)
and an expense (interest) leg, a finer split the owner did not ask for here.
