# Brand asset provenance

Every brand asset that ships gets a row here **before** it is used.

**Why this file exists.** US Copyright Office guidance (January 2025) is that
prompts alone earn no copyright. What is protectable, and what a trademark
position rests on, is the record of *human* authorship — selection, editing,
refinement. The **"Human edit"** column is therefore the most important one:
"picked from 40" is a real answer; an empty cell is a problem.

It is also simply operational memory. A year from now, "which model made this,
what did we change, and can we regenerate it consistently?" is a question
someone will ask.

## How to fill a row

| Column | What goes in it |
|---|---|
| **Asset** | The file path that ships |
| **Date** | ISO date the asset was accepted |
| **Tool + version** | e.g. `Recraft V4.1 Vector`, `Nano Banana Pro (Gemini 3 Pro Image)`, `hand-authored`, `Nunito (typeset)` |
| **Prompt / method** | The prompt verbatim, or "hand-authored SVG", or "typeset in Inter" |
| **Reference chain** | Which accepted asset was passed as the style anchor, if any |
| **Human edit** | **What a person actually changed.** Selection counts; be specific |
| **Bead** | The bead the work was done under |

Assets that were **never generated** still get a row — the wordmark's row is the
evidence that it was typeset rather than produced by a model.

---

## Log

| Asset | Date | Tool + version | Prompt / method | Reference chain | Human edit | Bead |
|---|---|---|---|---|---|---|
| `docs/product/brand/mark.svg` | 2026-09-04 | Recraft V4.1 Vector + hand cleanup | Session-2 prompt (`recraft-session-2.md`) — solid four-band D, owner-directed after several rounds | Owner's own earlier D explorations fed back as style reference | **Owner authored the direction and selected this output from many rounds.** Agent cleanup: corrected `#DB8F6C`→`#DB8F6B` to match the token; gave the wave an explicit `#1c1815` fill (it had none and was rendering pure black); stripped C2PA metadata (10 KB→4.8 KB), unused `xlink`, and fixed width/height; added `role`/`aria-label`; path data cleaned to 1dp precision (4,775→3,554 B), verified geometry-preserving — max coordinate shift 0.05 units = 0.0008 px at 16 px render | `n76x.3.3` |
| `docs/product/brand/wordmark.svg` | 2026-09-04 | Nunito Variable @ wght 800, typeset by fontTools, converted to OUTLINES | Not generated — typeset. `scripts/wordmark.py` instantiates the variable font, applies GPOS kerning, and emits glyph outlines | — | Weight chosen at 800 to match the site h1; converted to outlines so the mark never depends on the viewer having Nunito, per ADR 0063 | `n76x.3.3` |
| `docs/product/brand/lockup-*.svg` (4 files) | 2026-09-04 | Composed by `scripts/lockup.py` | Mark + wordmark composed at fixed optical ratios; horizontal and stacked, each in emerald and ink | `mark.svg` + `wordmark.svg` | Spacing and scale ratios set by hand and checked at three sizes | `n76x.3.3` |
| `docs/product/brand/lockup-substitute*.svg` (3) | 2026-09-04 | Composed by `scripts/substitute.py` | Mark stands in for the D in DohFlow; remaining 'ohFlow' typeset in Nunito 800 with GPOS kerning | `mark.svg` + Nunito | Mark scaled to Nunito's actual cap height (705) and baseline-aligned with a 1.2% overshoot, matching how a round letterform sits. Three scales (100/94/88%) compared before choosing 100%. Emerald, ink and paper wordmark variants | `n76x.3.3` |
| `dohflow-site/public/icon.svg` | 2026-09-04 | hand-authored | Geometric "D": stem plus semicircular bowl, `fill-rule="evenodd"`, `prefers-color-scheme` swap | — | Authored by hand, no model involved. **TEMPORARY** placeholder | `n76x.4` |
| `dohflow-site/public/favicon.ico`, `apple-touch-icon.png` | 2026-09-04 | hand-authored (Pillow) | Same geometry rasterised at 8× and downsampled | `icon.svg` | Stem geometry corrected after visual review — first pass had a notch on the left edge. **TEMPORARY** | `n76x.4` |
| `dohflow-site/public/og-default.png` | 2026-09-04 | hand-authored (Pillow), San Francisco | Wordmark, tagline, terracotta rule, feature line on paper ground | — | Rebuilt after review: first version was ~80% empty space with no wordmark. Set in SF because the display face ships woff2-only. **TEMPORARY** | `n76x.4` |
| `apps/desktop/src-tauri/icons/*` (classic macOS / Tauri set: `icon.icns`, `icon.ico`, `icon.png`, `32x32`, `64x64`, `128x128`, `128x128@2x`, `Square*Logo`, `StoreLogo`) from `apps/desktop/src-tauri/icons/source/dohflow-icon.svg` | 2026-09-05 | Tauri CLI 2.11.2 `tauri icon` + hand-composed source SVG | `pnpm -C apps/desktop tauri icon src-tauri/icons/source/dohflow-icon.svg`. The source SVG wraps the mark's inner markup, copied verbatim (no re-drawing), on a paper plate | `mark.svg` | Layout decisions by hand: Apple classic macOS icon grid (1024 canvas, 824 plate at 100 px margins) with continuous-curvature corners (superellipse approximation, r 184.6 = 22.4% of the side, smoothing 0.6); plate filled with the paper token `#faf8f6` plus a 1 px inner hairline in the border token `#e3ddd5` so the plate keeps an edge on white surfaces; mark kept at scale 1 (64.3% of plate width, inside the 62–68% target) so the geometry stays literally verbatim, translated (1.36, 0.05) to center its bezier-exact bounding box. Checked at 512/64/32/16 px: D and wave read at 32, the ink band merges at 16 (inherent to the four-band mark; not simplified). `tauri icon` writes `icon.png` at 512, same as the placeholder it replaced; the 1024 master is the `ic10` slice of `icon.icns` and the source SVG. The CLI's `android/` and `ios/` output was not kept. Light variant only; the macOS 26 Liquid Glass icon is a separate bead. No text, gradient, or shadow | `4d8.28.2` |
| `apps/desktop/src/brand/brand-paths.ts` (+ `Brand.tsx` components) | 2026-09-06 | `docs/product/brand/scripts/emit-brand-paths.mjs` (Node, hand-written) | Not generated by a model. The script reads `mark.svg` and `wordmark.svg`, measures the mark's artwork bounds exactly as `lockup.py` does, and emits the path data as a TypeScript module; `Brand.tsx` composes the horizontal and stacked lockups with `lockup.py`'s ratios (gap 0.34 / wordmark 0.62; gap 0.22 / wordmark 0.40) | `mark.svg`, `wordmark.svg`, `lockup-*.svg` (proportions verified by test) | Fills mapped to design tokens rather than literals: emerald → `--primary` (brand emerald in light, the token's brighter green on dark surfaces), terracotta → `--terracotta`, ink → `--on-brand-fill`, white → `--brand-white`; the wordmark is `currentColor`. Geometry untouched. `Brand.test.tsx` re-derives the data from the SVGs so a hand edit on either side fails | `4d8.28.3` |

> Rows marked **TEMPORARY** are placeholders to be replaced by `n76x.3.5` once
> the real mark exists. Replace the row when you replace the asset — do not
> delete the history.
