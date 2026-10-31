# Dev-only tools

Not part of the app bundle: nothing in the app imports this folder, and `index.html` is the only
build entry. Fake data only.

## Contrast audit (`personal-cfo-4d8.35`)

Renders the **real** `Badge`, `Input`, `NativeSelect`, `Combobox`, `Button`, `EmojiPicker`,
`PaginationControls`, `ThemeToggle` and `AppearanceCard`, **and** every `border-input` class string found
in the source, in Light and Dark on both supported surfaces (`--background` and `--card`), then reports what
was actually **painted**: the composited background, the tinted fill, the border, the focus ring and the badge
mark, with the contrast ratio of each pair, the pass or fail against the floors plus 0.1 of headroom, and the
engine's version.

```bash
pnpm -C apps/desktop exec vite --port 5199 --strictPort
# open http://127.0.0.1:5199/contrast-audit.html in any browser
```

The summary line, the failure list and the full row table are on the page; the same data is in
`window.__contrastAudit` (`{ engine, rows }`). **Open it in Safari as well**: Safari's WebKit is the closest
engine to the app's WKWebView, and the page needs nothing else. If its numbers differ from the committed
evidence by more than the 0.1 headroom, that is a finding.

How it measures, and what it does **not** do:

- Colors are composited through a 1x1 canvas (`measure.ts`), so `color-mix` tints, alpha borders and any color
  syntax the engine understands resolve as they paint. The WCAG math is `contrast-math.ts` (unit tested).
- **Hover and focus** are not simulated with a pointer. The page finds the rules the stylesheet declares for
  `.<class>:hover` and `:focus-visible` (Tailwind v4 nests them) and applies their declarations to a tile that
  has the control's original classes, with transitions off so it reads the final state. It **fails loudly** if a
  tile whose classes declare a state variant receives no rule. A real mouse hover and a real Tab were compared
  against this for the shared controls (see the evidence README).
- Disabled controls are exempt (WCAG 1.4.11 exempts inactive components), and a consumer whose boundary is not
  what identifies it (a labelled button, a chip, a divider, a native checkbox) is **reported but not
  required**, with the reason shown. Everything else is a field and must clear 3:1 plus headroom.

## Status fit (`personal-cfo-4d8.35`, the owner's timestamp condition)

Renders the **real** `ConnectionRow` and `CardStatsStrip` with the real status words (short, same-year and prior-year
stamps, a long name, a pending refresh, each due state) in a component of a chosen width, and reports whether anything
is clipped, extends past the component, overlaps a control, or scrolls sideways.

```bash
pnpm -C apps/desktop exec vite --port 5199 --strictPort
# http://127.0.0.1:5199/status-fit.html?w=368&theme=dark&set=stale
```

`w` is the component width in px (the evidence uses 368 and 672), `theme=dark` selects Dark, and `set=stale` keeps only
the rows that stress the fit. Each row's label says "fits" or lists its problems; `window.__statusFit` carries the same.
Open it in Safari as well (and a narrow `w`) if the engine matters.

## Disclosure example (`personal-cfo-4d8.38`)

`disclosure-example.html` shows the shared `Disclosure` with a long title, a persistent error and retry, a collapsed
summary and a nested delete guard. `?theme=dark` and `?open=0` change the state.
