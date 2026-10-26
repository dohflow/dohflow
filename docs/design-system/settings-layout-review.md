# Settings design review candidate

Status: **RC1 proposal; independent screen review and owner decisions pending.**
This record describes a design artifact, not an accepted frontend convention or
an implementation handoff.

- Task: `personal-cfo-twpo.3`.
- [Paper candidate](https://app.paper.design/file/01M5TNGF0SVKWW6WQCTF8Z41H0/p-1-0).
- Frozen: **October 25, 2026, 5:07:29 PM Pacific** (2026-10-26 00:07:29 UTC).
- Source: `e8e6ed00389278bb14af8b88d92097b5f61cda4d`.
- Design-system audit baseline: v0.2, frozen October 25, 2026,
  4:18:31 PM Pacific (23:18:31 UTC).
- Baseline token hash: `0afec932`; candidate-local token hash: `a378c7e0`.
  All 129 copied token names and values were compared and match. The files do
  not have synchronized component instances.

## Proposed layout

Recovery and connection health come first, preferences next, maintenance and
help below, and security actions last. All sixteen current cards remain.
The artifact uses synthetic demo data and retains the source help and
irreversible-action explanations.

The proposed container policy uses remaining main content width `C`, after
the sidebar and the existing 24px horizontal main padding on each side:

| Content width | Layout |
| --- | --- |
| Below 480px | One column; Comfort band fields stack. |
| 480–735px | One column; Comfort band fields may share a row. |
| 736–1007px | Small preference/help cards can pair; recovery cards remain full width. |
| 1008px and above | Backups and Connections can also pair, each at least 496px. |

The proposal retains 16px gaps, 24px default card padding, the existing 16px
Vaults/Delete padding, and the source control sizes. It uses stable DOM order,
wrapping actions, and no dense grid packing or masonry. Full names remain
available without hover. It does not introduce a new shared primitive.

## Coverage and component mapping

ST frames contain complete default cards. S frames show state deltas to those
cards; their unchanged descriptions, warnings, and confirmation requirements
remain present. The paired state sheets index 126 coverage variants per theme,
including four existing password-strength buckets and explicit N/A cases.
These are static design specifications, not runtime test results.

Paths below are relative to `apps/desktop/src`.

| State index | Existing composition |
| --- | --- |
| S01 | `backup/BackupScheduleCard.tsx`, `backup/BackupView.tsx` |
| S02 | `settings/ConnectionsCard.tsx`, `settings/connections/*`, `settings/NewMappedAccountDialog.tsx` |
| S03 | `settings/VaultHealthCard.tsx` |
| S04 | `settings/HouseholdCard.tsx`, `components/ui/combobox.tsx` |
| S05 | General in `settings/SettingsView.tsx` |
| S06 | `settings/AppearanceCard.tsx` |
| S07 | `settings/ComfortBandCard.tsx` |
| S08 | `settings/AutoCategorizeOnImportCard.tsx` |
| S09 | `settings/SoftwareUpdateCard.tsx` |
| S10 | `settings/DiagnosticsCard.tsx` |
| S11 | `settings/VaultsCard.tsx` |
| S12 | Setup guide in `settings/SettingsView.tsx` |
| S13 | `settings/AboutCard.tsx`, `brand/Brand.tsx` |
| S14 | `settings/ChangePasswordCard.tsx`, `vault/PasswordStrengthMeter.tsx` |
| S15 | `settings/RotateKeyCard.tsx` |
| S16 | `settings/DeleteVaultCard.tsx` |

The compositions use the existing local Card, Button, Input, Label,
NativeSelect, Combobox, Skeleton, EmptyState, Badge and Notice patterns.
Appearance radios and the native auto-categorize checkbox remain feature
compositions. Connection dialogs remain the existing feature compositions.

R frames prove the remaining-width layouts at 1280×800 and 800×600 with
224px/56px sidebar space, plus a 640×600 stress case. The latter is **not a
supported-minimum claim**. The sidebar is schematic context in these proofs:
its icons, collapse control, support action and build footer are omitted.
No sidebar change is proposed. ST frames supply the complete scrolled content.

## Copy and accessibility gates

Source strings are evidence, not blanket copy approval. ST-00 records the
provenance and proposed decisions. The new visible section headings are marked
PROPOSED. Contextual accessible names, long-name wrapping, consistent title
size, and retaining the existing Save label while busy are also explicit
proposals. Account mapping labels remain visually hidden in the product;
the state sheets print their accessible names as annotations.

Notifications remain first in main. The existing persistent connection
attention notice precedes the page title. The Backups focus destination remains
`settings-backups`. Reading order and keyboard focus order agree at every
column count. The specification preserves existing picker, native dialog and
blocking-rotation behavior and requires modal focus containment, background
inertness and focus return. It does not claim the current implementation has
passed those checks.

The dark-theme review corrected SVG paint bindings and diagnostics preview text.
Token calculations give text/card contrast of 17.63:1 light and 15.34:1 dark;
muted text/card is 5.97:1 and 6.69:1. Primary and destructive button text also
clear 4.5:1 in both themes. These calculations are **not a blanket accessibility
PASS**. Required input boundaries remain a known source gap: 1.27:1 light and
1.46:1 dark against the input background. The proposed neutral text plus
semantic mark treatment needs independent review and approval.

Provider wording and historical economics in F00/F01/F05 come from the
source registry fixture; they are not current commercial claims. Diagnostics
uses synthetic test-fixture JSON. No real financial data, paths or credentials
were used.

## Exact frozen frame manifest

A review binds to this freeze and these names. Any Paper edit requires a new
freeze and review. The owner approves layout/copy and performs merges; an
artifact review does not authorize implementation.

| Page | Frame |
| --- | --- |
| Page 1 | ST-00 · RC1 review manifest and specifications |
| Page 1 | ST-01 · Settings full content · Wide Light · RC1 |
| Page 1 | ST-02 · Settings full content · Wide Dark · RC1 |
| Page 1 | ST-03 · Settings full content · C528 Light · RC1 |
| Page 1 | ST-04 · Settings full content · C528 Dark · RC1 |
| Page 1 | ST-05 · Settings full content · C368 Light · stress · RC1 |
| Page 1 | ST-06 · Settings full content · C368 Dark · stress · RC1 |
| 02 · States and accessibility | S01 · Backups state deltas · Light · RC1 |
| 02 · States and accessibility | S02 · Connections state deltas · Light · RC1 |
| 02 · States and accessibility | S03 · Vault health state deltas · Light · RC1 |
| 02 · States and accessibility | S04 · Household state deltas · Light · RC1 |
| 02 · States and accessibility | S05 · General state deltas · Light · RC1 |
| 02 · States and accessibility | S06 · Appearance state deltas · Light · RC1 |
| 02 · States and accessibility | S07 · Comfort band state deltas · Light · RC1 |
| 02 · States and accessibility | S08 · Auto-categorize imported transactions state deltas · Light · RC1 |
| 02 · States and accessibility | S09 · Software update state deltas · Light · RC1 |
| 02 · States and accessibility | S10 · Diagnostics state deltas · Light · RC1 |
| 02 · States and accessibility | S11 · Vaults state deltas · Light · RC1 |
| 02 · States and accessibility | S12 · Setup guide state deltas · Light · RC1 |
| 02 · States and accessibility | S13 · About state deltas · Light · RC1 |
| 02 · States and accessibility | S14 · Change master password state deltas · Light · RC1 |
| 02 · States and accessibility | S15 · Rotate encryption key state deltas · Light · RC1 |
| 02 · States and accessibility | S16 · Delete this vault state deltas · Light · RC1 |
| 01 · Responsive windows | R01 · 1280×800 · Sidebar 224 · Light · RC1 |
| 01 · Responsive windows | R02 · 1280×800 · Sidebar 56 · Light · RC1 |
| 01 · Responsive windows | R03 · 800×600 · Sidebar 224 · Light · RC1 |
| 01 · Responsive windows | R11 · 800×600 · Existing persistent attention · Light · RC1 |
| 01 · Responsive windows | R12 · 800×600 · Existing persistent attention · Dark · RC1 |
| 01 · Responsive windows | R04 · 800×600 · Sidebar 56 · Light · RC1 |
| 01 · Responsive windows | R05 · 1280×800 · Sidebar 224 · Dark · RC1 |
| 01 · Responsive windows | R06 · 1280×800 · Sidebar 56 · Dark · RC1 |
| 01 · Responsive windows | R07 · 800×600 · Sidebar 224 · Dark · RC1 |
| 01 · Responsive windows | R09 · 640×600 · Sidebar 224 · Light · stress, not supported minimum · RC1 |
| 01 · Responsive windows | R10 · 640×600 · Sidebar 224 · Dark · stress, not supported minimum · RC1 |
| 01 · Responsive windows | R08 · 800×600 · Sidebar 56 · Dark · RC1 |
| 03 · Dark state counterparts | S01 · Backups state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S02 · Connections state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S03 · Vault health state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S04 · Household state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S05 · General state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S06 · Appearance state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S07 · Comfort band state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S08 · Auto-categorize imported transactions state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S09 · Software update state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S10 · Diagnostics state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S11 · Vaults state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S12 · Setup guide state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S13 · About state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S14 · Change master password state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S15 · Rotate encryption key state deltas · Dark · RC1 |
| 03 · Dark state counterparts | S16 · Delete this vault state deltas · Dark · RC1 |
| 04 · Connection and blocking flows | F01 · Provider disclosure · Light · RC1 |
| 04 · Connection and blocking flows | F05 · Credential step after disclosure · Light · RC1 |
| 04 · Connection and blocking flows | F02 · Create and map dialog · Light · RC1 |
| 04 · Connection and blocking flows | F03 · Shared-feed confirmation · Light · RC1 |
| 04 · Connection and blocking flows | F04 · Blocking rotation · Light · RC1 |
| 04 · Connection and blocking flows | F01 · Provider disclosure · Dark · RC1 |
| 04 · Connection and blocking flows | F05 · Credential step after disclosure · Dark · RC1 |
| 04 · Connection and blocking flows | F02 · Create and map dialog · Dark · RC1 |
| 04 · Connection and blocking flows | F03 · Shared-feed confirmation · Dark · RC1 |
| 04 · Connection and blocking flows | F04 · Blocking rotation · Dark · RC1 |
| 04 · Connection and blocking flows | F00 · Provider picker · source registry fixture · Light · RC1 |
| 04 · Connection and blocking flows | F00 · Provider picker · source registry fixture · Dark · RC1 |

## Verification

- Screenshot checkpoints covered layout, wrapping, alignment, type, theme and
  clipping, with targeted repairs before this freeze.
- Token name/value parity was checked against the pinned system baseline.
- Text-color and SVG-theme binding audits covered the candidate roots.
- The application was not launched. No application code, package or dependency
  changed, so application test/build suites were not run.

