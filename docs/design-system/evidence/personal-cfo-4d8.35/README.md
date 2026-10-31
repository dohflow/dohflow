# Evidence: `personal-cfo-4d8.35` (semantic badge text, control boundaries and status fit)

Fake data only. Produced by the committed audit page, `apps/desktop/src/dev/contrast-audit.tsx` (how to run
it: `apps/desktop/src/dev/README.md`) and the status-fit page `apps/desktop/src/dev/status-fit.tsx`.
Source: `main` at `1ab8a84` (merged into this branch) plus this PR. **Values are the owner's decision**:
`--input` option A+ (Light `#90877f`, Dark `#607065`) and the foreground-words-plus-mark badge (Studio decision
bead `personal-cfo-4d8.46`, 2026-10-30).

## Engine

Two engines ran the same committed audit page against this head:

- **Chromium 152.0.7977.167** in the Claude desktop app's built-in browser pane (macOS, device pixel ratio 2,
  app build `Claude/3.6556.2`).
- **WebKit**: Safari in the iOS 27.0 Simulator (`AppleWebKit/605.1.15 ... Version/27.0`, device pixel ratio 3),
  opened at `http://localhost:5199/contrast-audit.html`. `webkit-contrast-audit-summary.jpg` shows its summary line.

Both report the same summary. **Neither is the macOS app's WKWebView**, and the iOS WebKit is not a certification
of it: `HEADROOM = 0.1` is a project buffer. A run in a packaged build's webview remains open; nothing here claims
the WKWebView numbers. The status-fit page was also opened in the WebKit simulator at a 330px component width
(`webkit-status-fit-330.jpg`); every row reported "fits". WebKit formats the prior-year stamp as
"Jan 5, 2025 at 04:35 PM" where Chromium prints "Jan 5, 2025, 04:35 PM": same length, so the fit holds.

## Result of the committed audit on this PR

`PASS 316 · FAIL 0 · info 124` (440 rows: 2 themes x 2 surfaces x every badge, shared control, state tile and
discovered consumer), on Chromium 152 **and** on iOS 27 WebKit, at `--input` option A+ and with `main` at `1ab8a84`
merged in (the earlier option-A run had the same counts).

## F1: NativeSelect hover, measured before and after

The audit **reproduced the Studio's finding** against the unfixed `hover:border-ring/60` and now passes
against `hover:border-ring`. Borders are painted over the control's own `--background` fill and compared
with the surface outside it and with the fill inside it.

| Surface | Before (60% ring) outside / fill | After (full ring) outside / fill |
|---|---|---|
| Light, `--background` | **2.05** / **2.05** | 3.55 / 3.55 (`#2e9470`) |
| Light, `--card` | **2.18** / **2.05** | 3.76 / 3.55 |
| Dark, `--background` | **2.74** / **2.74** | 5.56 / 5.56 (`#12a06b`) |
| Dark, `--card` | **2.55** / **2.74** | 5.17 / 5.56 |

Both sizes (`default` and `sm`) are identical. The Studio's source estimate was 2.05/2.11 Light and 2.75/2.67
Dark; the rendered numbers agree to within 0.1.

**Real hover versus derived hover.** With a real mouse parked over the real NativeSelect (`:hover` true) the
computed border is `rgb(46, 148, 112)` = `#2e9470`, equal to the audit's derived hover. A real Tab press gave
the focus ring `rgb(250, 248, 246) 0 0 0 2px, rgb(46, 148, 112) 0 0 0 4px` on an Input (Light) and
`rgb(14, 20, 17) 2px, rgb(18, 160, 107) 4px` (Dark), matching the derived focus tiles.

> Two false starts in building the audit, recorded because they would have produced wrong evidence:
> deriving a state by stripping the `hover:` prefix measured 1.27:1, an artifact (Tailwind never generates the
> un-prefixed class); and reading a state mid-transition returned the resting color in a hidden pane. The audit
> now applies the stylesheet's own state rules with transitions off, and a state tile that gets no rule fails.

## Raw pairs (rendered, Chromium 152)

**Control boundary, resting** (`--input` A+, painted over the control's `--background` fill):

| | border | on outside | on fill |
|---|---|---|---|
| Light / `--background` | `#90877f` | 3.33 | 3.33 |
| Light / `--card` | `#90877f` | 3.52 | 3.33 |
| Dark / `--background` | `#607065` | 3.55 | 3.55 |
| Dark / `--card` | `#607065` | 3.31 | 3.55 |

The tightest pair is Dark `--card` at **3.31**, 0.21 over the 3.1 the audit requires. Identical for Input,
NativeSelect (both sizes), Combobox trigger, EmojiPicker trigger and the pagination select. **Focus ring** (unchanged):
`#2e9470` 3.55 / 3.76 Light, `#12a06b` 5.56 / 5.17 Dark.

**Badge**, fill is the composited 10% tint, label is `--foreground`, mark is the token:

| Variant | Light page: fill / label / mark | Light card: fill / label / mark | Dark page: fill / label / mark | Dark card: fill / label / mark |
|---|---|---|---|---|
| gain | `#e2eae6` 14.40 / `#0f7a54` 4.36 | `#e7f1ee` 15.30 / 4.63 | `#11231c` 14.51 / `#2fb37e` 6.14 | `#172b22` 13.23 / 5.60 |
| warning | `#f2eade` 14.78 / `#b3780a` **3.14** | `#f7f1e6` 15.69 / 3.33 | `#232214` 14.18 / `#e0a82e` 7.49 | `#292a1a` 12.90 / 6.81 |
| info | `#e6e9ee` 14.49 / `#3a6fa5` 4.32 | `#ebf0f6` 15.39 / 4.59 | `#172125` 14.51 / `#6ba0d8` 5.97 | `#1d292b` 13.23 / 5.44 |
| loss | `#f4e4e2` 14.31 / `#c23b2e` 4.30 | `#f9ebea` 15.20 / 4.57 | `#231c19` 14.85 / `#e06a5e` 5.11 | `#29241f` 13.60 / 4.68 |

The tightest mark is Light warning on `--background` at **3.14:1**, 0.14 over the 3.0 floor and above the
3.1 the audit requires. **Caller overrides** are measured too: `Autopay` (10px, uppercase) keeps the same
ratios; `Resurfaced` (its own icon, `before:hidden`) is measured through the icon at the warning ratios;
a 62-character label in a 220px column wraps to **2 lines**, the mark stays vertically centered, and
**nothing overflows** in any of the 8 narrow rows.

## Raw consumer inventory (reconciled)

`border-input` matches in non-test source at `093dbb3`: **39 matches in 29 files** by
`git grep -n border-input -- apps/desktop/src ':!*.test.*'` less the one comment in `globals.css`. The earlier
**38/28** came from `grep -rn border-input . --include='*.tsx' | grep -v test`, where `grep -v test` drops any
*line* containing "test"; the Studio's count was right. 04b-review counts **40 in 30** at an earlier head of this PR,
which includes the one comment in `globals.css`.

Classified (the audit discovers these from the source and reports each, three states, four surfaces):

| Class | Count | Boundary required? | Result |
|---|---|---|---|
| Hand-rolled text, textarea and color-swatch fields (`input.tsx`, `BillsView:62`, `SplitEditor:19,305`, `AccountEditorDrawer:486`, `AddCategoryForm:140`, `CategoriesView:432`) | 7 | yes | pass |
| Hand-rolled `<select>` class strings (18 matches) | 18 | yes | pass |
| Shared NativeSelect, Combobox, EmojiPicker trigger, pagination select | 4 | yes | pass (hover and focus included) |
| Custom checkbox box (`SeriesPicker:162`: an `aria-hidden` span whose border is the unchecked state) | 1 | **yes**: the box is the visible state of the control | pass |
| Outline `Button` (`button.tsx:16`) | 1 | **no**: identified by its text | reported: boundary 3.31 to 3.55; hovered fill 2.58 to 2.80 |
| `ThemeToggle` icon button | 1 | **no**: identified by its icon and accessible name | reported |
| Segmented Appearance group and divider (`AppearanceCard:32,45`) | 2 | **no**: segments identified by their labels | reported |
| Chips (`BillsView:912`, `TransactionDetailDrawer:283`) | 2 | **no**: identified by their labels | reported |
| First-forecast step indicator (`FirstForecastWizard:395`) | 1 | **no**: not interactive | reported |
| Native checkboxes (`BillsView:986`, `CategoriesView:172`) | 2 | **no**: the platform draws them; the class sets only a color | reported |

**Disabled** controls are exempt (WCAG 1.4.11 exempts inactive components). **Invalid:** the source sets
`aria-invalid` in 16 places but defines **no** `aria-invalid:` border style (errors are carried by text), so
there is no separate invalid border to measure. **Open/selected:** the Combobox trigger's border does not change
when open; selected chips are exempt as above.

## Decisions applied

The owner decided on 2026-10-30 (Studio bead `personal-cfo-4d8.46`, quoted on `personal-cfo-4d8.35`): **A+** for the
input border, **B** for the badge, and the two exact status labels with a condition that the timestamp not make the
status look wonky.

**1. Outline `Button` border.** `button.tsx` outline is `border border-input`, so it follows the token: from 1.27 / 1.35
(Light) and 1.46 / 1.36 (Dark) to **3.33 / 3.52** (Light) and **3.55 / 3.31** (Dark). It is reported, not required (a
labelled button is identified by its text) and the owner accepted the shared impact. Pagination and every other
`border-input` consumer follow too.

**2. Options that were offered**, for the record (same hue family, only lightness moves):

| | Light `--input` | on `--background` / `--card` | Dark `--input` | on `--background` / `--card` |
|---|---|---|---|---|
| A | `#948c84` | 3.12 / 3.31 | `#5f6b63` | 3.35 / 3.11 |
| **A+ (chosen)** | `#90877f` | 3.33 / 3.52 | `#607065` | 3.55 / 3.31 |
| A++ | `#8c837b` | 3.51 / 3.72 | `#637569` | 3.80 / 3.53 |

**3. B: the text carries the status; the mark is a redundant cue.** Audit of every non-test `<Badge>` call site (15)
against that rule. The mark is generated content (`::before` with no text), so it adds nothing to the accessible name.

| Call site | Wording | Names the status in words? |
|---|---|---|
| `ConnectionsCard` warning | "Needs attention" | yes |
| `ConnectionsCard` outline | "Never refreshed" | yes |
| `ConnectionsCard` warning / gain | "Out of date · refreshed {stamp}" / "Refreshed {stamp}" | yes (was one wording under both variants) |
| `AccountDetailView` | "Due soon · in {n} days" (0 to 5 days) / "in {n} days" | yes (was one element with a conditional variant) |
| `FutureCashEntries` gain | "Matched" | yes |
| `CreditCardsView` gain x2 | "Actual" | yes |
| `CreditCardsView` warning | "Not yet closed — ignored" | yes |
| `BillsView` gain | "Autopay" | yes (a state, not a verdict) |
| `CardReviewModal` warning | "Resurfaced" (own icon) | yes |
| `AccountDetailView` outline / secondary, `CardReviewModal` tag, `ProviderPicker` chip | neutral tags | n/a (not semantic) |

A **source-scan guard** (`components/ui/badge-wording.test.ts`) fails if a semantic Badge picks its variant by a
condition on one element, uses the same words under two variants, or has no literal words; each rule was shown to trip by
mutating a real call site.

## Status fit: the owner's timestamp condition

The owner's condition: the stamp must not "drift too long where the button looks wonky". The status-fit page renders the
**real** `ConnectionRow` and `CardStatsStrip` with the real words, short, same-year, prior-year and long-name stamps, a
pending refresh and each due state, at a chosen component width, and reports clipped text, anything past the component,
overlapping controls and sideways scroll.

**Before**, at a 368px component: the connection's status lane could not shrink, so the identity collapsed to an
ellipsis ("clipped" on every row) and, for "Out of date · refreshed {stamp}", **Refresh now was pushed past the card's
edge** on all four stale rows. That is a real defect the approved longer label exposed.

**After** (this PR): the header wraps (`flex-wrap`), the identity wraps instead of truncating (`break-words`, `basis-40`),
the status lane drops below the identity when they do not fit side by side, inside the lane the badge may wrap its
complete label (`whitespace-normal`) while **Refresh now keeps its 36px size**, and the card strip's payment-due date and
badge wrap (`gap-x-3 gap-y-1`). Result: **13 of 13 rows fit at 368px and at 672px, in Light and Dark** (Chromium; also at a 300px stress width in Dark), and
at 330px in WebKit.

| File | What it shows |
|---|---|
| `status-fit-368-light.jpg`, `status-fit-368-dark.jpg` | 368px: an old prior-year stamp, a long connection name, a pending refresh, the card strip due soon and in 34 days |
| `status-fit-672-light.jpg`, `status-fit-672-dark.jpg` | 672px: the lane sits beside the identity; the same rows |
| `webkit-status-fit-330.jpg` | iOS WebKit at 330px |

Logic tests (`ConnectionsCard.test.tsx`, `AccountDetailView.test.tsx`): 72 hours minus 30 s reads "Refreshed", exactly 72 hours
reads "Out of date"; the year appears only for a prior-year stamp and the label stays whole across a year rollover; a null
or unparseable stamp reads "Never refreshed"; a last error or an attention item takes precedence over an old stamp; a
pending refresh keeps the action's name and shows the spinner; the due badge at 0, 1 (singular), 5, 6, far, past and with no
cycle. A source guard (`src/dev/status-fit-geometry.test.ts`) pins the wrapping classes, and was shown to fail when the old
non-shrinking lane is restored. Not covered: zoom beyond the 368px width (a 200% zoom of a 672px pane is about 336px,
inside the range measured), longer locales (the formatter uses the runtime locale; a longer string wraps by the same
rule), and a VoiceOver pass.

## Screenshots (fake data)

| | |
|---|---|
| `light-background.jpg`, `light-card.jpg` | Light, both surfaces: all badge variants, the long-label badges in a 220px column, every shared control, pagination, the segmented Appearance control |
| `dark-background.jpg`, `dark-card.jpg` | Dark, the same |
| `states-light.jpg`, `states-dark.jpg` | default / hover / focus tiles for the shared controls and the discovered consumers |
| `narrow-light.jpg`, `narrow-dark.jpg` | the same at a **375px** viewport: long labels wrap, nothing overflows |

All eight were retaken at option A+ on this head (Chromium 152). The status-fit screenshots are listed with their section above.
The two narrow shots also show the `NativeSelect` fix: the long option ("Statement balance carried o…") is clipped
clear of the chevron, with no text under the arrow, in both themes. The audit (PASS 316 · FAIL 0 · info 124) and all 13
status-fit rows were re-run on the head that merged `main` (including the shared disclosure, `personal-cfo-4d8.38`).

## What this evidence does not show

- The macOS app's own WKWebView (bounded in the Engine section), a longer-locale run, and a VoiceOver pass.
- **The marketing site.** `design-tokens.json`'s `input` also feeds the site's generated `tokens.css` (ADR 0061 section 7).
  Read-only check of the site checkout: `src/styles/tokens.css` defines `--input` in three places and **no site source
  references the token** (`src` and `public` hold only third-party CMS code that mentions "input" as a word), so a sync
  would change a variable nothing renders. This PR does not sync, and the Website Studio's own scope approval is still
  needed before any sync.
- Paper's baseline: the Studio updates it after the merge.
