# Connector provider registry

How DohFlow knows what each bank-connection provider is, what it costs and
who pays, what the user must be told before connecting, and whether it can be
used at all, and how the Connections picker turns that into a connect flow.

- **Decisions:** [ADR 0015](../adr/0015-connector-capability-cost-terms-registry.md)
  (the registry's shape, plus its 2026-09-27 referral and link-guide addenda) and
  [ADR 0076](../adr/0076-multi-provider-connector-strategy.md) (which providers
  enter the free app, the ship conditions, the picker order and the affiliate
  stance).
- **Built by:** `personal-cfo-5jjz` (the registry), `personal-cfo-r2pow` (the
  second adapter, LunchFlow, registered disabled) and `personal-cfo-dto2j` (the
  picker).
- **Scope:** describes what is merged. It introduces no provider, cost claim
  or enablement.

## 1. The registry is compiled Rust, not configuration

There is no runtime registry file, TOML or otherwise, and no database table
(ADR 0015 §1). Each adapter crate registers itself at compile time, together
with its metadata, in one call:

```rust
register_connector!(SIMPLEFIN, SIMPLEFIN_METADATA);
```

`register_connector!` takes the adapter **and** its `ConnectorMetadata`, so an
adapter cannot be registered without a registry entry. Registrations are
collected by `inventory` into a static list, the same mechanism the file
importers use.

Lookups (`crates/connector-core/src/lib.rs`):

| Function | Returns |
|---|---|
| `all_registrations()` / `registration_by_id(id)` | the adapter and its metadata |
| `all_connectors()` / `connector_by_id(id)` | the adapter alone |
| `validate_registrations(..)` | every consistency problem, each naming the adapter |

## 2. What an entry holds

A provider is described in two halves (ADR 0015 §2):

- **What the code can fetch** stays on the adapter trait:
  `ConnectorAdapter::capabilities()` returns a `CapabilitySet` (accounts,
  transactions, balances, holdings, liabilities). An adapter never declares a
  capability it can't serve; holdings stay off until the staged holdings shape
  exists (`personal-cfo-kmw5`).
- **What the provider is** lives in `ConnectorMetadata`:

| Field | Meaning |
|---|---|
| `tier: CredentialTier` | ADR 0004's tier: `UserToken`, `ByoCredential` or `Relay` |
| `account_types: &[AccountType]` | coarse coverage: `Depository`, `Credit`, `Loan`, `Investment` |
| `regions: &[&str]` | ISO 3166-1 alpha-2 codes the provider serves |
| `economics: ConnectorEconomics` | who pays (`Payer`), base cost and `BillingPeriod`, included connections, per-extra-connection cost and its own period, the currency, `cost_reviewed_at`, `terms_url`, `terms_reviewed_at`, and a free-text `history_depth_expectation` |
| `disclosure: DisclosureText` | the four points shown before any credential: `independent_party`, `handles_credentials`, `cost_summary`, `optional` |
| `link_guide: ConnectorLinkGuide` | how to connect: panel `title`, `refresh_note`, `setup_steps`, `provider_url`, and the credential's `credential_label` / `credential_noun` / `credential_placeholder` / `paste_instructions` |
| `referral: Option<ConnectorReferral>` | a referral URL and its FTC sentence, when DohFlow may earn from the provider (ADR 0076 §5) |
| `enabled: bool` | whether users can connect it today (§4) |

Money is integer minor units, never a float. Every text field is written **per
provider**. There is no shared default wording, and in particular no shared
"independent and unaffiliated" line: whether that sentence is true is a fact
about one provider, and false for one DohFlow has a referral relationship with.

`validate_registrations` rejects an inconsistent entry. It covers:

- **Ids and coverage:** duplicate adapter ids; empty or non-ISO regions; no
  account types.
- **Cost:** cost fields on a free (`Payer::None`) provider; a paid provider
  without a base cost; a cost without its period or currency.
- **Copy:** an empty disclosure point or link-guide field.
- **URLs and terms:** non-https provider or referral URLs; a referral without
  its disclosure; an enabled provider without a terms URL.

## 3. Review cadence

`cost_reviewed_at` and `terms_reviewed_at` record when a human last checked
the provider's own pages. ADR 0015 §7 allows six months
(`REVIEW_CADENCE_MONTHS`). `ConnectorEconomics::overdue_reviews(today)` reports
which reviews are overdue, with the date injected for tests. CI's **"Connector
registry review cadence (warning only)"** step runs
`cargo run -p connector-core --example registry_review`. That prints a GitHub
Actions warning for each overdue review and always passes: a stale review asks
a human to re-check, and never blocks an unrelated release.

## 4. Enabled versus implemented-but-disabled

A provider can be fully implemented and still unusable. Two states matter:

| State | Registered? | In the picker? | `connector_link` |
|---|---|---|---|
| **Enabled** | yes | yes | links it |
| **Implemented, disabled** | yes | no | refuses it, before the adapter's `link` runs, with `IpcError::Validation` ("not available yet") |

New adapters ship disabled (ADR 0076 decision 2). A release bead flips the flag
only once ADR 0076 decision 3's ship conditions hold: cross-provider dedupe,
the currency refusal, help and disclosure placements, and the threat model's
egress row.

At the time of writing, **SimpleFIN is enabled** and **LunchFlow is
implemented but disabled**. Its release bead is `personal-cfo-p3f7r`.

The refusal is `connector_link_registered_impl`
(`apps/desktop/src-tauri/src/ipc/commands.rs`), the only path the
`connector_link` command takes. Because it refuses before `link`, a disabled
provider's adapter never receives a token.

## 5. The read-only registry IPC

`connector_adapters` returns every registered provider, disabled ones included
and flagged, as `ConnectorAdapterDto[]`. It:

- reads compiled-in configuration only, and needs no vault;
- is granted in `apps/desktop/src-tauri/permissions/app-commands.toml` and
  kept in lockstep by `tests/acl_coverage.rs`;
- returns the registry in **ADR 0076 §4's order**: SimpleFIN first as the
  launch provider, then every other provider alphabetically by display name.
  Nothing in an entry can change its position, and a referral in particular
  cannot.

**The DTO carries no secret.** The registry holds no credentials. The IPC test
walks every serialized key and rejects anything secret-shaped, allowing by exact
name only the fields that hold wording *about* a credential
(`handles_credentials` and the link guide's `credential_*` fields). The
frontend reads the registry through `useConnectorAdapters`, a TanStack Query
hook that never refetches the static list.

## 6. The picker and disclosure flow

`apps/desktop/src/settings/connections/` holds one flow, used by both the
Settings Connections card and onboarding's "Connect my banks" step. There is
exactly one link surface (ADR 0060's addendum).

1. **Picker** (`ProviderPicker`) — enabled providers only, in registry order.
   Each entry shows its capability chips, a cost line naming who pays, when its
   terms were reviewed (with the terms URL), and its countries. It is skipped
   when exactly one provider is enabled.
2. **Disclosure** (`ProviderDisclosure`) — the panel `title`, then the four
   disclosure points and the `refresh_note`, then the setup steps. Where the
   entry carries a referral, the referral URL and its FTC sentence appear
   together, and only then.
3. **Credential form** (`ConnectProviderFlow`) — appears only after
   **Continue**. No credential field exists in the DOM before the user has seen
   the disclosure. The field's label, placeholder and help come from the link
   guide. The pasted value lives in component state only until submit, is
   zeroed on every path, and goes straight to `connector_link` with the chosen
   adapter id.

Every URL is **selectable text, never a link**, because the app's only opener
grant is scoped to `https://dohflow.app/*` (ADR 0010). The connection rows, the
card's description and empty state, and the "forget" wording all take the
provider's name and credential noun from the registry. The Connections UI
carries no provider-specific copy.

SimpleFIN's panel reads exactly as the onboarding panel it replaced
(`personal-cfo-kdw6`). That panel's rendered text was frozen before the change
in `apps/desktop/src/settings/connections/fixtures/simplefin-panel.shipped.txt`.
It is pinned in two links:

- `apps/desktop/src-tauri/tests/connector_registry.rs` checks that every
  registry string appears in the frozen text, and that the frontend's SimpleFIN
  fixture equals what the registry emits;
- `ProviderDisclosure.test.tsx` checks that rendering that fixture reproduces
  the frozen text byte for byte.

## 7. Adding a provider

A new adapter goes through the same gates as LunchFlow did. None of these steps
enables it.

1. **ADR first.** Check the provider against ADR 0076 decision 1: a user-token
   or BYO tier, where the user pays the provider directly. A relay-tier
   provider is not a free-app provider.
2. **The crate:** `crates/connectors/<id>-adapter`. Its id is the registry key
   and the `source_type` token, and never changes (ADR 0076 decision 9). Add it
   to the workspace members, to `crates/connector-core`'s dev-dependencies, and
   to the `SHIPPED` list in `crates/connector-core/tests/registry.rs`. A test
   fails if a workspace adapter crate is missing from that roster.
3. **The schema token:** a reviewed migration that widens the
   `source_batches.source_type` CHECK (ADR 0076 decision 8). Migration 53 is
   the pattern: a table rebuild run with `foreign_keys_off`, with up/down tests.
4. **The entry:** `register_connector!(ADAPTER, METADATA)` with
   `enabled: false`. Write every disclosure point and link-guide field for
   *this* provider. If DohFlow may earn from it, fill `referral` with its FTC
   sentence (ADR 0076 §5), and don't call it unaffiliated. Take cost and terms
   from the provider's live pages, with the review dates of that check.
5. **The threat model:** add the provider's pinned endpoint to TB3 and a row to
   the threats table in `docs/security/threat-model.md` (ADR 0076 decision 3d).
6. **Tests:**
   - the adapter's own fixture tests;
   - the registry roster and validation tests;
   - a check that `connector_link` refuses the provider while it is disabled;
   - an env-gated live drill modelled on
     `apps/desktop/src-tauri/tests/lunchflow_drill.rs`.
7. **Enabling** is a separate release bead. It flips `enabled` only after ADR
   0076 decision 3's conditions hold, re-checks the review dates, and has the
   owner approve the disclosure and link-guide wording.

When a provider's cost or terms change, update its `economics` and the matching
review date in one reviewed change. The review-cadence warning (§3) is the
reminder when nobody has looked for six months.

## 8. Where the contract is tested

| Guarantee | Test |
|---|---|
| Every shipped adapter has exactly one entry; only SimpleFIN is enabled; LunchFlow ships disabled with its referral; every workspace adapter crate is in the roster | `crates/connector-core/tests/registry.rs` |
| Inconsistent metadata is rejected; the six-month boundary | unit tests in `crates/connector-core/src/lib.rs` |
| A disabled provider is refused before link (mock and the real registry); the DTO has no secret fields; disabled entries are listed and flagged; registry order | `apps/desktop/src-tauri/tests/connector_ipc.rs` |
| SimpleFIN's copy in the registry is the shipped text; the frontend fixture matches the registry | `apps/desktop/src-tauri/tests/connector_registry.rs` |
| The picker shows enabled entries only, in order, with details; one provider skips it; disclosure comes before any credential field; referral slot; no links | `apps/desktop/src/settings/connections/*.test.tsx` |
| The command is granted | `apps/desktop/src-tauri/tests/acl_coverage.rs` |
