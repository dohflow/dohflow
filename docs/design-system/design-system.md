# DohFlow — Design System

> **For Claude Design (and any UI mockup tool).** This is the single source of
> truth for mocking up DohFlow screens. Paste this file plus
> [`design-tokens.json`](./design-tokens.json) into your design tool as context,
> and use [`component-gallery.html`](./component-gallery.html) (open it in a
> browser, screenshot it) as the visual reference for how components should look.
>
> Canonical implementation: [`apps/desktop/src/styles/globals.css`](../../apps/desktop/src/styles/globals.css)
> (bead `personal-cfo-x99h`). If this doc and `globals.css` ever disagree, **`globals.css` wins** — fix this doc.

---

## 1. Concept

DohFlow is a **private, local-first personal finance desktop app**. The visual
language is calm, trustworthy, and warm — closer to a well-made ledger than a
neon fintech dashboard.

- **Primary:** deep emerald green, anchored on `#006341`. Signals money, growth, trust.
- **Accent:** Saltillo **terracotta** `#db8f6b` — warmth and highlights in UI chrome. Charts use its own re-stepped slot `--chart-2`, not this token (ADR 0054).
- **Neutrals:** warm grays (slightly brown-tinted), not cold blue-grays. The light
  background is a warm off-white `#faf8f6`.
- **Finance semantics:** dedicated `gain` / `loss` / `warning` / `info` colors so
  money always reads correctly at a glance.
- **Dark mode is a token swap**, not a redesign — the green brightens (`#12a06b`)
  for contrast on dark surfaces; everything else maps 1:1.

**Component basis:** shadcn/ui (Radix primitives + Tailwind + `class-variance-authority`),
themed entirely through the CSS-variable tokens below. Mock everything as shadcn
components — that is what the app is built from.

---

## 2. Typography

- **Family:** Inter (variable build, self-hosted via `@fontsource-variable/inter`).
  Stack: `"Inter Variable", ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`.
- **No separate mono font.** Numbers use Inter with **`tabular-nums`**.
- **Tabular numerals are global** (`font-variant-numeric: tabular-nums` on `body`)
  so currency columns align. Always render money/quantities with tabular figures.
- Rendering: `-webkit-font-smoothing: antialiased`.

### Type scale (Tailwind defaults — what the app uses)

| Token | Size | Line height | Typical use |
|-------|------|-------------|-------------|
| `text-xs`   | 0.75rem  | 1rem    | Captions, table meta, badges |
| `text-sm`   | 0.875rem | 1.25rem | Body default, inputs, buttons, descriptions |
| `text-base` | 1rem     | 1.5rem  | Emphasized body |
| `text-lg`   | 1.125rem | 1.75rem | Card titles, section labels |
| `text-xl`   | 1.25rem  | 1.75rem | View headings |
| `text-2xl`  | 1.5rem   | 2rem    | Page titles, hero metric values |
| `text-3xl`  | 1.875rem | 2.25rem | KPI / big-number displays |

**Weights:** 400 normal (body), 500 medium (buttons, labels), 600 semibold (titles).
`CardTitle` uses `font-semibold leading-none tracking-tight`.

---

## 3. Color tokens

Every color is a CSS variable, exposed to Tailwind as a theme color (so
`bg-primary`, `text-muted-foreground`, `ring-ring`, `text-gain`, etc. resolve).
**Never hard-code a hex in UI** — always use the token utility.

### Core (surfaces, text, controls)

| Token / utility | Light | Dark |
|-----------------|-------|------|
| `background`            | `#faf8f6` | `#0e1411` |
| `foreground`            | `#1c1815` | `#eef2ef` |
| `card`                  | `#ffffff` | `#151c18` |
| `card-foreground`       | `#1c1815` | `#eef2ef` |
| `popover`               | `#ffffff` | `#151c18` |
| `popover-foreground`    | `#1c1815` | `#eef2ef` |
| `primary`               | `#006341` | `#12a06b` |
| `primary-foreground`    | `#f7fbf9` | `#06140e` |
| `secondary`             | `#efebe6` | `#26302a` |
| `secondary-foreground`  | `#36302a` | `#d6ddd8` |
| `muted`                 | `#efebe6` | `#1e2620` |
| `muted-foreground`      | `#6b6259` | `#9aa39d` |
| `accent`                | `#f1ebe4` | `#232c26` |
| `accent-foreground`     | `#36302a` | `#eef2ef` |
| `destructive`           | `#c23b2e` | `#e06a5e` |
| `destructive-foreground`| `#ffffff` | `#2a0e0b` |
| `border`                | `#e3ddd5` | `#26302a` |
| `input`                 | `#e3ddd5` | `#2a352e` |
| `ring`                  | `#2e9470` | `#12a06b` |

### Finance + chart semantics

| Token / utility | Light | Dark | Use |
|-----------------|-------|------|-----|
| `terracotta`  | `#db8f6b` | `#db8f6b` | Brand accent + highlights in UI chrome. **Not** a chart series (charts use `chart-2`), and **not** negative money. |
| `gain`        | `#0f7a54` | `#2fb37e` | Positive money — deposits, surplus, gains. |
| `loss`        | `#c23b2e` | `#e06a5e` | Negative money. Deliberately redder than terracotta. |
| `warning`     | `#c98a12` | `#e0a82e` | Caution / attention-needed. |
| `info`        | `#3a6fa5` | `#6ba0d8` | Informational, neutral emphasis. |
| `chart-1`     | `#006945` | `#11a06b` | Series 1 (emerald — always first) |
| `chart-2`     | `#d38058` | `#e46c29` | Series 2 (terracotta) |
| `chart-3`     | `#3a6fa5` | `#6499d2` | Series 3 (info blue) |
| `chart-4`     | `#8e492e` | `#c87450` | Series 4 (clay) |

**Chart slots are four, fixed-order, and never cycled — [ADR 0054](../adr/0054-categorical-chart-palette.md).**
These are not free choices: the four steps are picked so every *adjacent* pair stays
distinguishable under protanopia and deuteranopia in **both** modes, so reshuffling the
order or adding a fifth breaks the property. Past four series, fold the tail into an
explicit "Other", facet into small multiples, or cap the selection and say so — never
reuse a colour, and never ask for a `--chart-5`. `apps/desktop/src/styles/chart-palette.test.ts`
enforces this; it exists because the previous by-eye palette shipped with dark-mode
`chart-1` and `chart-2` collapsing to ΔE 1.6 under protanopia.

Note `chart-2` is *not* the same value as `--terracotta`. The brand accent stays
`#db8f6b` for non-chart UI; the chart slot is that hue re-stepped for legibility against
a chart surface.

**Money color rule:** positive → `text-gain`, negative → `text-loss`, zero/neutral →
`text-foreground` or `text-muted-foreground`. In the app this is centralized in
`signedAmountClass`. Never use plain green/red — use the `gain`/`loss` tokens.

---

## 4. Shape & spacing

- **Radius:** base `--radius: 0.5rem`. Derived: `sm` = base − 4px, `md` = base − 2px,
  `lg` = base. Cards use `rounded-lg`; buttons/inputs use `rounded-md`.
- **Borders:** 1px, `border` token. Default border color is applied globally.
- **Elevation:** subtle only — `shadow-sm` on cards. Avoid heavy drop shadows; depth
  comes from surface color (`card` vs `background`) and borders.
- **Spacing:** Tailwind 4px scale. Cards pad `p-6`; card headers stack with `gap-1.5`;
  form fields stack with `gap-2`; section gaps `gap-4`/`gap-6`.

---

## 5. Component conventions (shadcn)

Build mockups from these primitives. The four that exist in code today are
**Button, Card, Input, Label** (`apps/desktop/src/components/ui/`); the rest are
standard shadcn components we adopt as we need them — mock them in our tokens.

### Button (`variant` × `size`)
- Variants: `default` (bg-primary), `secondary`, `outline`, `ghost`,
  `destructive`, `link`.
- Sizes: `default` (h-10 px-4), `sm` (h-9 px-3), `lg` (h-11 px-6 text-base),
  `icon` (h-10 w-10).
- Base: `rounded-md text-sm font-medium`, focus ring = `ring-2 ring-ring ring-offset-2`,
  disabled = `opacity-50`. Icons are `size-4`.

### Card
- `rounded-lg border bg-card text-card-foreground shadow-sm`.
- Slots: `CardHeader` (`p-6 gap-1.5`) → `CardTitle` (`font-semibold tracking-tight`) +
  `CardDescription` (`text-sm text-muted-foreground`); `CardContent` (`p-6 pt-0`);
  `CardFooter` (`p-6 pt-0 flex items-center`).
- The primary container for everything: KPIs, lists, forms, charts.

### Input / Label
- Input: `h-10 rounded-md border border-input bg-background px-3 text-sm`, focus ring.
- Label: `text-sm font-medium`, paired above the control with `gap-2`.

### Other shadcn components (mock in our tokens as needed)
- **Dialog / Modal:** `popover` surface, `rounded-lg border shadow-lg`, dimmed
  overlay (`bg-foreground/50`), title `text-lg font-semibold`, description
  `text-sm text-muted-foreground`, footer actions right-aligned.
- **Badge:** `rounded-full px-2.5 py-0.5 text-xs font-medium`. Use semantic tints —
  e.g. gain badge = `bg-gain/15 text-gain`, warning = `bg-warning/15 text-warning`.
- **Table:** header row `text-xs text-muted-foreground` uppercase-ish, rows
  separated by `border`, money columns right-aligned with `tabular-nums`.
- **Tabs:** underline or pill; active = `text-foreground`, inactive =
  `text-muted-foreground`; the in-app section switcher is a tab in `UnlockedHome`.
- **Toast / inline alert:** semantic left-border or tinted background using
  `info` / `warning` / `gain` / `destructive`.
- **Select / Dropdown / Checkbox / Switch:** standard shadcn, `input`/`border`
  tokens, `primary` for the checked/active state, `ring` for focus.

### Golden recipes for token-sensitive components
For the components where the shadcn API + our tokens are easy to get wrong —
**interactive multi-line chart (shadcn + Recharts), data table, dialog/modal, and
RHF+Zod form** — use the copy-pasteable snippets in [`recipes.md`](./recipes.md)
rather than re-deriving. Key rule: chart colors are full hex tokens, so
`ChartConfig` uses `color: "var(--chart-1)"`, **not** `hsl(var(--chart-1))`.

### Every data surface = 4 explicit states
Loading, error, empty, success — **never** an ambiguous blank/null. Mock all four
when you design a list, table, or chart-backed view. (This is a hard app rule.)

### Accessibility
Label every control; meaningful `aria-label`s (esp. charts); visible focus ring on
everything keyboard-reachable; sufficient contrast in **both** modes.

---

## 6. App information architecture (what to mock)

The unlocked app is a single shell with a section switcher. Current views:

Grouped into **Overview / Planning / Review** in the sidebar, with Backup + Settings pinned
below (ADR 0049).

*Overview*
- **Dashboard** — KPI cards (net worth, cash, this-month flow), spending chart, recent activity.
- **Accounts** — account list with balances.
- **Transactions** — the Activity list; table, filter/search, signed amounts.

*Planning*
- **Cash Flow** — realized history + projected balance with its likely range.
- **Bills** — upcoming bills, add-bill form.
- **Recurring Transfers** — scheduled money movement.
- **Income** — income sources/schedule.

*Review*
- **Money Inbox** — items needing review (snooze/dismiss); badged with its pending count.
- **Categories** — category management.

*Pinned*
- **Backup** — encrypted backup/restore.
- **Settings** — incl. theme (light/dark).

There is also a **vault lock/unlock** lifecycle (lock screen, unlock with password,
locked-state empty surfaces) — the app is local-first and encrypted.

---

## 7. Visual reference: the component gallery

[`component-gallery.html`](./component-gallery.html) renders the components above in
**both light and dark** using the exact token values. Workflow:

1. Open it in a browser. Toggle light/dark with the button top-right.
2. Screenshot the relevant section.
3. Hand the screenshot to Claude Design alongside this doc so it has a concrete
   visual target, not just hex values.

Regenerate/extend the gallery as we add components so the reference stays current.

---

## 8. Keeping the design system alive (screenshot workflow)

This system is meant to **evolve as the app gets built**. As real screens land:

1. **Capture** a screenshot of the real view (light and/or dark).
2. **Save** it to [`screenshots/`](./screenshots/) with a descriptive name, e.g.
   `dashboard-light.png`, `transactions-empty-dark.png`,
   `add-bill-modal-light.png`.
3. **Index** it in the table below (one row each) with a one-line note on what it
   shows and anything notable about the layout.
4. **Feed** the screenshot + this doc to Claude Design when mocking adjacent
   screens, so new mockups match shipped reality.
5. When a pattern recurs across screenshots, **promote it** into §5/§6 here (and,
   if it's a real component, into `apps/desktop/src/components/ui/` + the gallery).

> Tip: name files `<view>-<state>-<mode>.png` (`state` = default/empty/error/loading/modal).
> Keep them reasonably sized; this folder is reference material, not app assets.

### Screenshot index

| File | View / state | Mode | Notes |
|------|--------------|------|-------|
| _(none yet)_ | — | — | Add rows as screens ship. |

---

## 9. Quick reference for prompting Claude Design

Paste something like:

> Use the DohFlow design system. Font: Inter, tabular-nums on all numbers.
> Primary emerald `#006341` (dark mode `#12a06b`), terracotta accent `#db8f6b`,
> warm off-white background `#faf8f6` / warm-gray neutrals. Components are
> shadcn/ui: cards `rounded-lg border bg-card shadow-sm`, buttons `rounded-md`
> primary/secondary/outline/ghost/destructive. Money: positive = gain green
> `#0f7a54`, negative = loss red `#c23b2e`. Calm, trustworthy, ledger-like — not
> neon fintech. Mock light and dark. Show loading/empty/error/success states.

Full machine-readable values: [`design-tokens.json`](./design-tokens.json).
