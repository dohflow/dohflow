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

## For the Studio's decision

**1. Every outline `Button` border darkens too.** `button.tsx` outline is `border border-input`, so it
follows the token. Rendered, its border goes from 1.27 / 1.35:1 (Light) and 1.46 / 1.36:1 (Dark) to
**3.12 / 3.31** (Light) and **3.35 / 3.11** (Dark). It is **reported, not required**: a labelled button is
identified by its text. It is visible in every screenshot below (the `outline` button beside `secondary`). The
alternative is to give the outline `Button` its own lighter border token, which keeps today's quiet buttons and
adds one more token; this PR does not.

**2. The Dark-card boundary has almost no spare headroom.** With the 0.1 rule the requirement is 3.1:1, and
`#5f6b63` on `--card` measures **3.11** (Light `#948c84` on `--background` is 3.12), so 0.01 to 0.02 to spare.
The borders are opaque, so these static ratios equal the rendered ones. **Offered alternatives**, same hue
family, only lightness moves:

| | Light `--input` | on `--background` / `--card` | Dark `--input` | on `--background` / `--card` |
|---|---|---|---|---|
| **A (this PR)** | `#948c84` | 3.12 / 3.31 | `#5f6b63` | 3.35 / **3.11** |
| **A+ (3.3 target)** | `#90877f` | 3.33 / 3.52 | `#607065` | 3.55 / 3.31 |
| A++ (3.5 target) | `#8c837b` | 3.51 / 3.72 | `#637569` | 3.80 / 3.53 |

The PR stays on A until the Studio chooses; switching is a two-token edit in `globals.css` and
`design-tokens.json`, with the contrast tests unchanged.

**3. B: the text must carry the status, and the mark is only a redundant cue.** Audit of every non-test
`<Badge>` call site (15) against that rule. The mark is generated content (`::before` with no text), so it adds
nothing to the accessible name; the label's words are the signal.

| Call site | Wording | Names the status in words? |
|---|---|---|
| `ConnectionsCard` warning | "Needs attention" | yes |
| `ConnectionsCard` outline | "Never refreshed" | yes |
| `ConnectionsCard` warning / gain | "Refreshed {stamp}" under **both** variants | **no**: stale (3 days or more) and fresh differ only by tint and dot |
| `AccountDetailView:583` | "in {n} days" under `warning` (5 days or fewer) or `secondary` | **no**: urgency is carried by the variant alone |
| `FutureCashEntries` gain | "Matched" | yes |
| `CreditCardsView` gain x2 | "Actual" | yes |
| `CreditCardsView` warning | "Not yet closed — ignored" | yes |
| `BillsView` gain | "Autopay" | yes (a state, not a verdict) |
| `CardReviewModal` warning | "Resurfaced" (own icon) | yes |
| `AccountDetailView` outline / secondary, `CardReviewModal` tag, `ProviderPicker` chip | neutral tags | n/a (not semantic) |

Two sites rest on color alone. They need **new wording**, which is a copy decision, so it has been sent to
planning with proposals ("Stale · refreshed {stamp}" and "Due soon · in {n} days") and is **not** in this PR
until approved. When it is, the PR adds a source-scan test that fails if any semantic Badge uses the same words
under two variants or picks its variant by a condition on one element.

**4. Counts.** 04b-review counts **40 uses in 30 files** at this PR's head; that includes the one comment in
`globals.css` (and my dev files' own mentions are excluded from the audit). At `093dbb3` it is **39 in 29**.

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
