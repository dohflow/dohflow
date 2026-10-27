# Settings layout specification

Status: engineering design specification; application implementation is pending.

This specification defines the responsive layout and interaction behavior for the desktop Settings page. Existing feature callbacks, validation, persistence and financial behavior remain unchanged. The implementation must reconcile its current source revision with the component paths below.

## Layout

Usable width is the main pane width after the sidebar and 24px horizontal insets. Use 16px gutters. Closed tiles require at least 320px: three columns fit from 992px, two from 656px, otherwise one. A 368px content example is a stress fixture, not a supported minimum.

Combined Household / General / Appearance preferences begin expanded. Other healthy sections begin collapsed. Connections, About and the security group span the full content width.

| Group and state | Three columns | Two columns | One column |
| --- | --- | --- | --- |
| Backups / Comfort / categorization, healthy | one / one / one | half / half / full | full |
| Comfort editor open | one / two / full | all full | full |
| Comfort closed with partial failure | half / half / full | half / half / full | full |
| Health / Update / Diagnostics, healthy | one / one / one | half / half / full | full |
| Health problem | full / half / half | full / half / half | full |
| Vaults / conditional setup | half / half | half / half | full |

Preserve source reading and focus order. Do not use dense packing or visual reordering. Equalize collapsed siblings to the tallest natural height in their actual row; do not clip text or impose a global height. Expanded editors retain natural height. Errors grow naturally and retain their controls.

Align parallel preference controls at their top edges while preserving source control sizes. Keep help below controls, let labels and help wrap, and stack lanes when they cannot fit. Preference cards use three lanes from 866px card width, two from 586px, otherwise one; Appearance spans the next row when there are two lanes.

## Controls and components

Use existing semantic theme tokens and source controls. Cards retain an 8px radius, default bodies 24px padding and the delete body 16px padding. Disclosure headers have a 44px minimum, a fixed 16px chevron lane, 12px gap, and 16px semibold text with 24px line height. Source controls retain their 40px, 36px and 32px sizes.

Page composition remains in apps/desktop/src/settings/SettingsView.tsx. The existing HouseholdCard, AppearanceCard, ComfortBandCard, AutoCategorizeOnImportCard, ConnectionsCard, VaultHealthCard, SoftwareUpdateCard, DiagnosticsCard, VaultsCard, ChangePasswordCard, RotateKeyCard, DeleteVaultCard and AboutCard components remain in that settings directory. Backup components are in apps/desktop/src/backup/. Connection flows are in apps/desktop/src/settings/connections/ and NewMappedAccountDialog.tsx is in apps/desktop/src/settings/.

A shared disclosure implementation is required. This document does not choose a library or claim that a shared Accordion component exists. Reuse existing Card, CardFooter, Button, Input, Label, NativeSelect, Combobox, Skeleton, EmptyState, Badge and Notice patterns.

## Interaction and accessibility

Sections expand independently. A heading contains one native trigger button with a stable name, aria-expanded and aria-controls. Enter/Space activates it; Tab follows source order. Use named regions selectively. Before hiding focused content, return focus to its trigger. Hidden bodies leave keyboard and accessibility navigation while ordinary drafts survive.

Errors, complete remedies and relevant actions remain outside hidden bodies. Maintain exactly one active retry. Collapse never submits, discards drafts, cancels work or rolls back a partially persisted result. A Comfort partial failure retains both drafts, the complete source lower-saved/upper-failed alert and one Save action. A failed connection retains Refresh now. A failed timezone change retains its draft and the source Save action outside the hidden body when collapsed. Existing pending and disabled rules continue to apply to the relevant source controls. Save retains its name alongside a decorative 16px progress indicator. Announce state transitions once; errors persist and do not auto-dismiss. Respect reduced motion.

All ancestor disclosures stay expanded while a destructive confirmation is open, armed or irreversibly busy. Focusable aria-disabled triggers ignore collapse attempts. Explicit source Cancel resets the applicable confirmation/password/error and returns focus without collapsing the section. Blocking rotation makes the rest of the application inert and remains non-dismissible, without Escape or Cancel.

Use contextual accessible names Refresh now, [connection name] and Rename [vault name]. Preserve the full existing destructive warnings.

## Display text

Use Household · General · Appearance and Password, encryption and deletion for the combined group headings. Retain existing feature names and source wording.

Conditional summaries reflect actual data: scheduled frequency and verified backup status; formatted Comfort range; On only when enabled; active vault name; connection count; application version. Do not substitute fixed fixture values or claim a healthy/verified result while loading or failed. Progress text retains the action name, including Back up now · Backing up… and Refresh now · Refreshing….

## Verification and implementation limits

Verify natural row edges, parallel control tops and span maps at 992/991px and 656/655px, wide/narrow sizes and both sidebar states. Cover Light/Dark themes, long synthetic names/paths/amounts, absent summaries and open/closed/mixed/error/partial states. Verify keyboard order, focus return, screen-reader names and announcements, reduced motion, retry uniqueness and destructive inertness with fake data.

The known required input-boundary contrast gap must be remediated and measured on rendered surfaces before production accessibility acceptance. Text and required non-text contrast, source/convention reconciliation, shared disclosure implementation and runtime verification remain release requirements. Static design review does not establish runtime accessibility or implementation correctness.
