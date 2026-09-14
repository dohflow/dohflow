# Recraft session 2 — the chosen direction, refined

Follow-on to [`recraft-session-1.md`](./recraft-session-1.md). Bead `personal-cfo-n76x.3.3`.

The owner's own experiments landed on a solid **D** split by a flowing wave into
four colour bands. This session refines that specific mark rather than exploring.

**Settings:** Style **Vector Illustration** · aspect **1:1** · set the palette
swatches to `#006341`, `#1c1815`, `#FFFFFF`, `#DB8F6B`.

**Upload your best previous output as a style reference this time.** Session 1
said not to, because a reference collapses variance — but variance is no longer
the goal. You have a direction; now you want variations of it.

---

## The prompt

```
A minimal flat vector logo mark for a personal finance app called DohFlow. A solid, filled letter D: perfectly straight vertical left edge running the full height, and one large smooth rounded bowl on the right. Softly rounded corners.

The D is divided by a single flowing S-curve wave into four horizontal bands, from top to bottom: a large deep emerald green area (#006341), then a black band (#1c1815), then a white band (#FFFFFF), then a warm terracotta area (#DB8F6B) filling the base.

CRITICAL: the shape must be completely solid and closed with no transparent gaps or holes anywhere. Every band must extend all the way to the straight left edge and touch it. The black band and the white band must be exactly parallel to each other, following the same curve at constant thickness along their whole length, like two ribbons laid together.

Completely flat colour. No gradients, no shading, no highlights, no 3D, no drop shadows, no outlines, no texture. Reads clearly at 16 pixels. Square composition, centred, generous margin. NO TEXT.

NEGATIVE: gradients, shading, 3D, clay texture, glossy highlights, drop shadows, transparency checkerboard, outlines, strokes; Play-Doh, play doh, modeling compound, plastic tubs of clay, squishy toy compound; bread, dough, baking, bakery, pastry, flour, wheat; Simpsons, Homer, "D'oh"; coins, dollar signs, piggy banks, stacks of cash; fintech gradients, glassmorphism, neon, crypto aesthetics; text, lettering, words, numerals, watermarks, signatures.
```

### Shorter version, if the above gets diluted

```
Flat vector logo mark: a solid letter D with a straight vertical left edge and a large rounded right bowl. Divided by one smooth S-curve wave into four bands top to bottom — deep emerald green #006341, black #1c1815, white #FFFFFF, terracotta #DB8F6B. Completely solid and closed, no transparent gaps. All bands reach the straight left edge. The black and white bands are exactly parallel at constant thickness. Flat colour only, no gradients or shading. Reads at 16 pixels. Square, centred. NO TEXT.
NEGATIVE: gradients, shading, 3D, texture, outlines, transparency checkerboard, Play-Doh, bread, bakery, Simpsons, coins, piggy banks, fintech gradients, text, lettering, numerals.
```

---

## Reject an export on any of these

Learned the hard way from the last batch — check before you get attached to one:

1. **A white background rectangle baked in.** Open the SVG and look for a path
   covering the full canvas in white. It ruins the app icon and dark mode.
2. **File over ~20 KB, or hundreds of paths.** The clay attempts were 439 KB and
   492 KB with 1,471 paths each, 68% of them sub-25px slivers — that is a traced
   bitmap, not vector art. A clean mark of this complexity is under 10 KB.
3. **Grey-and-white checkerboard squares in the artwork.** Recraft traced the
   transparency pattern into the file. It renders as visible squares.
4. **`fill-opacity` anywhere.** Semi-transparent slivers are anti-aliasing
   artifacts and will follow the mark into every export.
5. **Colours outside the four.** Previous exports smuggled in `#1A151D`, `#D9785A`,
   `#073826`, `#FEF4EC`, `#201324` and several greys.
6. **A gap on the left edge.** Zoom in. The four bands must all touch the vertical.
7. **Black and white bands drifting out of parallel** — usually widening toward
   the left, where the wave tapers.

Quick check on any candidate:

```sh
grep -o 'fill="[^"]*"' candidate.svg | sort -u   # should show only the four colours
grep -c '<path' candidate.svg                    # single digits, not hundreds
grep -c 'fill-opacity' candidate.svg             # must be 0
```

Then view it on white, on `#0e1411`, and on a mid-grey — a solid four-colour mark
must look identical on all three.
