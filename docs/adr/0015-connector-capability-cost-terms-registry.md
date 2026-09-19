# ADR 0015 — Connector capability/cost/terms registry

- **Status:** Accepted
- **Tier:** Public — a data-schema and architecture decision; no DohFlow business
  figures (ADR 0082). Where this ADR names a third-party provider's own public
  cost, that is the same category of fact `docs/research/simplefin-feasibility.md`
  and `docs/research/lunchflow-feasibility.md` already state publicly, not
  DohFlow's own cost structure.
- **Bead:** `personal-cfo-j5d`
- **Slot note:** `docs/adr/README.md` previously listed 0015 among numbers "never
  assigned." That was inaccurate for this number specifically — `personal-cfo-j5d`
  has held the ADR 0015 slot since 2026-05-03, predating the note. Corrected in
  the same PR as this file (owner-confirmed 2026-09-19): 0015 is reused, not
  skipped.
- **Enables:** `personal-cfo-5jjz` (the implementation — extends
  `crates/connector-core` per this ADR's Decision §2)
- **Related:** ADR 0004 (connector relay boundary — the credential tiers this
  registry classifies each provider against), ADR 0060 (SimpleFIN-first
  strategy + the onboarding disclosure panel this registry's disclosure text
  feeds), ADR 0076 (`personal-cfo-m0kgx`, multi-provider strategy — decides
  *which* providers ship and the affiliate/FTC stance; does not redefine this
  registry's shape), ADR 0022 (parser/document isolation — the no-runtime-
  dynamic-loading precedent this ADR's Rust-vs-TOML decision follows)

## Context

`crates/connector-core` already has a capability model:
[`Capability`](../../crates/connector-core/src/lib.rs) (`Accounts`,
`Transactions`, `Balances`, `Holdings`, `Liabilities`),
[`CapabilitySet`](../../crates/connector-core/src/lib.rs) (which of those an
adapter actually supports), `CapabilitySet::ensure` (a typed
`ConnectorError::CapabilityMissing` when feature code asks for a capability
an adapter lacks), and a compile-time registry —
`register_connector!`/`all_connectors()`/`connector_by_id()`, built on
`inventory`, mirroring `importer-core`'s own registry. The shipped SimpleFIN
adapter (`crates/connectors/simplefin-adapter`) registers concretely:
`id() == "simplefin"`, `capabilities()` declaring accounts/transactions/
balances true, holdings/liabilities false ("the Bridge emits holdings, but no
staged shape exists yet" — the adapter's own comment).

What does **not** exist yet, and is the actual subject of this ADR: which
ADR 0004 credential tier a provider uses, what account types and
countries/regions it serves, who pays for it and how much, what the user must
be told before connecting, whether it's enabled for real users at all, and a
review cadence keeping the cost/terms facts from silently rotting. None of
that is a capability in the `Capability` sense — a provider can support
`Transactions` and still be the wrong provider to enable today because its
terms haven't been reviewed in eight months. `personal-cfo-5jjz` is the
bead that builds this; this ADR decides its shape first (AGENTS.md §1A),
because the picker (`personal-cfo-m0kgx`'s `C.6`) and the disclosure panel
both read from it and neither should invent the shape implicitly.

Risk `o12w` (provider cost/API changes going unnoticed, P2) is the immediate reason a
review cadence must be part of the registry rather than a separate manual
process: a provider that quietly changes its cost or terms and nothing in
the codebase notices is exactly this risk realized.

## Decision

### 1. The registry lives in typed Rust, compiled into the binary — not a TOML file, not a database table

Every other adapter-facing surface in this codebase resolves at compile time:
`register_connector!`'s `inventory`-based collection, `importer-core`'s mirror
registry, and ADR 0022's explicit rejection of runtime dynamic loading for
parsers. A TOML file loaded at runtime would be the one provider-facing
surface in the app that *isn't* compiled in, reviewable in a PR diff, and
guaranteed present at every build — for no benefit, since providers change
rarely enough that a `cargo build` is not a meaningful release friction. The
registry entry lives beside each adapter's `register_connector!` call, in the
same crate, reviewed the same way the adapter code itself is.

### 2. Shape: a `ConnectorMetadata` struct alongside every registration

`ConnectorRegistration` (today: `adapter: &'static dyn ConnectorAdapter`)
gains a second field, `metadata: ConnectorMetadata`, populated at the same
`register_connector!` call site as the adapter itself:

```rust
pub struct ConnectorMetadata {
    pub tier: CredentialTier,              // §3
    pub account_types: &'static [AccountType],
    pub regions: &'static [&'static str],  // ISO 3166-1 alpha-2, or "US" as today's only value
    pub economics: ConnectorEconomics,     // §4
    pub disclosure: DisclosureText,        // §5
    pub enabled: bool,                     // §6
}
```

`Capability`/`CapabilitySet` are unchanged and stay on the adapter trait
itself (`fn capabilities(&self) -> CapabilitySet`) — they are a runtime
property of what the adapter's code can actually do, proven by
`capability_mismatch.rs`'s tests, not a static fact about the provider's
business terms. `ConnectorMetadata` is the new, purely-static half.

### 3. Credential tier, account types, regions

`CredentialTier` names ADR 0004 §2/§3's three tiers exactly:
`UserToken`, `ByoCredential`, `Relay`. This is what the picker (`m0kgx`'s
`C.6`) reads to decide whether "connect" means "paste a token" or "open the
system browser" before any adapter-specific code runs. `account_types` and
`regions` are the "what this provider actually covers" facts a picker needs
before showing a provider as an option at all — SimpleFIN today would
declare `regions: &["US"]`; a future LunchFlow adapter would declare its
much broader region list per `docs/research/lunchflow-feasibility.md`.

### 4. `ConnectorEconomics` — who pays, how much, and how stale that fact is allowed to get

```rust
pub struct ConnectorEconomics {
    pub payer: Payer,                          // UserDirect | DohflowBrokered | None
    pub base_cost_minor_units: Option<u32>,     // e.g. 150 = $1.50, in the currency below
    pub currency: Option<&'static str>,         // ISO 4217, e.g. "USD"
    pub billing_period: Option<BillingPeriod>,  // Monthly | Annual
    pub included_connections: Option<u32>,
    pub extra_connection_cost_minor_units: Option<u32>,
    pub extra_connection_period: Option<BillingPeriod>,
    pub cost_reviewed_at: chrono::NaiveDate,
    pub terms_url: Option<&'static str>,
    pub terms_reviewed_at: chrono::NaiveDate,
    pub history_depth_expectation: &'static str, // free-text, e.g. "~2-6 months at first link"
}
```

`Payer::None` covers file import — no provider, no cost, and this is the
only variant every other field is allowed to be `None`/empty under. The
`_minor_units` shape (not a float) avoids the classic money-as-float bug;
`base_cost_minor_units`/`extra_connection_cost_minor_units` are independent
fields with independent billing periods because real providers (LunchFlow,
per `docs/research/lunchflow-feasibility.md`) publish genuinely different
per-connection cadences for their monthly vs. annual plans — a single shared
`billing_period` field would silently misrepresent one of them.
`history_depth_expectation` is free text, not a typed duration, because it
answers `personal-cfo-ef4q`'s question ("what should the user expect on
first link") with provider-specific nuance ("2-6 months, institution-
dependent" for SimpleFIN) that a single numeric field can't carry honestly.

### 5. `DisclosureText` — what the user is told before any credential is entered

```rust
pub struct DisclosureText {
    pub independent_party: &'static str,
    pub handles_credentials: &'static str,
    pub cost_summary: &'static str,
    pub optional: &'static str,
}
```

Four fields, not one blob, matching the four fixed points ADR 0060's
onboarding addendum (`personal-cfo-kdw6`) already established for the
disclosure panel verbatim: independent and unaffiliated · handles the user's
bank credentials under its own terms · costs money paid to them · optional.
The picker (`m0kgx`'s `C.6`) renders these before any token entry field, the
same way the existing Settings Connections card does today for SimpleFIN —
this registry makes that pattern data-driven per provider instead of
hardcoded to one.

### 6. `enabled: bool` — a provider can be fully implemented and still off

An adapter ships with `enabled: false` in its `ConnectorMetadata` until a
release bead flips it once ADR 0076's ship conditions are met (dedupe across
providers, currency-refusal, whatever else that ADR names). `connector_link`
(`apps/desktop/src-tauri/src/ipc/commands.rs`) gains a check before calling
`ConnectorAdapter::link` at all: a disabled adapter id refuses with a typed
`IpcError` (the same `Validation` variant `connector_link_impl` already uses
for "this adapter requires browser-based auth, which is not supported yet" —
a disabled-provider refusal is the same shape of "not available right now,"
not a new error category) rather than reaching the adapter and failing
there. `personal-cfo-5jjz` implements the check; this ADR fixes that it must
exist and where.

### 7. Review cadence — a CI warning, not a failure, past six months

`cost_reviewed_at` and `terms_reviewed_at` are reviewed at least at every
release that touches connectors, and at most six months apart regardless. A
review date older than six months is a **CI warning**, not a build failure —
unlike `dohflow-site`'s compare/migrate 120-day *hard* limit
(`personal-cfo-y0o0x`), a stale connector-registry date does not make a
false claim publicly visible the way a stale public compare page does; it's
an internal signal that a human should re-check a provider's terms, and a
hard failure over it would block unrelated releases. This is the mitigation
risk `o12w` names: provider cost/API changes are not something the
codebase can detect automatically, but a review date going quiet for six
months is.

### 8. Capability mismatch stays a typed error, unchanged

`CapabilitySet::ensure` already does this — proven today by
`crates/connector-core/tests/capability_mismatch.rs`'s three tests (feature
code checking capabilities up front, a direct `fetch_*` call against a
missing capability, and the default `sync()` composition silently skipping
fetches the adapter can't do rather than crashing). This ADR does not change
that mechanism; `ConnectorMetadata` is additive, sitting beside
`CapabilitySet`, not replacing it.

### 9. Invariant: a provider's cost, terms, or API changing never breaks manual mode

Manual entry and file import have no `ConnectorMetadata` dependency at all —
`Payer::None`, no adapter, no registry entry. Invariant 5 (manual-mode-must-
work, plan §1.5) holds structurally: nothing in this registry's shape can be
on the critical path for the app's core functionality, only for the optional
connectors layered on top of it.

## Rejected alternatives

- **Assume-everything-works connector contract** (no typed capability/cost
  declaration at all) — rejected: providers vary too widely in what they
  actually support and charge for the picker or the disclosure panel to
  guess correctly; the whole point of `personal-cfo-o12w`'s risk is that
  providers drift, and an undeclared contract can't be checked against
  anything.
- **Runtime capability detection** (probe the provider at connect time to
  discover what it supports) — rejected: too late. The picker needs to know
  a provider's tier, region coverage, and cost *before* the user attempts to
  connect, not after a failed probe; ADR 0004's disclosure requirement is
  explicitly pre-credential-entry.
- **A database table as the registry** (`personal-cfo-07u`'s shape, applied
  here) — rejected: a static registry needs no schema, no migration, and no
  query path; it ships in the binary the same way `Capability`/
  `CapabilitySet` already do. A database table would also put provider
  cost/terms facts in the same class of data as user financial records,
  which they are not — they're app configuration, not vault content.

## Consequences

- **Positive.** `personal-cfo-5jjz` has a concrete shape to implement against
  rather than inventing one mid-implementation. The picker (`m0kgx`'s `C.6`)
  and the disclosure panel both become data-driven per provider instead of
  each hardcoding SimpleFIN's specifics.
- **Positive.** `ConnectorMetadata` is purely additive — no change to
  `ConnectorAdapter`, `Capability`, `CapabilitySet`, or any of the three
  existing capability tests. `5jjz` should not need to touch
  `capability_mismatch.rs`.
- **Negative / accepted.** Every adapter now carries two static declarations
  (capabilities on the trait, metadata on the registration) instead of one.
  Judged worth it: conflating "what the code can fetch" with "what the
  provider charges and requires" would make the capability type do two
  unrelated jobs.
- **Negative / accepted.** The six-month review cadence is enforced by a CI
  *warning*, which a maintainer can ignore longer than six months if
  nobody's watching CI output closely. Accepted because the alternative (a
  hard failure) blocks releases that have nothing to do with the stale
  provider, and `personal-cfo-o12w` already frames this as a monitored risk,
  not a release gate.

## Revisit if

- A connector class breaks the typed-capability assumption entirely — e.g. a
  streaming-only provider whose "capabilities" aren't a fixed set known at
  registration time.
- A provider becomes DohFlow-brokered (the managed-relay mode, ADR 0004 §3
  mode 3) — `Payer::DohflowBrokered` was named in anticipation of this but
  has no real user yet; when one exists, confirm the economics shape still
  fits a brokered provider's actual billing relationship (DohFlow may be the
  one setting the cost, not just relaying a third party's).
