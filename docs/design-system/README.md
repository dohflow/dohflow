# DohFlow — Design System kit

A reference for the DohFlow design system **as shipped**, kept in sync with the app.
**Paper** (the Desktop Studio's design file) is the source of truth for design intent; this kit and
the app are the source of truth for what shipped
([ADR 0031, addendum 2026-10-27](../adr/0031-ui-quality-and-design-workflow.md)). Where they
differ, that is a gap recorded as a bead.

## Files

| File | What it is | How to use it |
|------|-----------|---------------|
| [`design-system.md`](./design-system.md) | The human-readable description of what is built: concept, tokens, type, the **component inventory** (and what is not shared yet), the app's structure, and the **Paper to code map**. | Start here. |
| [`design-tokens.json`](./design-tokens.json) | Machine-readable tokens (color/font/radius, light + dark). | Use for exact values. |
| [`settings-layout-review.md`](./settings-layout-review.md) | The engineering specification for the Settings page layout and disclosure behavior (the selected composition). | Read before building or changing Settings. |
| [`recipes.md`](./recipes.md) | Golden copy-pasteable snippets for the token-sensitive components: interactive multi-line chart (shadcn+Recharts), data table, dialog/modal, RHF+Zod form. | Copy when building those so our shadcn API and tokens match exactly. Note the dialog is a recipe, not a shared component. |
| [`dataviz.md`](./dataviz.md) | Chart conventions: the four-slot palette rule, picking a form, mark specs, labels/legends, interaction + a11y (`4d8.27.4.5`, ADR 0054). | **Read before writing any chart code or picking a chart colour.** The palette half is enforced by `styles/chart-palette.test.ts`. |
| [`surface-audit.md`](./surface-audit.md) | Inventory of every collection surface (table / list-card / card-grid), its four states, and whether it migrates to `DataTable` (`4d8.27.4.3`). | Read before adding or restyling a list; it records what each surface already does and why. |
| [`component-gallery.html`](./component-gallery.html) | Self-contained, themeable **static** page rendering shadcn-style components (buttons, cards, KPIs, table, modal, badges, alerts, controls, empty/loading states, chart) in the real tokens. It makes no network requests. | Open in a browser, toggle light/dark, screenshot for comparison. If it disagrees with the app, the app wins. |
| [`screenshots/`](./screenshots/) | Real app screenshots as views ship. | Capture → save → index in `design-system.md` §8 → feed alongside the spec. |

## Source of truth

Tokens mirror [`apps/desktop/src/styles/globals.css`](../../apps/desktop/src/styles/globals.css)
(bead `personal-cfo-x99h`). If they drift, **`globals.css` wins** — update these files to match.

## Keeping it alive

The system **evolves as the app is built**. Each time a real screen lands, drop a
screenshot in `screenshots/`, index it in `design-system.md` §8, and promote any new
recurring pattern into the spec + the gallery. See §8 for the full loop.
