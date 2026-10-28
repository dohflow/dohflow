# Evidence: `personal-cfo-4d8.35` (semantic badge text and control boundaries)

Fake data only. Produced by the committed audit page, `apps/desktop/src/dev/contrast-audit.tsx` (how to run
it: `apps/desktop/src/dev/README.md`). Source: `main` at `093dbb3` plus this PR.

## Engine

Measured in the Claude desktop app's built-in browser pane: **Chromium 152.0.7977.167** (macOS, Intel
emulation string `Macintosh; Intel Mac OS X 10_15_7`, device pixel ratio 2, app build `Claude/3.6556.2`).
This is **not the app's WKWebView**. `HEADROOM = 0.1` is a project buffer, not a certification of equivalence.
**Remaining validation, bounded:** open `contrast-audit.html` in Safari (macOS, WebKit) and in a packaged
build's webview if one is available, and compare the summary line and any row that differs by more than 0.1.
Nothing in this PR claims the WKWebView numbers.

## Result of the committed audit on this PR

`PASS 316 · FAIL 0 · info 124` (440 rows: 2 themes x 2 surfaces x every badge, shared control, state tile and
discovered consumer).

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

**Control boundary, resting** (`--input`, painted over the control's `--background` fill):

| | border | on outside | on fill |
|---|---|---|---|
| Light / `--background` | `#948c84` | 3.12 | 3.12 |
| Light / `--card` | `#948c84` | 3.31 | 3.12 |
| Dark / `--background` | `#5f6b63` | 3.35 | 3.35 |
| Dark / `--card` | `#5f6b63` | 3.11 | 3.35 |

Identical for Input, NativeSelect (both sizes), Combobox trigger, EmojiPicker trigger and the pagination
select. **Focus ring** (unchanged): `#2e9470` 3.55 / 3.76 Light, `#12a06b` 5.56 / 5.17 Dark.

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
*line* containing "test"; the Studio's count was right.

Classified (the audit discovers these from the source and reports each, three states, four surfaces):

| Class | Count | Boundary required? | Result |
|---|---|---|---|
| Hand-rolled text, textarea and color-swatch fields (`input.tsx`, `BillsView:62`, `SplitEditor:19,305`, `AccountEditorDrawer:486`, `AddCategoryForm:140`, `CategoriesView:432`) | 7 | yes | pass |
| Hand-rolled `<select>` class strings (18 matches) | 18 | yes | pass |
| Shared NativeSelect, Combobox, EmojiPicker trigger, pagination select | 4 | yes | pass (hover and focus included) |
| Custom checkbox box (`SeriesPicker:162`: an `aria-hidden` span whose border is the unchecked state) | 1 | **yes**: the box is the visible state of the control | pass |
| Outline `Button` (`button.tsx:16`) | 1 | **no**: identified by its text | reported: boundary 3.12 to 3.35; hovered fill 2.58 to 2.80 |
| `ThemeToggle` icon button | 1 | **no**: identified by its icon and accessible name | reported |
| Segmented Appearance group and divider (`AppearanceCard:32,45`) | 2 | **no**: segments identified by their labels | reported |
| Chips (`BillsView:912`, `TransactionDetailDrawer:283`) | 2 | **no**: identified by their labels | reported |
| First-forecast step indicator (`FirstForecastWizard:395`) | 1 | **no**: not interactive | reported |
| Native checkboxes (`BillsView:986`, `CategoriesView:172`) | 2 | **no**: the platform draws them; the class sets only a color | reported |

**Disabled** controls are exempt (WCAG 1.4.11 exempts inactive components). **Invalid:** the source sets
`aria-invalid` in 16 places but defines **no** `aria-invalid:` border style (errors are carried by text), so
there is no separate invalid border to measure. **Open/selected:** the Combobox trigger's border does not change
when open; selected chips are exempt as above.

## Screenshots (fake data)

| | |
|---|---|
| `light-background.jpg`, `light-card.jpg` | Light, both surfaces: all badge variants, the long-label badges in a 220px column, every shared control, pagination, the segmented Appearance control |
| `dark-background.jpg`, `dark-card.jpg` | Dark, the same |
| `states-light.jpg`, `states-dark.jpg` | default / hover / focus tiles for the shared controls and the discovered consumers |
| `narrow-light.jpg`, `narrow-dark.jpg` | the same at a **375px** viewport: long labels wrap, nothing overflows |

(In `light-background.jpg` the first Select shows its real hover border because the pointer was parked on it.)

## What this evidence does not show

- The app's own WKWebView (bounded above), and a VoiceOver pass (not asked for by this bead).
- The marketing site: `design-tokens.json`'s `input` also feeds the site's generated `tokens.css`; the site
  picks the new value up on its next sync (ADR 0061 section 7). That needs the Website Studio's scope check
  before the sync, and is not done here.
- Kit and gallery convergence with #190 (the kit docs, the gallery's badge rendering): a follow-up once #190
  merges.
