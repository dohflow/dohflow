# design-sync notes — personal-cfo desktop design system

Repo-specific gotchas for future syncs. Read this before re-syncing.

## Shape & why it needs a barrel + compiled CSS
- The design system is a shadcn/ui-style primitive set living **inside the Vite
  app** at `apps/desktop/src/components/ui/*.tsx` (+ `src/components/PageHeader.tsx`).
  There is **no standalone library package, no `dist/`, and no shipped `.d.ts`**.
- `cfg.buildCmd` = `node .design-sync/build.mjs`, which produces the two artifacts
  the converter can't derive itself:
  1. `apps/desktop/.design-sync-entry.tsx` — a generated barrel that `export *`s
     every DS component file so esbuild lands all exports (12 primaries **plus**
     the Card*/Table*/Chart* subcomponents — 28 total) on `window.PersonalCFO`.
     Passed via `--entry`.
  2. `apps/desktop/.design-sync-dist/ds.css` — the app's REAL compiled Tailwind v4
     stylesheet, cherry-picked out of a `vite build --base ./` (tokens + every
     utility the components use + the Inter `@font-face` rules), with its woff2
     siblings copied alongside so the relative `url(./…)` refs still resolve.
     Passed via `cfg.cssEntry`.
  Both are gitignored and regenerated every sync.
- `--entry` must point at a file **inside `apps/desktop`** so the converter's
  package.json walk-up resolves `PKG_DIR = apps/desktop` (correct name/version,
  correct `@/` alias base). Do NOT drop `--entry` — without it the tool tries
  `node_modules/@personal-cfo/desktop` (a workspace pkg, not self-installed) and
  everything breaks.

## Contracts (`dtsPropsFor`)
- No shipped `.d.ts` → the converter emits empty `{ [key: string]: unknown }`
  stubs. `cfg.dtsPropsFor` hand-writes the real prop bodies for all 12. DOM-spread
  components (Badge, Button, Card, Input, Label, NativeSelect, Skeleton, Table)
  carry named DS props + a permissive `[key: string]: unknown` index signature
  (they genuinely forward every native attribute). The four pure function
  components (ChartContainer, EmptyState, PaginationControls, PageHeader) have
  precise props (no index signature). Keep these in sync if a component's API
  changes upstream.

## Grouping
- All 12 land in group `general` — the src dirs (`components`, `ui`) are both in
  the converter's GENERIC_DIR set and there are no per-component doc frontmatter
  categories, so nothing else supplies a group. Acceptable for a 12-component DS.

## Toolchain
- Node 25, pnpm 11.5.2. Pass `COREPACK_ENABLE_STRICT=0` to any pnpm invocation
  (build.mjs already sets it) so corepack doesn't try to self-provision.
- Converter deps live in `.ds-sync/` (esbuild, ts-morph, @types/react, playwright,
  typescript). Chromium for the render check installs to
  `~/Library/Caches/ms-playwright/` (macOS path, NOT `~/.cache`).
- `--node-modules apps/desktop/node_modules` (where the app's react resolves).

## Guidelines
- `docs/design-system/{design-system,recipes,README}.md` ship into `guidelines/`
  via `cfg.guidelinesGlob`. `recipes.md` has golden Chart/Table snippets and the
  critical token rule: chart colors are **full hex** — use `color: "var(--chart-N)"`,
  never `hsl(var(--chart-N))`.

## Preview-authoring learnings (from the first sync)
- **Import spec**: previews import DS components from `"@personal-cfo/desktop"`
  (redirected to `window.PersonalCFO`). `lucide-react` and `recharts` import
  normally (bundled from node_modules, sharing the vendored React).
- **Tailwind JIT gotcha**: the shipped CSS only has utilities the app already
  used. Preview layout must stick to confirmed-present classes; for fixed
  widths/heights (`w-80`, `h-4`) and off-palette colors (`text-loss`) use inline
  `style` — e.g. signed money is colored with `style={{ color: "var(--gain)" }}` /
  `"var(--loss)"`, the standard pattern in these previews.
- **Recharts static capture**: set `isAnimationActive={false}` on every `<Line>`
  (and Bar/Area) or the screenshot catches the line-draw animation mid-flight
  (lines only reach ~halfway). See `previews/ChartContainer.tsx`.
- **cardMode**: wide/multi-field previews trip `[GRID_OVERFLOW]`. `cfg.overrides`
  sets `cardMode: "column"` for the wide ones (EmptyState, Input, Label,
  NativeSelect, PageHeader, PaginationControls, Skeleton, Table) and
  `cardMode: "single"` + a viewport for the chart. Badge/Button/Card stay grid.

## Known render warns
- None outstanding. The 7 `[GRID_OVERFLOW]` warns from the first full build were
  all resolved by the `cardMode: "column"` overrides above; a clean driver run
  shows `gridOverflow 0`. A NEW grid-overflow warn on re-sync means a preview grew
  wider content — widen the override, don't ignore it.

## Re-sync risks (what can silently go stale)
- **Compiled-CSS vocabulary**: `cssEntry` is the app's *compiled* Tailwind set, so
  the design agent building NEW screens is limited to utilities the app already
  emitted (the full token palette + common layout utilities are present; exotic /
  arbitrary-value utilities may not be). If designs come out under-styled on a
  class, broaden the utility set (e.g. a Tailwind safelist in the build) — filed as
  a possible enhancement, not done in the first sync.
- **Inlined preview data**: `previews/ChartContainer.tsx` and `Table.tsx` hardcode
  sample finance data/rows. That's fine (it's composition, not app data) but it
  won't track real app changes — re-check the recipes if the Chart/Table API moves.
- **`dtsPropsFor` is hand-written**: if a component's real props change upstream
  (new variant, renamed prop), the emitted `.d.ts` won't follow automatically —
  update `cfg.dtsPropsFor` to match the source.
- **Toolchain assumptions**: build.mjs runs the app's `vite build` (Node 25,
  pnpm 11.5.2) and cherry-picks `dist/assets/*.css`. If the app's build output
  layout changes (e.g. CSS no longer the largest `.css`, or fonts no longer
  emitted as sibling woff2), fix `.design-sync/build.mjs` accordingly.
- **Group is `general`** for all 12 — a future add lands there too; revisit if the
  DS grows enough to want real grouping (docsMap stubs with `category` frontmatter).
