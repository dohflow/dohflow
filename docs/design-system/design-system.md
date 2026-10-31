# DohFlow — Design System

> **What this is.** A description of the design system **as shipped** in the desktop app: the
> tokens, the type, the components that exist in code, and the app's structure. It is the
> reference for engineering and for anyone comparing a design to what is built. It is **not**
> where design intent is decided: **Paper** (the Desktop Studio's design file) is the design
> source of truth for how a screen should look and behave, and this repository is the source of
> truth for what shipped ([ADR 0031, addendum 2026-10-27](../adr/0031-ui-quality-and-design-workflow.md)).
> Where Paper and the app differ, that is a gap recorded as a bead, not something either side
> edits away. Use [`design-tokens.json`](./design-tokens.json) for exact values and
> [`component-gallery.html`](./component-gallery.html) as a visual reference.
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

[ADR 0063](../adr/0063-typeface-pairing.md): **two families, split on one line: numbers stay sober.**

- **UI and data face: IBM Plex Sans** (variable build, self-hosted via
  `@fontsource-variable/ibm-plex-sans`). This is the **default** (`font-sans` and `body` resolve to
  it) and it holds **every numeral**: balances, tables, forecasts, forms.
  Stack (`--font-ui`): `"IBM Plex Sans Variable", "IBM Plex Sans", ui-sans-serif, system-ui,
  -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`. The family name **must** be
  `IBM Plex Sans Variable`, which is what the fontsource package declares; the plain name falls
  back to the system face **silently**.
- **Display face: Nunito** (variable build, self-hosted via `@fontsource-variable/nunito`),
  **opt-in** through the `font-display` utility (`--font-display`, falling back to the UI face).
  It is for the wordmark, headings and hero or marketing copy, never for digits, tables or
  forecasts. **As of this writing no shipped component uses `font-display`**: the stylesheet is
  loaded and the utility is declared, and the only Nunito in the product is the wordmark, which is
  converted to outlines (`apps/desktop/src/brand/`), not set as live text. Variable Nunito's
  default weight is 200, so always set a weight explicitly when opting in.
- **No separate mono font.**
- **Numerals:** `font-variant-numeric: tabular-nums` is set on `body`, but alignment does not come
  from the `tnum` feature (the shipped woff2 has none). It comes from IBM Plex Sans's digits being
  uniform-width by default. `slashed-zero` was tried and removed: the shipped woff2 has no `zero`
  feature either. Keep `tabular-nums` on money anyway; it costs nothing and survives a font swap.
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
| `warning`     | `#b3780a` | `#e0a82e` | Caution / attention-needed. Light value was `#c98a12` (2.95:1 on `card`); `#b3780a` keeps the hue and clears 3.5:1 on both `card` and `background` (`personal-cfo-2qonl`). |
| `info`        | `#3a6fa5` | `#6ba0d8` | Informational, neutral emphasis. |
| `chart-1`     | `#006945` | `#11a06b` | Series 1 (emerald — always first) |
| `chart-2`     | `#d38058` | `#e46c29` | Series 2 (terracotta) |
| `chart-3`     | `#3a6fa5` | `#6499d2` | Series 3 (info blue) |
| `chart-4`     | `#8e492e` | `#c87450` | Series 4 (clay) |

**Theme-invariant ink.** Two tokens deliberately do **not** flip with the theme, because the thing
they sit on does not: `--on-brand-fill` (`#1c1815`) is the text color on a brand fill such as the
terracotta build badge, and `--brand-white` (`#ffffff`) is the white band of the brand mark. They
exist so that "no raw hex in components" stays true: the exceptions are named.

**Known contrast gaps (open, not waived; `personal-cfo-4d8.35`).** Text in a semantic color on a 10%
tint of that color over the Light background (the `Badge` treatment) is below 4.5:1 for 12px text
(measured: gain 4.40, warning 3.17, info 4.36, loss 4.34), and the `Input` boundary against its background is 1.27:1 Light and 1.46:1 Dark, below the
3:1 expected of a boundary that identifies a control. Neither is remediated yet, and no document
here treats them as accepted. Do not copy these treatments into new work as if they were settled.

**Chart slots are four, fixed-order, and never cycled — [ADR 0054](../adr/0054-categorical-chart-palette.md).**
These are not free choices: the four steps are picked so every *adjacent* pair stays
distinguishable under protanopia and deuteranopia in **both** modes, so reshuffling the
order or adding a fifth breaks the property. Past four series, fold the tail into an
explicit "Other", facet into small multiples, or cap the selection and say so — never
reuse a color, and never ask for a `--chart-5`. `apps/desktop/src/styles/chart-palette.test.ts`
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

## 5. Components: what is built, and the conventions

Components are shadcn-style: **copy-in source we own** (Radix + Tailwind +
`class-variance-authority`), themed only through the tokens above. Once a component lives in
`apps/desktop/src/components/ui/`, **our local copy is the source of truth**
([`FRONTEND.md`](../agent/FRONTEND.md) section 7.1). Compose these; do not hand-roll a new table,
chart or confirm per view.

### Inventory (current `main`)

Paths are under `apps/desktop/src/`. "Shared" means it lives in `components/ui/` or
`components/` and is imported by more than one screen.

| Component | Path | What it is |
|-----------|------|-----------|
| `Button` | `components/ui/button.tsx` | Variants `default` · `secondary` · `outline` · `ghost` · `destructive` · `link`; sizes `default` h-10, `sm` h-9, `lg` h-11, `icon` h-10 w-10. `buttonVariants` is exported. |
| `Card` family | `components/ui/card.tsx` | `Card`, `CardHeader` (`p-6`, `gap-1.5`), `CardTitle`, `CardDescription`, `CardContent` (`p-6 pt-0`), `CardFooter` (`flex items-center p-6 pt-0`). |
| `Input` | `components/ui/input.tsx` | h-10, `rounded-md border border-input bg-background px-3 text-sm tabular-nums shadow-sm`. |
| `Label` | `components/ui/label.tsx` | `text-sm font-medium`, above its control. |
| `NativeSelect` | `components/ui/native-select.tsx` | The platform `<select>` with one shared skin (h-10; `size="sm"` is h-8 `text-xs`). Kept native on purpose: reliable keyboard and assistive-technology behavior. |
| `Combobox` | `components/ui/combobox.tsx` | Searchable single select (`items` with `label`, optional `hint` and `searchText`; a `clearLabel` and a persistent `footerAction`). Portal-rendered. |
| `Badge` | `components/ui/badge.tsx` | Variants `default` · `secondary` · `outline` · `gain` · `warning` · `info` · `loss` (the four semantic ones are a 10% tint with a 25% border, `warning` 30%). |
| `Skeleton` | `components/ui/skeleton.tsx` | Pulsing placeholder block for loading states. |
| `EmptyState` | `components/ui/empty-state.tsx` | Icon medallion, one-line title, "what to do next" description. |
| `Table` family | `components/ui/table.tsx` | `Table` (wraps an `overflow-x-auto` container), `TableHeader`, `TableBody`, `TableFooter`, `TableHead`, `TableRow`, `TableCell`, `TableCaption`. |
| `DataTable` | `components/ui/data-table.tsx` | The collection surface (ADR 0053): columns, sort, an expander row, and the **loading / empty / error** treatments, so a screen cannot invent a fourth. |
| `PaginationControls` | `components/ui/pagination.tsx` | Display range, sticky page-size selector (10 / 25 / 50 / 100), previous and next. Pairs with `usePagination`. |
| `ChartContainer`, `ChartTooltipContent` | `components/ui/chart.tsx` | The themed chart wrapper (Recharts): exposes each series color as `--color-<key>`. See [`dataviz.md`](./dataviz.md). |
| `RankedBars` | `components/ui/ranked-bars.tsx` | Ranked horizontal bars for a nominal category and the money against it. |
| `EmojiPicker` | `components/ui/emoji-picker.tsx` | Curated, dependency-free emoji grid with filtering. |
| Notices | `components/ui/notice.tsx`, `notice-context.ts`, `notice-stack.ts` | `NoticeProvider`, `NotificationsRegion`, `useNotices()`; the Undo notice and the stack rule ([`FRONTEND.md`](../agent/FRONTEND.md) section 6.2). |
| `RemoveConfirm` | `components/ui/remove-confirm.tsx` | The inline confirm for a reversible remove (section 6.2). |
| `Disclosure`, `usePinDisclosures` | `components/ui/disclosure.tsx` | The shared independent disclosure (Paper A2): a card whose heading holds one trigger button; a hidden, inert body that keeps drafts; a `persistent` slot for the error, remedy and action; ancestor pinning for destructive confirmations. No screen uses it yet (`FRONTEND.md` section 7.5). |
| `PageHeader` | `components/PageHeader.tsx` | One title scale for a page: title, optional muted subtitle, a right-aligned actions slot. Used by Dashboard, Accounts and Settings. |
| Brand mark and wordmark | `brand/Brand.tsx` | The mark and the wordmark (outlines, not live text). |

### Not shared yet (each screen builds its own)

Naming these so a mock is not drawn from a component that does not exist:

- **Dialog / modal / drawer.** There is **no shared Dialog**. Modals and drawers are hand-rolled
  per view (for example `settings/RotateKeyCard.tsx`, `settings/NewMappedAccountDialog.tsx`,
  `accounts/ReconcileBalanceModal.tsx`, `search/GlobalSearch.tsx`,
  `transactions/TransactionDetailDrawer.tsx`, `bills/BillDetailDrawer.tsx`). The dialog snippet
  in [`recipes.md`](./recipes.md) is a recipe, not a component.
- **Tabs.** There is no tabs component. The section switcher is the sidebar
  (`vault/screens/Sidebar.tsx`).
- **Checkbox and Switch.** Native inputs, styled per use; there is no shared component.
- **Disclosure / accordion.** `Disclosure` (above) is the shared one, built to the A2 contract
  (`personal-cfo-4d8.38`). **No screen has adopted it yet.** Several screens still implement their
  own (`accounts/AccountsView.tsx`, `accounts/CreditCardsView.tsx`,
  `future-cash/NeedsConfirmation.tsx`, `future-cash/ProjectedActivityTable.tsx`,
  `transactions/TransactionFilterBar.tsx`, among others); `FRONTEND.md` section 7.5 says which
  differences a migration must reconcile.
- **Toast.** Use a notice (above); there is no separate toast.

### Conventions (shadcn)

**Button, Card, Input, Label.** As in the table. Base button: `rounded-md text-sm font-medium`,
focus ring `ring-2 ring-ring ring-offset-2`, disabled `opacity-50`, icons `size-4`. Card:
`rounded-lg border bg-card text-card-foreground shadow-sm`.

**Badge.** `inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium`. The semantic variants are
`border-<token>/25 bg-<token>/10 text-<token>` (warning uses `/30` on the border); see the known
contrast gap in section 3.

**Table.** Header row `text-xs text-muted-foreground`, rows separated by `border`, money columns
right-aligned with `tabular-nums`.

**Dialog (the pattern the hand-rolled ones follow).** `popover` surface,
`rounded-lg border shadow-lg`, dimmed overlay (`bg-foreground/50`), title `text-lg font-semibold`,
description `text-sm text-muted-foreground`, footer actions right-aligned.

**Alerts and notices.** Semantic left-border or tinted background using
`info` / `warning` / `gain` / `destructive`; transient messages with an Undo use the notice
primitive.

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

## 6. App information architecture (what is built)

The unlocked app is one shell: a sidebar and a main pane (`vault/screens/UnlockedHome.tsx`,
`vault/screens/Sidebar.tsx`, ADR 0049). The sidebar can collapse to icons; every item keeps its
accessible name. The groups, in order:

*Overview*
- **Dashboard** — KPI cards (net worth, cash, this-month flow), spending chart, recent activity.
- **Accounts** — account list with balances.
- **Debt** — loans and cards, payoff views.
- **Transactions** — the Activity list; table, filter/search, signed amounts.

*Planning*
- **Cash Flow** — realized history plus the projected balance with its likely range.
- **Scenarios** — what-if layers over the forecast.
- **Bills** — recurring bills, suggestions, add-bill form.
- **Recurring Transfers** — scheduled money movement.
- **Income** — income sources and their schedule.

*Review*
- **Money Inbox** — items needing review (snooze/dismiss), badged with the pending count.
- **Categories** — category management.

*Pinned*
- **Settings** — appearance (light/dark), household, backups, connections, vaults, security. There
  is **no separate Backup item**; backups live in Settings.

There is also a **vault lock/unlock** lifecycle (vault picker, create, lock and unlock screens,
recovery) before the shell appears: the app is local-first and encrypted.

---

## 7. Visual reference: the component gallery

[`component-gallery.html`](./component-gallery.html) renders shadcn-style components in **both light
and dark** using the token values in section 3. It is a **static reference page**, not the
running app: it does not import the app's code, so it can drift. When it and the app disagree, the
app (and `globals.css`) win. It makes no network requests; the typefaces render only if IBM Plex
Sans is installed locally, and otherwise fall back to the system sans.

1. Open it in a browser. Toggle light/dark with the button top-right.
2. Screenshot the relevant section for comparison with Paper or with the running app.

Extend it when a shared component lands so the reference stays current.

---

## 8. Keeping the design system alive (screenshot workflow)

This system is meant to **evolve as the app gets built**. As real screens land:

1. **Capture** a screenshot of the real view (light and/or dark).
2. **Save** it to [`screenshots/`](./screenshots/) with a descriptive name, e.g.
   `dashboard-light.png`, `transactions-empty-dark.png`,
   `add-bill-modal-light.png`.
3. **Index** it in the table below (one row each) with a one-line note on what it
   shows and anything notable about the layout.
4. **Hand** the screenshot to the Desktop Studio (on the bead) so Paper can be compared with
   shipped reality. Screenshots use **fake data only**.
5. When a pattern recurs across screenshots, **promote it** into §5/§6 here (and,
   if it's a real component, into `apps/desktop/src/components/ui/` + the gallery).

> Tip: name files `<view>-<state>-<mode>.png` (`state` = default/empty/error/loading/modal).
> Keep them reasonably sized; this folder is reference material, not app assets. It is empty
> today; the index below lists what exists.

### Screenshot index

| File | View / state | Mode | Notes |
|------|--------------|------|-------|
| _(none yet)_ | — | — | Add rows as screens ship. |

---

## 9. Quick reference (for a simple mockup or a brief)

Paper is where a screen is designed. For a **simple mockup or a small in-app tweak** in another
tool (not authoritative; see the ADR 0031 addendum), a brief that matches the shipped system:

> DohFlow design system. Fonts: IBM Plex Sans (numbers always, tabular figures); Nunito only for
> the wordmark and display headings. Primary emerald `#006341` (dark `#12a06b`), terracotta accent
> `#db8f6b`, warm off-white background `#faf8f6`, warm-gray neutrals. Components are shadcn/ui:
> cards `rounded-lg border bg-card shadow-sm`, buttons `rounded-md`
> primary/secondary/outline/ghost/destructive. Money: positive = gain `#0f7a54`, negative = loss
> `#c23b2e`; warning `#b3780a`. Calm, trustworthy, ledger-like. Light and dark. Show loading,
> empty, error and success states.

Full machine-readable values: [`design-tokens.json`](./design-tokens.json).

---

## 10. Paper to code map

Paper is the design source of truth; the app is what shipped
([ADR 0031, addendum 2026-10-27](../adr/0031-ui-quality-and-design-workflow.md)). This section is
the join between them, so a reader can tell, for any Paper pattern, **what implements it, or that
nothing does yet**.

**How to cite Paper.** A build or a row below names the Paper **page** and the **freeze** (UTC
time, and the hash or revision the Studio recorded). The Paper *file* is recorded on the bead, not
here: a page id alone is only meaningful with it, so a row is a pointer for the Studio and for
engineering, not a link for a casual reader. **Status** is one of: **Shipped** (code exists and
matches), **Convention** (a written rule in `FRONTEND.md`, no component), **Proposed** (a bead
exists, nothing is built), **Not shipped** (drawn in Paper, and no code or bead claims it; none yet).

| Paper pattern | Paper page and freeze | Code | Status |
|---------------|-----------------------|------|--------|
| Shared independent disclosure ("A2"): a heading button that opens and closes a section, with the error, remedy and action outside the hidden body and nested destructive guards | page `p-7-0`, frozen 2026-10-26 05:24:16 UTC | `components/ui/disclosure.tsx` (`personal-cfo-4d8.38`), built to the specification [`settings-layout-review.md`](./settings-layout-review.md). Unadopted: the existing screens keep their own until each is migrated | **Shipped** (component), adoption pending |
| Layout contract ("LCR-1 v1"): priority-ordered, container-aware 3, 2 or 1 spans; collapsed siblings at the tallest natural height; parallel control tops aligned | page `p-8-0`, frozen 2026-10-27 15:21:48 UTC, hash `0afec932` | the written convention in [`FRONTEND.md`](../agent/FRONTEND.md) section 7.4. No layout component: a page composes it with Tailwind grid and subgrid | **Convention** |
| Settings page, selected composition "A" | the Settings design page, v3, frozen 2026-10-27 15:36:39 UTC | `apps/desktop/src/settings/SettingsView.tsx` composes the existing cards today as **one column of always-open cards** (no disclosure, no grid). The target composition is `personal-cfo-4d8.39` | **Proposed** |
| Collapsed-section summaries (frequency and verified backup status, the formatted Comfort range, the active vault name, the connection count, the application version) | within the Settings design page above | none; they depend on the shared disclosure (`personal-cfo-4d8.38`) and belong to the Settings composition (`personal-cfo-4d8.39`) | **Proposed** |
| Neutral-foreground status text with a semantic mark | owner decision recorded on the Studio's bead | none. It depends on the contrast remediation `personal-cfo-4d8.35`, and **no palette change is made or implied here** | **Proposed** |
| Every component in the section 5 inventory | **no Paper comparison has been made yet** | the section 5 table | **Shipped**, Paper counterpart to be recorded by the Studio on its first comparison |

**Adding a row.** When a build implements a Paper design, add or update its row in the same change:
the Paper page and freeze it was built from, the real code path, and the status. Never mark a row
**Shipped** from a design file alone.
