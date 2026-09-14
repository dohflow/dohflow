# ADR 0054 — Categorical chart palette: four validated slots, fixed order, never cycled

- **Status:** Accepted
- **Date:** 2026-08-02
- **Beads:** `personal-cfo-4d8.27.4.5` (this decision + the tokens), `personal-cfo-hnba`
  (the cycling defect), `personal-cfo-azeb` (the deferred heatmap + its sequential ramp)
- **Extends:** ADR 0031 (UI quality + design workflow)
- **Constrained by:** the design system's brand hues (`docs/design-system/design-system.md`)
- **Governs:** `--chart-*` in `apps/desktop/src/styles/globals.css` and every chart that
  assigns a series colour

## Context

The app shipped five chart tokens (`--chart-1`…`--chart-5`) chosen by eye from the brand
palette. Nobody had ever run them through a colour-vision check. Doing that as part of the
dataviz pass found that **the shipped palette fails in both modes**, and that one failure
is not cosmetic.

Measured with the Machado–Oliveira–Fernandes 2009 CVD simulation at severity 1.0, ΔE as
Euclidean distance in OKLab ×100 — the same standard the rest of this decision uses:

| Mode | Check | Result |
| --- | --- | --- |
| dark | adjacent CVD separation | **ΔE 1.6** between `chart-1` `#12a06b` and `chart-2` `#db8f6b` under protanopia (gate ≥ 8) |
| dark | lightness band | 3 of 5 slots outside L 0.48–0.67 |
| dark | normal-vision floor | ΔE 14.9 between clay and amber (gate ≥ 15) |
| light | chroma floor | emerald `#006341` and clay `#8e4a30` below C 0.10 — they read as gray |
| light | contrast vs card | terracotta 2.58:1, amber 2.95:1 (gate ≥ 3:1) |

**The dark-mode number is a live accessibility defect, not a theoretical one.**
`accounts/DebtBurndownChart.tsx` paints *avalanche* with `--chart-1` and *snowball* with
`--chart-2`. Those are the two colours that collapse to ΔE 1.6. A reader with protanopia
(roughly 1 in 12 men) cannot tell the two payoff strategies apart in dark mode — on a
chart whose only job is telling payoff strategies apart.

Separately, two charts **cycle** the palette with `i % length`
(`future-cash/MultiSeriesChart.tsx`, `accounts/DebtPerDebtChart.tsx`), so a 5th series is
painted identically to the 1st. `MultiSeriesChart` is worse than a plain wrap: it filters
out colours already used by drawn tiers, and when every tier is drawn that filter empties
the palette and falls back to the unfiltered list — so an account line can take the exact
colour of an aggregate line it is plotted against. `docs/design-system/recipes.md` was
*instructing* this ("More series → … cycling `--chart-4`, `--chart-5`"), so the pattern
was documented, not accidental.

## Decision

### 1. Four categorical slots, not five

`--chart-1..4` = **emerald · terracotta · blue · clay**. `--chart-5` is removed.

Four is what these five brand hues can actually support while every adjacent pair clears
the gates in **both** modes without a hue drifting off-brand. Keeping a fifth slot was
possible only by pushing dark-mode amber to `#896304` — a dark olive that no longer reads
as amber at all. A palette slot that has to stop being its own colour to fit is not a slot;
it is a promise the design system cannot keep.

This is a smaller cost than it looks: the method's own series ladder treats 1–3 as
comfortable, 4 as needing direct labels, and 5+ as a soft cap best served by folding or
faceting. No shipped chart draws more than four series today.

### 2. The values

Derived by holding each brand hue's OKLCH **hue angle** fixed and searching only its
lightness/chroma step and the slot order, minimising perceptual drift from the shipped
token subject to every gate passing. The brand hues are unchanged; their steps moved.

| Slot | Hue | Light | Dark | Drift from shipped (light / dark) |
| --- | --- | --- | --- | --- |
| `--chart-1` | emerald | `#006945` | `#11a06b` | ΔE 1.9 / **0.0** |
| `--chart-2` | terracotta | `#d38058` | `#e46c29` | ΔE 4.1 / 8.2 |
| `--chart-3` | blue | `#3a6fa5` | `#6499d2` | ΔE 0.0 / 2.2 |
| `--chart-4` | clay | `#8e492e` | `#c87450` | ΔE 0.3 / 0.2 |

Both palettes pass all five computable checks:

- **light** (surface `#ffffff`): worst adjacent CVD ΔE **14.7**, normal-vision ΔE **20.3**,
  all slots in band, all ≥ C 0.10, all ≥ 3:1.
- **dark** (surface `#151c18`): worst adjacent CVD ΔE **8.7**, normal-vision ΔE **21.5**,
  all slots in band, all ≥ C 0.10, all ≥ 3:1.

Emerald keeps slot 1 — it is the brand's money colour and leads every chart — and dark-mode
emerald is unchanged to three decimal places. The visible change is **terracotta becoming
warmer and more orange**. That is the entire price of separating it from emerald under
protanopia, and it is the change that fixes the debt chart.

`--terracotta` (the brand accent token, used by non-chart UI such as the build badge) is
**not** changed. A chart slot is derived from a brand hue but stepped for legibility on a
chart surface; the two tokens are allowed to differ and now do.

### 3. Fixed order, never cycled

Slots are assigned **in order, once**. No `i % palette.length`, no generated hue, no
"reuse from the top". Past four series a chart **folds the tail into an explicit "Other"
series, facets into small multiples, or caps the selection and says so** — it never
reuses a colour, because a reused colour is indistinguishable from the series it
duplicates and the legend then shows the same swatch twice.

The slot order is itself the colourblind-safety mechanism — adjacent pairs are what get
checked — so **the order is not a free choice** and must not be reshuffled per chart.

### 4. Colour follows the entity, not its rank

A series keeps its slot when other series are filtered out. Hiding a tier must not
repaint the survivors, or the reader's memory of "green is Net cash" breaks between
renders.

### 5. The palette is enforced by a test, not by review

`apps/desktop/src/styles/chart-palette.test.ts` re-runs the checks against the tokens
parsed out of `globals.css`. Changing a `--chart-*` value without clearing the gates fails
the suite. This is the same mechanical-enforcement posture as `copy-review.test.ts` for
ADR 0018: a rule nobody re-derives by hand is a rule that survives.

The test owns the thresholds and the CVD model, so the standard lives in the repo rather
than in a tool that happened to be installed the day the palette was written.

### 6. What this decision does NOT cover

**Sequential and diverging ramps are not defined here.** They encode magnitude and
polarity, and their check is lightness monotonicity across the ramp — not adjacency CVD.
Defining them without a consumer would repeat the mistake this codebase already made
three times over (`DataTable` shipped `leadingRow`, `footer` and `hideHeader` with no
production consumer). The sequential ramp lands with the heatmap that needs it,
`personal-cfo-azeb`.

## Consequences

- Every shipped chart changes colour: `FutureCashChart`, `MultiSeriesChart`,
  `AccountBalanceChart`, `DebtBurndownChart`, `DebtPerDebtChart`, and the credit-limit
  meter in `AccountDetailView`.
- `MultiSeriesChart` and `DebtPerDebtChart` must stop cycling (`personal-cfo-hnba`).
- `--chart-5` is removed; `DebtPerDebtChart` was its only code consumer.
- `recipes.md` loses its "cycling" instruction — it was teaching the defect.
- `design-system.md`, `design-tokens.json` and `component-gallery.html` are updated to
  match `globals.css`, which stays authoritative.
- The four-slot cap is a real constraint on future charts. A surface that genuinely needs
  more than four categories uses "Other", small multiples, or a table — and that is a
  design conversation, not a request for a fifth token.
