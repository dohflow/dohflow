# DohFlow — Design System kit

A portable design system for mocking up DohFlow UI in **Claude Design** (or any
mockup tool), kept in sync with the app.

## Files

| File | What it is | How to use it |
|------|-----------|---------------|
| [`design-system.md`](./design-system.md) | The full human-readable spec: concept, tokens, type scale, component conventions, IA, and the screenshot-evolution workflow. | Paste into Claude Design as context. Start here. |
| [`design-tokens.json`](./design-tokens.json) | Machine-readable tokens (color/font/radius, light + dark). | Paste/import for exact values. |
| [`recipes.md`](./recipes.md) | Golden copy-pasteable snippets for the token-sensitive components: interactive multi-line chart (shadcn+Recharts), data table, dialog/modal, RHF+Zod form. | Paste when mocking those components so Claude Design matches our shadcn API + tokens exactly. |
| [`dataviz.md`](./dataviz.md) | Chart conventions: the four-slot palette rule, picking a form, mark specs, labels/legends, interaction + a11y (`4d8.27.4.5`, ADR 0054). | **Read before writing any chart code or picking a chart colour.** The palette half is enforced by `styles/chart-palette.test.ts`. |
| [`surface-audit.md`](./surface-audit.md) | Inventory of every collection surface (table / list-card / card-grid), its four states, and whether it migrates to `DataTable` (`4d8.27.4.3`). | Read before adding or restyling a list; it records what each surface already does and why. |
| [`component-gallery.html`](./component-gallery.html) | Self-contained, themeable page rendering shadcn components (buttons, cards, KPIs, table, **modal**, badges, alerts, controls, empty/loading states, chart) in the real tokens. | Open in a browser, toggle light/dark, screenshot, hand to Claude Design as a visual target. |
| [`screenshots/`](./screenshots/) | Real app screenshots as views ship. | Capture → save → index in `design-system.md` §8 → feed alongside the spec. |

## Source of truth

Tokens mirror [`apps/desktop/src/styles/globals.css`](../../apps/desktop/src/styles/globals.css)
(bead `personal-cfo-x99h`). If they drift, **`globals.css` wins** — update these files to match.

## Keeping it alive

The system **evolves as the app is built**. Each time a real screen lands, drop a
screenshot in `screenshots/`, index it in `design-system.md` §8, and promote any new
recurring pattern into the spec + the gallery. See §8 for the full loop.
