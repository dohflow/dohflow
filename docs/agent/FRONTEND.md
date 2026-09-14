# Frontend conventions (apps/desktop)

Operating rules for the React + TypeScript desktop UI. Derived from **ADR 0003**
(trust boundary), **ADR 0010** (window/capability isolation), and **ADR 0020**
(frontend state/data/forms). When this doc and an ADR disagree, the ADR wins —
fix this doc.

> **Transition note.** ADR 0020 adopts **TanStack Query** and **React Hook Form +
> Zod**, but the migration is tracked by `personal-cfo-o9cz` and has **not landed
> yet**. Sections marked **(target — o9cz)** describe where new and migrated code
> goes; until the QueryClient/RHF wiring lands, match the **existing** hand-rolled
> pattern for consistency and migrate as part of `o9cz`, not piecemeal.

## 1. The trust boundary is absolute (ADR 0003)

- The WebView is **presentation only**. It never holds the vault password, any
  key, DB handles, connector tokens, or filesystem paths.
- **All** backend interaction goes through the generated typed bindings:
  `import { commands } from "@/bindings"` — **never** a raw `invoke`.
- Validation in the frontend is for UX only; **Rust re-validates and is
  authoritative**. Never trust a client-side check as a security control.
- No remote origins, no CDNs, no `eval`. Assets are self-hosted (fonts via
  `@fontsource-variable`). This honors the no-network rule and ADR 0010's CSP.

## 2. Data fetching — TanStack Query (target — o9cz)

- One query hook per read, calling a typed `commands.*`; a stable typed query key
  per `(command, args)`.
- Mutations (create/update commands) **invalidate** the affected query keys —
  never a bespoke `refresh()` or a tab remount to force a refetch.
- The `QueryClient` cache is **cleared on vault lock** (ADR 0003: "the cache is
  cleared on lock").
- **Until o9cz lands**, the existing pattern is a `useX` hook holding
  `data | null`, `error`, and a `refresh()` (see `useBills.ts`, `useDashboard.ts`).
  Match it for new screens, then migrate the whole set together.

Every IPC result is a `Result<T, IpcError>` (`{ status: "ok"|"error" }`). Map
errors for display with `describeIpcError` (`@/vault/useVault`).

## 3. Forms — React Hook Form + Zod (target — o9cz)

- One **Zod schema per form** (required fields, amount > 0, date format, currency
  match). RHF owns field state and submission.
- Rust still re-validates — Zod is fast, friendly client feedback, not authority.
- **Until o9cz lands**, forms are `useState` fields with explicit validity flags
  (see `BillsView.tsx` `AddBillForm`). Match it for consistency.

## 4. State + routing (ADR 0020)

- **Global state:** a small typed React **Context** (e.g. `useVault`) per concern.
  Do **not** add Zustand/Jotai yet — deferred with rationale in ADR 0020.
- **Routing:** vault lifecycle routes in `App.tsx`; in-shell sections are a
  `useState` tab in `UnlockedHome`. Do **not** add TanStack Router yet — deferred.
  Revisit both deferrals per ADR 0020's "revisit if".

## 5. Money + dates (ADR 0021)

- Money crosses the wire as `MoneyDto` (**integer minor units** + ISO currency) —
  never a float. Render with `lib/format`: `formatMoney`, `formatSignedMoney`,
  `signedAmountClass` (gain/loss colour).
- Parse user input with `dollarsToMinorUnits`.
- Render a bare `YYYY-MM-DD` calendar date with **`formatIsoDate`** (constructs a
  local `Date` to avoid the UTC-midnight off-by-one). Use `formatDate` only for
  RFC 3339 instants. (ADR 0021 §6.)

## 6. UI states — explicit, never ambiguous

- Every data surface handles **loading**, **error**, **empty**, and **success**
  explicitly. No widget shows an ambiguous null. (1vd7 AC; the gate cares.)
- Consolidate these into shared primitives rather than re-hand-rolling per screen:
  error (`personal-cfo-027s`), loading/skeleton (`-mm7a`), empty (`-tzds`),
  success (`-x3o0`).
- **For a collection surface, use `components/ui/data-table`** (ADR 0053): it owns the
  loading / empty / error treatments and the expander row's span, so a screen cannot
  invent a fourth. Do NOT hand-roll a spinner beside a table. For the list surfaces that
  stay lists (Categories, Accounts — see the surface audit), use `ui/empty-state` and
  `ui/skeleton` directly rather than a bespoke treatment.
- **A chart is not exempt** — it handles the same four states.

### 6.1 Charts

Read [`docs/design-system/dataviz.md`](../design-system/dataviz.md) **before** writing
chart code or picking a chart colour. The rules that bite most often:

- **Four categorical slots (`--chart-1..4`), assigned in order, NEVER cycled** (ADR 0054).
  `i % palette.length` paints two different series the same colour; past four, fold into
  "Other" / facet / cap-and-say-so. `styles/chart-palette.test.ts` enforces the palette
  itself and fails the suite if a token stops clearing its colour-vision gates.
- **One series → one colour.** Never shade nominal bars darker-where-bigger.
- **Never a dual axis.** Two scales on one plot invent a correlation.
- Text wears text tokens; only marks wear the series colour.
- Every chart is a `<figure>` with a real `aria-label`, and a tooltip never gates a value.

## 7. Components + styling

- Use the shadcn-style primitives in `@/components/ui` (`Button`, `Input`,
  `Label`, `Card`); compose classes with `cn()`; import via the `@/*` alias.
- Use the Tailwind theme tokens (`primary`, `secondary`, `loss`, `gain`,
  `warning`, `muted-foreground`, …) — never hard-coded hex. Money uses
  `tabular-nums`. Light + dark must both work.
- Accessibility: label every control, use semantic roles, keep sections
  keyboard-reachable, give meaningful `aria-label`s (e.g. the dashboard chart).

### 7.1 shadcn is copy-in, not a dependency

shadcn/ui is **not** an installed library — there is no `shadcn` package and no
`import ... from "shadcn"`. Components are **source you copy into this repo** (built
on Radix + Tailwind + `class-variance-authority`) and then **own and re-theme**.
Our `button.tsx`/`card.tsx`/`input.tsx`/`label.tsx` were authored this way; they
already carry our tokens. There is no `components.json` yet — it's created the first
time the CLI runs.

**Adding a new primitive** (e.g. `dialog`, `table`, `chart`, `form`):

- Preferred: `pnpm dlx shadcn@latest add <name>` from `apps/desktop` (creates
  `components.json` on first run), **then re-theme to our tokens** — replace any
  default colors with our token utilities (`bg-primary`, `text-muted-foreground`,
  `var(--chart-1)`, …) and verify light + dark.
- Or hand-author in the shadcn style (as the existing four were) when the CLI adds
  unwanted deps/config — keep the upstream structure, swap in our tokens.

**When to consult the official shadcn references** — for *discovering* what
components exist and confirming the canonical structure/API of one you're adding
(via WebFetch):

- Component catalog: `https://ui.shadcn.com/docs/components`
- Charts (Recharts wrapper): `https://ui.shadcn.com/docs/components/chart`
- Source registry: `https://github.com/shadcn-ui/ui`

**Authority rule:** upstream is the *seed*. Once a component lives in
`@/components/ui`, **our local copy is the source of truth** — don't re-derive its
behavior from the docs. Our token mapping and design conventions live in
[`docs/design-system/`](../design-system/) (`design-system.md`, `recipes.md`);
follow those over upstream defaults.

### 7.2 UI-quality discipline + the Claude Design loop (ADR 0031)

How we decide *when to just build vs. mock first* is [ADR 0031](../adr/0031-ui-quality-and-design-workflow.md).
The short version:

- **Shared primitives are the quality lever.** Promote the `recipes.md` components
  (chart, table, dialog, form) into `@/components/ui` and **compose** them; don't
  hand-roll a new table/chart per view. That inconsistency is what made the app stop
  "feeling like shadcn." Tracked by `personal-cfo-4d8.6`.
- **Three routing lanes:** *mechanical/correctness* (layout bug, wrong value) → just
  fix; *compose-from-primitives* (cards + shared table/chart/dialog/form) → build
  directly against `design-system.md` + the gallery; *net-new/complex screen* → mock
  in Claude Design first.
- **The agent can't drive Claude Design** (`/design-sync` needs interactive auth). For
  a net-new screen, the agent writes a ready-to-paste Claude Design prompt into the
  bead's `design` field (purpose, data, 4 states, primitives, tokens, light+dark);
  the maintainer mocks; the agent implements to match and drops a screenshot into
  [`docs/design-system/screenshots/`](../design-system/screenshots/).
- **`/design-sync` stays deferred** until the four primitives are real code
  (≈8–12 components); see the `claude-design-design-sync-is-deferred-not-abandoned`
  memory.
- **Track UI-quality gaps as their own beads** under the UX epic (`personal-cfo-4d8`),
  same as functional gaps.

### 7.3 Theme (dark mode) — personal-cfo-17u1

The app follows the macOS appearance by default, with a Settings → Appearance
override (System / Light / Dark). The **entire** mechanism is a token swap: toggling
the `.dark` class on `<html>` (via Tailwind's `@custom-variant dark (&:is(.dark *))`
in `globals.css`) re-resolves every `var(--token)` reference at paint, with zero
per-component dark-mode code. This is why §7's "Light + dark must both work" rule
above is enforceable at all — write through the tokens and both modes are already
handled; hard-code a hex and you've silently opted out of dark mode for that one spot.

- **`apps/desktop/src/theme/themePreference.ts`** is the single source of truth for
  resolving a preference against the live system query — shared by
  `theme-boot.ts` (a separate `<script type="module">` in `index.html`, loaded
  *before* `main.tsx` so the class is set before first paint — the CSP is
  `script-src 'self'`, so this can't be an inline script) and
  `theme/ThemeProvider.tsx` (the runtime state). Never resolve a theme any other
  way — a second resolution path is how boot-time and runtime disagree.
- **`ThemeProvider` is mounted exactly once, at the true app root (`App.tsx`), outside
  `VaultProvider`** — not inside Settings, not inside the unlocked shell. A review
  finding (`personal-cfo-17u1`) caught the first version of this getting that wrong:
  the theme state lived inside `AppearanceCard`, which `SettingsView` unmounts on
  every other tab and which never mounts at all on the lock screen — so "follows the
  macOS appearance while running" only held while Settings happened to be open, and a
  second independent instance is exactly how an explicit override could be silently
  reverted by a stale `matchMedia` listener from an earlier mount. `useTheme()` reads
  the ONE provider via context and throws if called outside it — there is deliberately
  no fallback path that would let a second instance exist by accident.
- **The appearance control itself must be reachable from every screen, locked
  included** — `theme/ThemeToggle.tsx` is rendered unconditionally in `App.tsx`'s
  `VaultRouter` (not gated on vault state, unlike `BuildBadge`), so switching
  System/Light/Dark never requires being inside the vault. `AppearanceCard` in
  Settings is the detailed, labeled control for when you're already there; the
  floating toggle is the quick, always-present one.
- **Every `:root` custom property needs a `.dark` counterpart**, with a short,
  named allowlist for the handful that deliberately don't flip (currently
  `--radius`, `--on-brand-fill`, `--brand-white` — each has its own doc comment in
  `globals.css` explaining why). `styles/theme-contrast.test.ts` enforces this and
  also holds the WCAG contrast floors (4.5:1 for text pairs, 3:1 for the
  chart/semantic tokens against `--card`) in both modes — extend its allowlists
  there, not by skipping the test.
- **Charts need no dark-mode code at all**: every chart color is a live
  `var(--chart-N)` (or `var(--gain)`/`var(--loss)`/etc.) string, never a value read
  once via `getComputedStyle` and cached — confirmed across every chart component as
  of 17u1. Keep it that way: caching a resolved color at render time would silently
  break on a live theme change.
- **A raw hex is not automatically a bug** — `design-tokens.test.ts`'s
  `DATA_DEFAULT` allowlist exists for exactly one legitimate case so far: an
  `<input type="color">`'s seed value (`categories/CategoriesView.tsx`,
  `AddCategoryForm.tsx`), which is user *data*, not theme styling, and can't take a
  CSS variable anyway. Any *other* raw hex is a styling decision that belongs in a
  token.
- **`color-scheme` follows the applied theme, not the system preference**:
  `:root` declares `color-scheme: light` and `.dark` overrides it to `dark`, so
  native controls/scrollbars match an explicit override even when it disagrees with
  macOS — declaring `light dark` on `:root` instead would let the browser pick by
  system preference alone and ignore the override.

## 8. Testing

- Each view gets a **Vitest + Testing Library** behavioral test that mocks
  `@/bindings` `commands.*` (see `BillsView.test.tsx`, `DashboardView.test.tsx`).
- **Async-rendered content uses `findBy*`, not `getBy*`** — query-backed UI only
  appears after the promise resolves (this caused a CI-only flake in `App.test`).
- Cover loading / populated / empty / error paths.
- Visual-regression testing is deferred to `personal-cfo-nbqd` (no infra yet).

## 9. IPC bindings are generated — keep them in sync

- `apps/desktop/src/bindings.ts` is **generated**, never hand-edited.
- When Rust IPC changes: `pnpm -C apps/desktop build` (needs `dist`), then from
  `apps/desktop/src-tauri` run `cargo run --bin export_bindings`, and update the
  command list in `bindings.test.ts`. CI's `ipc-codegen` job fails if it drifts.
- A **pure-frontend** change leaves `bindings.ts` untouched.

## 10. Gates before a frontend PR

`pnpm typecheck`, `pnpm lint`, `pnpm test`, `pnpm -r build` — all green. The
standalone desktop crate (`apps/desktop/src-tauri`) is tested separately only when
Rust IPC changed.
