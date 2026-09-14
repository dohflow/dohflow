# Claude Design prompts — the `4d8.27` wave's lane-3 surfaces

**Bead:** `personal-cfo-4d8.27.4.6` · **Governed by:** ADR 0031 §2 (three lanes) and §3
(the loop, and why the agent can't drive it)

## Why this file exists, and what changed about the bead

ADR 0031 routes UI work into three lanes. Lane 3 — *novel layout, dense information design,
or real interaction-design questions* — is supposed to be **mocked in Claude Design before
being built**, with the agent writing a ready-to-paste prompt because `/design-login` is
tty-gated and a headless session has no terminal to run it in.

The bead's acceptance criteria assumed those prompts would sit in each **needs-design child
bead**, written *before* the screen shipped. That is not what happened: **the wave's lane-3
surfaces were built directly from primitives**, and they shipped. A prompt written into a
closed bead now would be inert — nobody reads a closed bead's design field.

So this file is the reconciliation. It is the tracked list the criteria ask for, pointed at
the surfaces that **actually exist**, and the prompts are written to be run against them —
which makes them *more* useful than the pre-build version would have been, because each one
can name what shipped and what specifically to interrogate.

**Run these when you want a design pass**, not as a gate. Everything below is live and
working; this is polish, and it is sequenced after the launch gate.

---

## How to run one

1. Open Claude Design, `/design-login` (needs your terminal — the agent cannot).
2. Paste, as context: `docs/design-system/design-system.md`,
   `docs/design-system/design-tokens.json`, and a screenshot of
   `docs/design-system/component-gallery.html`.
3. Paste one prompt below.
4. Save the result into the project and tell me; I'll rebuild the surface against it.

---

## 1. Debt page

> Design the **Debt** page for a local-first personal-finance desktop app.
>
> **Purpose.** One destination for debt *analysis* — what is owed, what it costs, and how it
> pays down. Account identity and balances live on a separate Accounts page and must not be
> duplicated here.
>
> **Data.** Zero or more credit cards and loans. Per card: name, balance owed, APR, credit
> limit, statement close and payment due dates, projected statement amount. Per loan:
> balance owed, APR, monthly payment, payoff date. Plus: a spending-by-category breakdown
> for the selected cards, a transaction list scoped to the selected debts, and a payoff
> comparison of three strategies (minimum-only / snowball / avalanche) with a debt-free
> month and total interest each.
>
> **The interaction question I most want your answer on.** A selector at the top picks one
> or many debts and everything below scopes to it. The page's *shape* must not change with
> how many are selected — no separate "single" and "multi" layouts. How should the selector
> and its current scope read so that, at a glance, the user knows which debts every figure
> below refers to?
>
> **Four states.** Loading; empty (no cards or loans yet); error (a read failed); and the
> populated state, which should be designed for **five debts**, not one.
>
> **Component basis.** shadcn/ui. Existing primitives: card, table, chart (Recharts wrapper),
> ranked horizontal bars, badge, native select, button. Prefer composing these.
>
> **Tokens.** Use the provided design tokens only — no raw hex. Emerald `#006341` primary,
> Saltillo terracotta `#DB8F6B` accent, warm-neutral grays. Money is right-aligned and uses
> tabular figures. Liabilities are shown as a **positive amount owed**, never a negative.
>
> **Light and dark**, both. Charts have a four-slot categorical palette; do not introduce a
> fifth colour.
>
> **Tone.** Descriptive, never advisory. It states balances, rates and projected paydown; it
> never ranks a debt as good or bad and never tells the user to pay something. "Avalanche"
> and "snowball" are neutral labels.

---

## 2. Transactions — list + spend breakdown

> Design the **Transactions** screen: a dense activity table with a spending-by-category
> breakdown above it.
>
> **Purpose.** Review and categorize what actually happened. This is the screen the user
> spends the most time in, and the one most likely to hold thousands of rows.
>
> **Data.** Per row: date, description/counterparty, account, category, tags, signed amount,
> a reviewed/unreviewed marker, and an expandable set of split lines. Above it: the same
> filtered set aggregated by category as ranked horizontal bars, drillable into
> subcategories with a breadcrumb trail.
>
> **The interaction question I most want your answer on.** The chart and the list must
> always agree — same filters, same answer — but the chart adds a category drill-down the
> list does not have. How should the drill state read so the user never wonders whether the
> list below reflects it?
>
> **Four states.** Loading (a paged table — show what a skeleton row looks like); empty (no
> transactions); no-match (filters exclude everything — distinct from empty); error.
>
> **Component basis.** shadcn/ui: table, card, chart, ranked bars, combobox (searchable
> category picker), badge, checkbox, pagination.
>
> **Tokens.** Provided tokens only. Money right-aligned, tabular figures, signed. Inflows and
> outflows use the semantic gain/loss tokens, not the chart palette.
>
> **Light and dark.** Include a multi-select state: rows are selectable and a bulk action bar
> appears.
>
> **Density is the point.** Show 25 rows. I want to see how you handle the row rhythm,
> column alignment, and where the eye lands.

---

## 3. Cash Flow — history and projection in one chart

> Design the **Cash Flow** chart: realized past and projected future on a single timeline.
>
> **Purpose.** Answer "how much cash will I have, and when is it lowest?" — the app's
> central question.
>
> **Data.** A daily balance series. To the left of today it is *realized* (what happened);
> to the right it is *projected*, and the projection carries an uncertainty band that widens
> with distance. Three aggregate series (Spendable, Reserve, Net) plus optional per-account
> series. A shaded comfort band showing a target cash range. Markers where the projection
> crosses zero.
>
> **The interaction question I most want your answer on.** Realized and projected are
> *epistemically different* — one is fact, one is an estimate — but they are one continuous
> line. How should that boundary read without implying the past is uncertain or the future
> is certain?
>
> **Four states.** Loading; empty (not enough data to project); error; populated.
>
> **Component basis.** shadcn/ui chart (Recharts). Line, area for the band, reference line
> for today, reference area for the comfort band.
>
> **Tokens.** Four-slot categorical palette, provided tokens only. The uncertainty band must
> read as *less* certain than the line without becoming invisible in dark mode.
>
> **Light and dark**, both — the band is the hard one.
>
> **Tone.** Descriptive. It shows a projection and its uncertainty; it does not advise.

---

## 4. Scenarios

> Design the **Scenarios** page: named what-if plans layered over the real forecast.
>
> **Purpose.** Author a set of changes ("Rent hike", "New job"), see their effect on the
> forecast, and optionally apply one so the real forecast adopts it.
>
> **Data.** Per scenario: name, status (draft / active / archived), a change count, an
> optional expiry, and whether it has been applied. Per change: what it targets (a bill, an
> income source, a category) and the new value or date.
>
> **The interaction question I most want your answer on.** Several scenarios can be *stacked*
> and composed at once, and **the stacking order decides which wins** when two change the
> same bill. How should an ordered stack read — and be reordered — so precedence is obvious
> rather than something the user has to remember?
>
> **Four states.** Loading; empty (no scenarios yet — this is a first-run moment worth
> designing); error; populated with four scenarios, one applied and one archived.
>
> **Component basis.** shadcn/ui: card, badge, dialog, button, native select.
>
> **Tokens.** Provided tokens only. An applied scenario needs to read as *materially
> different* from a draft without using the semantic warning/error colours — it is not a
> problem state.
>
> **Light and dark**, both.
>
> **Tone.** Descriptive. Applying is consequential and reversible; the confirmation should
> state what changes and how to undo it, without discouraging or urging.
