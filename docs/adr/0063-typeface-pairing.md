# ADR 0063 — Typeface pairing: Nunito for display, IBM Plex Sans for UI and data

- **Status:** Accepted (owner decision, 2026-09-04)
- **Date:** 2026-09-04
- **Supersedes:** the single-family Inter choice in ADR 0033 / `personal-cfo-x99h`
- **Bead:** decision recorded here; implementation is `personal-cfo-<font-swap>`
- **Related:** ADR 0054 (chart palette + contrast discipline), ADR 0061 §7 (token
  generation), `docs/product/brand-direction.md`, `docs/product/brand/prompt-kit.md`

## Context

The design system has used **Inter** as its single family since the design-system
bead (`x99h`). It was a safe default and never a considered choice.

Two problems with keeping it.

**It reads as unconsidered.** Inter is the default of Linear, Vercel, and most
modern SaaS. It is not a bad typeface — it is an *invisible* one, and for a
product whose entire differentiation is having a point of view, defaulting is a
missed signal. (The owner framed this as looking machine-generated; the more
precise diagnosis is that it looks like nobody chose.)

**It fights the brand.** `brand-direction.md` commits to a claymation and
talavera-pottery visual language — hand-thrown, rounded, warm. A neutral grotesk
carries none of that.

But the same document sets a hard constraint in the other direction:

> Financial numbers stay sober: whimsy lives in illustration, motion, and copy
> accents — **never in the digits, tables, or forecasts.**

A single warm rounded family across the whole product would violate that on every
balance, forecast and table in the app. The two requirements are genuinely in
tension, and one family cannot satisfy both.

## Decision

**A two-family system**, splitting on exactly the line `brand-direction.md` draws.

| Role | Family | Applies to |
|---|---|---|
| **Display** | **Nunito** (variable, 200–1000) | Wordmark, headings, hero and marketing copy, section titles, illustration captions |
| **UI + data** | **IBM Plex Sans** (variable, wght 100–700, wdth 75–100) | Body text, tables, forecasts, balances, forms, all numerals, all data-dense surfaces |

The pairing is not decoration. It **encodes the brand principle in the type
system**: warmth where the brand speaks, sobriety where the money is.

### Why Nunito survives the test that matters

Money columns must align. Inspecting the actual font binary rather than trusting
documentation:

```
Nunito OpenType features: aalt calt case ccmp dnom frac kern liga locl
                          mark mkmk numr onum ordn salt sinf ss01 ss02 subs sups
  tnum: ABSENT
  digit advance widths: all ten glyphs = 600 units
```

**There is no `tnum` feature, and it does not need one — Nunito's figures are
already tabular by default.** A feature-presence check would have wrongly
rejected it; the right question is whether the digits align, and they do.

Two consequences to carry into implementation:

1. **The variable default weight is 200 (ExtraLight).** Any usage that does not
   set a weight explicitly renders wispy. Set it everywhere.
2. **`font-variant-numeric: tabular-nums` becomes a no-op under Nunito.** Harmless,
   but the declaration would be claiming work it is not doing. Keep it — it is
   load-bearing for IBM Plex Sans, which is where the numerals actually live.

### Why IBM Plex Sans for the data face

- **Digits are uniform-width by default**, like Nunito — alignment is not at risk.
- **Its digits are uniform-width by default** (600 units), so column alignment does
  not depend on an OpenType feature surviving the build. **This is the deciding
  advantage.**
- It has a genuine engineered character, so the pairing reads as deliberate
  contrast rather than two arbitrary sans faces.
- Variable, with a width axis available for dense tables if ever needed.

**Public Sans was the runner-up and was rejected on evidence:** its digits are
**not** uniform by default, so alignment depends on the `tnum` feature actually
being present at render time — and the correction below shows exactly why that is
a real risk rather than a theoretical one.

## Correction, 2026-09-05

The original text of this ADR gave IBM Plex Sans's **slashed zero** as the
deciding advantage. **That was wrong, and the error is instructive.**

The `zero` feature was verified in the desktop TTF from Google Fonts. The build
we actually ship is `@fontsource-variable/ibm-plex-sans`, whose subset woff2
contains only:

```
ccmp dnom frac kern liga mark numr
```

No `zero`. No `tnum` either. `font-variant-numeric: slashed-zero` and
`font-feature-settings: "zero" 1` were both tried in a browser against the
shipped file and rendered **no visible change**; the CSS was inert and has been
removed.

**The conclusion survives, the reasoning does not.** Plex remains the right data
face — but because its digits are uniform-width *by default*, which is exactly
the property that does not depend on subsetting. Public Sans, which needs `tnum`,
would have been the more fragile choice for precisely the reason this correction
uncovered.

**The transferable lesson: verify the artifact you ship, not the source it came
from.** A feature present in a foundry TTF may not survive webfont subsetting,
and the failure is silent — no build error, no console warning, just a
declaration that does nothing.

### Self-hosting

Both ship as `@fontsource-variable/nunito` and `@fontsource-variable/ibm-plex-sans`
(both v5.3.0). Self-hosting is mandatory, not preference: the site CSP is
`font-src 'self'` (ADR 0061 §6), so Google Fonts is not an option, and the app is
offline-first and cannot depend on a CDN at all.

## Consequences

**Good.** The brand principle becomes structural rather than a guideline someone
must remember. The product stops looking like every other SaaS. Numerals gain a
slashed zero. Both families are variable, so weight range costs no extra requests.

**Costs.** Two families instead of one — more font bytes, and a rule contributors
must learn. The token file gains a second family, so every consumer must be
updated together. The wordmark must be re-typeset in Nunito, which means the
temporary favicon and OG card from `n76x.4` need regenerating (they were already
marked TEMPORARY, so this is not new work — just now it has a reason).

**The rule, stated so it can be enforced:** if a number can appear in it, it is
IBM Plex Sans. Headings that contain figures — "Forecast: $4,210" — are data,
not display.

**Migration is not partial.** `docs/design-system/design-tokens.json` is the single
source of truth and generates both the app's styles and the site's `tokens.css`
(ADR 0061 §7). The swap lands in one change across both, or the two drift into
different type systems — the exact failure that generation exists to prevent.
