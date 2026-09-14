# Dataviz conventions

How charts are built in this app, so they read as one product rather than six.

- **Bead:** `personal-cfo-4d8.27.4.5`
- **Governed by:** [ADR 0054](../adr/0054-categorical-chart-palette.md) (the palette),
  [ADR 0018](../adr/0018-forecast-language-and-non-advice-boundary.md) (descriptive, never
  prescriptive), [ADR 0031](../adr/0031-ui-quality-and-design-workflow.md) (when to mock
  first)
- **Authority:** `apps/desktop/src/styles/globals.css` for token values;
  [`design-system.md`](./design-system.md) for everything non-chart. If this doc disagrees
  with either, they win — fix this doc.

The rules below are not stylistic preference. Each one is here because it is either
measurable (the palette gates), or because it was broken in a shipped chart and cost
something real. Those cases are named, because a rule with its scar attached survives
longer than a rule without one.

---

## 1. Colour

**Four categorical slots, in order, never cycled.** `--chart-1..4` = emerald ·
terracotta · blue · clay. Assign slot 1 to the first series, slot 2 to the second, and so
on. Never `i % palette.length`, never a generated hue, never a fifth token.

Past four series: fold the tail into an explicit "Other", facet into small multiples, or
cap the selection and **say so on the surface**. Silently dropping a series the user
picked is its own kind of lie; reusing a colour is worse, because the chart then shows two
different things as one and the legend carries the same swatch twice.

> This is ADR 0054, and it exists because two shipped charts cycled. `MultiSeriesChart`
> also had a fallback that could paint an account line the exact colour of an aggregate
> drawn beside it, and `recipes.md` was *instructing* the cycling — so it was a documented
> practice, not a slip.

**Colour follows the entity, not its rank.** Hiding or filtering a series must not repaint
the survivors. A reader who learned "emerald is Net cash" is misled the moment it moves.

**One series → one colour.** A ranked bar chart of nominal categories (merchants,
spend categories, accounts) paints *every* bar in slot 1. Do **not** shade bars
darker-where-bigger: that re-encodes bar length as hue, spends the only free channel on
information the length already carries, and fails the palette gates by construction. A
single-series chart also needs **no legend box** — the title already names what is
plotted.

**Ordered categories are a different job.** Funnel stages, size tiers, age bands — where
reordering would change the meaning — take a one-hue ramp so the order is visible in the
colour. We have no such ramp yet; see §6.

**Status colours are reserved.** `gain` / `loss` / `warning` / `info` mean state, and
never stand in for "series 3". When a series genuinely *means* good/bad it wears status
tokens; when it is identity it wears categorical. Never both in one chart.

**Text never wears the series colour.** Bars, lines, dots and fills carry the colour;
labels, values, legends and axis text use `foreground` / `muted-foreground`. Identity
comes from a coloured swatch *beside* the text. The one exception is a label set inside a
filled mark, which picks white or ink by the fill's luminance.

**Money keeps its own rule.** Positive → `text-gain`, negative → `text-loss`, via
`signedAmountClass`. That rule outranks the series palette wherever a number is shown as
money.

### Enforcement

`apps/desktop/src/styles/chart-palette.test.ts` parses the `--chart-*` tokens out of
`globals.css` and re-runs the colour-vision checks on every test run. Changing a value
without clearing the gates fails the suite. Do not "fix" it by loosening a threshold — the
thresholds are the standard.

---

## 2. Picking the form

Decide this **before** colour. The reader's job picks the form, and sometimes the answer
is not a chart.

| The reader must… | Use | Not |
| --- | --- | --- |
| Read one current value (+ maybe a trend) | a stat tile / hero number | a one-bar chart |
| Compare magnitude across categories | **ranked horizontal bars** | a pie, a treemap |
| Follow a value over time | line; area for a single series | — |
| Tell distinct series apart | multi-line, grouped/stacked bar | — |
| See one series against context | **emphasis** — one in colour, the rest gray | eight hues |
| Compare against a baseline | diverging bar, or line vs baseline | a dual axis |
| Read more than ~7 meaningful classes | a table, or table + chart | more colours |

**Never a dual-axis chart.** Two y-scales on one plot invent a correlation that is not in
the data, because the alignment of the two scales is arbitrary. Two measures of different
scale → two charts, small multiples, or both indexed to a common base.

**Bars beat area for magnitude.** Length is compared precisely; area is not. That is why
the spend drill-down is ranked bars and not a treemap (`personal-cfo-4d8.27.8.4`).

**Emphasis is the most underused form here.** If the story is "this one moved", one series
in colour with the rest in `muted-foreground` says it better than five hues.

---

## 3. Marks

| Mark | Spec |
| --- | --- |
| Bar / column | **≤ 24px thick** — cap it, let the leftover band be air; **4px rounded data-end, square at the baseline** |
| Line | **2px**, round join and cap |
| Marker / end dot | **≥ 8px** (r ≥ 4), filled with the series colour |
| Area fill | the series hue at **~10% opacity** — a wash, never a saturated block |
| Grid / axes | one step off the surface (`border` / `muted-foreground`), **hairline 1px, solid** |

**Gridlines are never dashed.** Dashing reads as "projection" or "threshold". In this app
dashing carries actual meaning — `MultiSeriesChart` and `AccountBalanceChart` draw
*projected* segments dashed and *realized* segments solid — so spending it on chrome would
collide with a real signal.

**Separate marks with space, not strokes.** A **2px gap in the surface colour** between
touching marks (stacked segments, adjacent bars); a **2px surface ring** on dots that
overlap a line. A border drawn around a mark adds ink that is not data.

**Never let a fixed container height clip the axis band.** Size the container to the plot
*plus* its axis labels, or let it grow. A card with a tiny nested scrollbar is the
symptom.

---

## 4. Labels, legends, axes

- **A legend is present for two or more series; a single series gets none.** Never make
  colour-matching the only identity channel.
- **Label selectively.** Bars → value at the tip. Lines → value at the end. A value beside
  every point is chaos and goes unread; let the axis and the tooltip carry the rest.
- **A label that does not fit is moved, never clipped.** Outside the bar end, or into the
  tooltip. Never `overflow: hidden` on the mark — cropping the first characters of a label
  is worse than having no label.
- **Axis ticks round to clean numbers** and use `tabular-nums` so they align.
- **`tabular-nums` on columns, not on hero numbers.** Equal-width digits make a large
  standalone `121` look loose. Stat-tile and hero values use proportional figures; table
  rows and axis ticks use tabular.
- **Copy is descriptive, never prescriptive** (ADR 0018). "Dining: $842 in July" and
  "Groceries, up $118 from June" are fine. "You're overspending on dining" is not, and
  neither is framing a period-over-period delta as a verdict. `copy-review.test.ts`
  enforces the phrase list over the globs it scans — extend the globs when a chart lands
  on a surface it does not yet cover.

---

## 5. Interaction and accessibility

- **Every chart is a `<figure>` with an `aria-label` that states what is plotted**, and a
  `<figcaption>` carrying the visible legend. This is the shipped pattern —
  `FutureCashChart`, `AccountBalanceChart`, `MultiSeriesChart`, `DebtPerDebtChart` — keep
  it.
- **A tooltip enhances, it never gates.** Every value must also be reachable without
  hover: a direct label, the axis, or a table view. Keyboard focus shows what hover shows.
- **Hit targets are bigger than the mark** — roughly 24px minimum. An 8px dot you must
  land on dead-centre is not interactive.
- **Filters live in one row above everything they scope**, not inside a chart card, so
  every chart on the surface re-renders against the same slice. ADR 0052 §2 makes this
  concrete for spend: the chart and the list share one filter state, and clicking a cell
  *adds a filter* rather than opening a disconnected panel.
- **Interactive marks are real buttons** — focusable, with a visible focus ring.
- **No skeleton flash on refetch.** Skeletons are for the first load; a refetch holds the
  previous render at reduced opacity so the layout does not jump.
- **Four states, always** — loading, error, empty, success. Same rule as list surfaces
  (ADR 0053 §1); a chart is not exempt because it is drawn rather than laid out.

---

## 6. What is deliberately not defined yet

**Sequential and diverging ramps.** They encode magnitude and polarity, and their check is
lightness monotonicity across the ramp, not adjacency CVD — so the categorical validator
does not cover them and would fail a *correct* ramp by design.

They are not defined here because nothing consumes them. Defining them now would repeat
the mistake `DataTable` made three times over — `leadingRow`, `footer` and `hideHeader`
shipped with zero production consumers, and only a review caught it. The sequential ramp
lands with the heatmap that needs it (`personal-cfo-azeb`), validated with the ordinal
checks (monotone lightness, adjacent ΔL ≥ 0.06, light end still ≥ 2:1 on the surface).

**Texture as a backup identity channel** (for full CVD, print, `forced-colors`) is
likewise unbuilt. Our palette clears its gates on hue alone, so texture would be
decoration today.

---

## 7. Before you ship a chart

1. The form matches the reader's job (§2) — and it is not a dual axis.
2. Colours come from the four slots, in order, with no cycling (§1).
3. Marks match the specs; grid is a solid hairline (§3).
4. Legend present for ≥ 2 series; labels selective; no label clipped (§4).
5. It is a `<figure>` with a real `aria-label`; tooltips enhance rather than gate (§5).
6. All four states are handled.
7. Copy is descriptive (ADR 0018), and `copy-review.test.ts` scans the surface it lands on.
8. Check it against §1–§5 once rendered — the palette test checks colour, not layout.
