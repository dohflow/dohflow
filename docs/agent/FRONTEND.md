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

### 6.2 Remove with undo (owner decision `personal-cfo-915.14`, option A)

Income sources, recurring bills and recurring transfers are **tombstoned** when removed
(`deleted_at`, ADR 0073 Addendum A), and each has a `reinstate_*` command that clears the
tombstone. A removal of such a thing is therefore **reversible**, and the interface says so and
offers the reversal. (A "Recently removed" list was considered and is **not** chosen.)

**Applies to** a remove that is a tombstone with a matching reinstate command. **Does not apply
to** archive, which has its own Archive and Restore affordances and an Archived section, and is not
a delete.

1. **The inline confirm.** Asking to remove shows **"Remove `<name>`?"** with the secondary line
   **"You can bring it back."** The confirm button repeats the verb (**Remove**); the other is
   **Cancel**. The prompt names the thing being removed, so a screen reader hears what it is
   confirming. Do not use "Delete" here: it promises something the app does not do.
2. **The notice.** After the removal succeeds, a transient notice reads **"Removed `<name>`. Undo"**,
   where **Undo** is a button. It is announced to screen readers (`role="status"`, a polite live
   region, never `alert`). **Where it lives:** in ONE **notifications region owned by the shell**
   (`role="region"`, `aria-label="Notifications"`), not in the screen, so a pending Undo survives
   leaving the screen (item 3). The region is rendered **first inside `main`**, so the tab order
   reaches its Undo button before the page content, and landmark navigation finds it by name.
   **A notice appearing never moves focus** (that would take the user out of what they were
   doing). Focus is never lost to the page body either: when the removed row disappears, focus
   moves to the nearest surviving row, or to the section heading if there is none.
3. **Duration and dismissal.** The notice stays for **10 seconds**, long enough to reach with the
   keyboard or a screen reader. The timer **pauses while the pointer is over the notice or focus is
   inside it**. It closes on expiry, on a close button, when Undo succeeds, and on **Escape while
   focus is inside that notice** (Escape elsewhere is never taken). Notices **stack** (newest
   last, at most three at once), so removing a second thing does not take away the chance to undo
   the first; **a fourth notice evicts the oldest Undo notice early**, ahead of its timer (see "When the stack is full"). Leaving the
   screen does not cancel a pending Undo: the notifications region lives with the shell.
4. **What Undo does.** It calls the matching reinstate command
   (`reinstate_income_source`, `reinstate_recurring_bill`, `reinstate_recurring_transfer`) with a
   fresh idempotency key, then refreshes **the same queries the removal refreshed** (and the
   forecast ones), so the item reappears where it was. Reinstating something that is not removed is
   an accepted no-op, so a double click is harmless.
5. **When Undo fails** it is an error state (section 6): the notice does not vanish. It keeps its
   **Undo** button (the removal still stands, so the user can retry), stops its timer until the user
   retries or dismisses it, and shows **"Couldn't undo — try again."** as its **own `role="alert"`
   element** inside the notice. The notice's own `role="status"` is not changed, so the live regions
   announce the failure once rather than re-announcing the whole notice.
6. **Irreversible deletes keep explicit destructive wording.** A remove with **no** reinstate
   command (a real, permanent deletion) never uses the wording above. Its confirm says that it
   **cannot be undone**, names what is lost, and has no Undo notice. Do not borrow the reversible
   pattern for it.

**An error notice (no Undo).** The same region and primitive also carry the news that an action
**partly worked**: the thing the user asked for was done, but a step that follows it failed (for
example a bill that was added but could not be linked to its loan, `personal-cfo-6wk.22`). Such a
failure is never silent (section 6). It is a notice with **no Undo**, in the same Notifications
region and stack (it is never evicted; see "When the stack is full"), whose message is its own **`role="alert"`**
element, mounted empty and filled on the next frame like the status line. Unlike an Undo notice it
**does not close on a timer**: it stays until the user closes it with its close button or with
**Escape while focus is inside it**, so it cannot vanish before it is read. Hover and focus rules
are moot (nothing is timed); a notice appearing still never moves focus. The copy says what was
done, what was not, and what to do next.

**When the stack is full.** At most three notices show at once. A further notice evicts the
**oldest Undo notice** (its timer was running anyway). It **never evicts an error notice**: the user
has not closed it, and it must not vanish before it is read. If all three are errors, the new notice
**waits** and is shown as soon as one is closed, so nothing is lost; waiting notices keep their order.

**The shared primitive.** The notice is one component in `components/ui` (a `Notice` and a small
provider/hook that a screen calls with the message, the Undo handler and the name), added by the
follow-up that applies this pattern (`personal-cfo-egn67`, AC5). New removals do not hand-roll
toasts, timers or live regions. The same primitive carries the Undo of **marking an occurrence paid**
in Cash Flow (`personal-cfo-4d8.32`; a confirmation rather than a remove, but the same notice rules:
one stack, one Escape, one focus rule, and a failure as its own alert). Its tests assert the
live-region text and role, that Undo calls the reinstate command, the failure state, the paused
timer under fake timers, stacking, eviction and the three dismissals.

## 7. Components + styling

- Use the shadcn-style primitives in `@/components/ui`; compose classes with `cn()`; import via
  the `@/*` alias. The current set (and what is **not** shared yet, such as a Dialog or a
  disclosure) is the inventory in
  [`docs/design-system/design-system.md`](../design-system/design-system.md) section 5.
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

### 7.2 UI-quality discipline and the design workflow (ADR 0031 and its 2026-10-27 addendum)

How we decide *when to just build vs. design first* is [ADR 0031](../adr/0031-ui-quality-and-design-workflow.md)
and its addendum. The short version:

- **Two sources of truth.** **Paper** (the Desktop Studio's design file) is the source of truth
  for design **intent**; the repository is the source of truth for what **shipped**. Tokens still
  follow `globals.css`. [`docs/design-system/`](../design-system/) and this file describe the
  shipped implementation and map Paper to code (`design-system.md` section 10); they never claim
  something shipped because Paper draws it. A difference between the two is a **bead**, not a
  silent edit on either side.
- **Shared primitives are the quality lever.** Compose what is in `@/components/ui` (inventory in
  `design-system.md` section 5); don't hand-roll a new table, chart or confirm per view. That
  inconsistency is what made the app stop "feeling like shadcn." Tracked by `personal-cfo-4d8.6`.
- **Three routing lanes:** *mechanical/correctness* (layout bug, wrong value) → just fix;
  *compose-from-primitives* (cards + the shared table/chart/form) → build directly against
  `design-system.md`; *net-new/complex screen* → **designed in Paper by the Desktop Studio**.
- **A build that implements a Paper design cites the Paper version** (page and freeze) in the bead
  and in the `design-system.md` section 10 row. **Source drift that affects approved behavior,
  wording or tokens goes back to the Studio before coding.**
- **Claude is still fine for simple mockups and small in-app tweaks.** Those are not
  authoritative and do not replace the Paper design for a lane-3 screen. The agent does not drive
  a design tool itself. `/design-sync` is unchanged and not re-enabled by the addendum.
- **Hand back what you ship.** When a merged change is something the Studio compares against, the
  bead records the merged commit and the evidence (screenshots of **fake data only**) and carries
  the Studio's handoff labels.
- **Track UI-quality gaps as their own beads** under the UX epic (`personal-cfo-4d8`), same as
  functional gaps.

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

### 7.4 Page layout: priority-ordered and container-aware

The convention every page follows when it arranges more than one block. It comes from the Settings
layout work (`personal-cfo-twpo.3`), whose reviewed specification is
[`docs/design-system/settings-layout-review.md`](../design-system/settings-layout-review.md) (merged
in #187), and from the Paper layout contract **LCR-1 v1** (page `p-8-0`, frozen 2026-10-27
15:21:48 UTC, hash `0afec932`; `design-system.md` section 10). Settings is the first page to apply
it (`personal-cfo-4d8.39`); the numbers below are Settings' and are examples of the method, not
constants for every page.

**The rules**

1. **Measure the container, not the window.** The width that matters is the **usable width of the
   main pane**: what is left after the sidebar and the page's horizontal insets. The sidebar can
   collapse, so one window gives two usable widths and a layout keyed to the window gets one of
   them wrong. Mark the page wrapper `@container` (it needs a definite width, such as `w-full`) and switch with
   container variants (`@min-[656px]:`), not viewport breakpoints (`md:`, `lg:`). Container
   queries and `subgrid` are supported by the WebKit that macOS 14 ships (14 is the minimum
   supported version today, ADR 0065).
2. **Decide the columns from the smallest tile that still reads.** A closed tile needs a minimum
   width (Settings: 320px) and gutters are fixed (16px), so the column count falls out of the
   usable width (Settings: three from 992px, two from 656px, otherwise one). State the thresholds
   for the page; do not guess per card.
3. **Spans are explicit, per section, per width.** Write the map (a table: group and state by
   three, two and one columns) and implement exactly that. A deliberate full-row item is allowed
   and is stated; **no orphan** cell and no auto-placement filling a gap.
4. **Priority order is source order.** The DOM order is the reading order and the focus order at
   **every** width. **No dense packing** (`grid-flow-dense`) and no visual reordering (`order-*`)
   to make a layout fit.
5. **Collapsed siblings are as tall as the tallest in their actual row, at their natural height.**
   Equalize within the row the browser lays out, not across the page. **Never** a global height,
   a fixed height or a line clamp to force equality, and nothing is clipped. An **expanded**
   editor keeps its natural height.
6. **Parallel controls align at their top edges** on shared tracks (a grid with `subgrid` rows, so
   the tracks are the same across siblings), **preserving the source control sizes** (40, 36 and
   32px). **Help text sits below the control.** Labels, help and errors **wrap and grow their
   row**; when the lanes cannot fit, they **stack**.
7. **Errors grow naturally and keep their controls.** An error or a remedy is never cut off and
   its action never leaves the page. A narrow stress width (Settings' 368px example) is a stress
   fixture, **not** a promised minimum.

**Good and bad**

```tsx
// GOOD: the container decides; spans are explicit; DOM order is the reading order.
<div className="@container">
  <div className="grid gap-4 @min-[656px]:grid-cols-2 @min-[992px]:grid-cols-3">
    <Card>{/* backups */}</Card>
    <Card>{/* comfort */}</Card>
    {/* deliberate: the third item takes the full row at two columns, one cell at three */}
    <Card className="@min-[656px]:col-span-2 @min-[992px]:col-span-1">{/* categorization */}</Card>
  </div>
</div>

// GOOD: parallel controls share the parent's row tracks, so the controls' tops align whatever
// each label's height. DOM order stays lane by lane (label, then its control).
<div className="grid grid-cols-2 gap-x-4">
  <div className="row-span-2 grid grid-rows-subgrid gap-y-2">
    <Label>Household timezone</Label>
    <NativeSelect />
  </div>
  <div className="row-span-2 grid grid-rows-subgrid gap-y-2">
    <Label>Base currency</Label>
    <Input />
  </div>
</div>
```

```tsx
// BAD: viewport breakpoints. The sidebar changes the usable width, so "lg" lies.
<div className="grid lg:grid-cols-3">

// BAD: dense packing / visual reorder: the focus order no longer matches what the eye reads.
<div className="grid grid-flow-dense"> <Card className="order-last" />

// BAD: forcing equal height by clipping or capping it. Long text is lost.
<Card className="h-[148px] overflow-hidden"> <p className="line-clamp-2" />

// BAD: unequal collapsed siblings in one row (ragged edges) from self-start.
<div className="grid items-start">
```

**LCR-1 pre-owner visual check.** Before a layout is shown to the owner, each of these holds, with
evidence:

- [ ] Collapsed siblings are the tallest's natural height within each actual row; none clipped;
      no global height.
- [ ] Expanded editors are natural height.
- [ ] Parallel control tops align; control sizes are the source sizes; help is below.
- [ ] Long labels, help and errors grow rows; lanes stack when they cannot fit.
- [ ] Each section's span map is right at every width, with any full-row item deliberate, no
      orphan and no dense reorder.
- [ ] DOM and focus order is source order at every width.

**Evidence.** Capture it at **actual** usable widths and **on each side of every transition**
(Settings: 992/991px and 656/655px), with the sidebar **open and closed**, in **Light and Dark**,
with **long and absent** summaries, and in **open, closed, mixed, error and partial** states.
Screenshots use **fake data only**. A **Designer critique** and an **independent review at the
exact freeze, manifest and commit** come before the owner sees it. Layout evidence does **not**
establish contrast: the required form-control boundary gap (`personal-cfo-4d8.35`) stays a release
blocker and is not waived by a layout approval.

### 7.5 The shared disclosure (A2): contract and what exists

The Paper contract **A2** (page `p-7-0`, frozen 2026-10-26 05:24:16 UTC) is a heading button that
opens and closes a section. Its behavior is specified in
[`settings-layout-review.md`](../design-system/settings-layout-review.md). The shared component is
`@/components/ui/disclosure` (`Disclosure` and `usePinDisclosures`, `personal-cfo-4d8.38`),
hand-authored in the shadcn style (section 7.1) on React primitives with no new dependency. **No
screen uses it yet**: each existing screen keeps its own until it is migrated, and a migration
reconciles that screen's differences (the "today" column) rather than changing its behavior in
passing.

How to use it:

- Put the heading text in `title` (it is the trigger's stable name). A `summary` shows only while
  collapsed and must come from real data.
- Put the error, its complete remedy and the one relevant action in `persistent`. It renders once,
  outside the hidden body, and is never collapsed away. Keep exactly one active retry.
- The body stays mounted when collapsed (`hidden` and `inert`), so drafts survive; do not unmount
  it to collapse. Collapse never submits, resets or cancels work.
- A component that owns a destructive confirmation calls `usePinDisclosures(active)` with `active`
  true while the confirmation is open, armed, or busy past the point of no return. Every enclosing
  disclosure stays open and its trigger turns `aria-disabled` (still focusable). The component's
  own Cancel clears its state and returns focus to the control that opened it. That control must
  exist after the state change: when the confirmation replaces it, focus it from a layout effect
  that runs after the re-render, not from the Cancel handler (the ref is still null there).
- No expansion state is persisted by the component.

| A2 contract | On `main` today |
|-------------|-----------------|
| One native trigger button inside the heading, with a stable name, `aria-expanded` and `aria-controls`; Enter and Space activate; Tab follows source order | `Disclosure` is the shared component; no screen uses it yet. Individual screens implement their own (`accounts/AccountsView.tsx`, `accounts/CreditCardsView.tsx`, `future-cash/NeedsConfirmation.tsx`, `future-cash/ProjectedActivityTable.tsx`, `transactions/TransactionFilterBar.tsx`), with varying semantics; reconcile them before reuse |
| Sections expand independently; the DOM is stable | In `Disclosure`; per screen elsewhere |
| Focus returns to the trigger **before** focused content is hidden; a hidden body leaves keyboard and accessibility navigation while ordinary drafts survive | In `Disclosure` (focus returns before the hide; the body is `hidden` and `inert` and stays mounted); not provided elsewhere |
| Errors, complete remedies and the one relevant action stay **outside** the hidden body; exactly one active retry; collapse never submits, discards drafts or cancels work | In `Disclosure` (`persistent` slot); not provided elsewhere |
| Destructive confirmations that are open, armed or irreversibly busy pin every ancestor open (focusable `aria-disabled` triggers); explicit Cancel resets and returns focus without collapsing; a blocking key rotation makes the rest of the app inert and cannot be dismissed | The rotation card exists (`settings/RotateKeyCard.tsx`) and is not yet inside a disclosure; ancestor pinning is `usePinDisclosures`, which no screen calls yet |
| Summaries reflect real data and never claim healthy or verified while loading or failed | `Disclosure` has a `summary` slot; no screen supplies one, so no collapsed summary is shipped |
| Respect reduced motion; announce a state change once; errors persist | `Disclosure`: the chevron respects reduced motion and the body does not animate; the state change is the native `aria-expanded` (no extra live region); the `persistent` slot keeps errors. Per screen elsewhere |

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
  `apps/desktop/src-tauri` run `cargo run --bin export_bindings --features
  export-bindings` (the feature gate keeps this dev-only tool out of `tauri
  build --target universal-apple-darwin`'s bundle, ADR 0072), and update the
  command list in `bindings.test.ts`. CI's `ipc-codegen` job fails if it drifts.
- A **pure-frontend** change leaves `bindings.ts` untouched.

## 10. Gates before a frontend PR

`pnpm typecheck`, `pnpm lint`, `pnpm test`, `pnpm -r build` — all green. The
standalone desktop crate (`apps/desktop/src-tauri`) is tested separately only when
Rust IPC changed.
