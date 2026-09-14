# Personal CFO — building with this design system

A shadcn/ui-style React design system: Radix primitives + Tailwind v4 +
`class-variance-authority`, themed through CSS-variable tokens. Import components
from `window.PersonalCFO.*`. The visual language is calm and warm — a well-made
ledger, not a neon fintech dashboard: deep emerald primary (`#006341`), Saltillo
terracotta accent, warm-neutral grays, and dedicated finance colors so money
always reads correctly.

## Setup — no provider needed
Components are self-contained; there is **no theme/context provider to wrap**.
Render them directly. Two things come from the environment (both already shipped
in `styles.css`):
- **Font**: Inter Variable. Numbers use `tabular-nums` globally so currency
  columns align — always render money/quantities with tabular figures.
- **Dark mode** is a token swap via a `.dark` class on any ancestor
  (e.g. `<html class="dark">`). Every token remaps and the emerald brightens to
  `#12a06b` — don't restyle for dark, just toggle the class.

## Styling idiom — Tailwind utilities on design tokens
Style with **Tailwind utility classes**. Every component accepts `className`,
merged with its own via `tailwind-merge` (so your class wins conflicts). **Never
hard-code a hex** — use the token utility. Tokens (light + dark both defined):

| Purpose | Utilities |
|---|---|
| Surfaces | `bg-background` `bg-card` `bg-popover` `bg-muted` `bg-secondary` |
| Text | `text-foreground` `text-card-foreground` `text-muted-foreground` `text-primary` `text-secondary-foreground` |
| Brand / action | `bg-primary text-primary-foreground` · `bg-secondary` · `bg-destructive text-destructive-foreground` |
| Borders | `border` (theme border) · `border-input` |
| Money & status | `text-gain` (positive) · `text-loss` (negative) · `text-warning` · `text-info`; low-opacity tints like `bg-gain/10 border-gain/25` for badges |
| Charts | `var(--chart-1)` … `var(--chart-5)` — these are **full hex**; reference them as `var(--chart-N)`, never `hsl(var(--chart-N))` |

**Money color rule**: positive → `text-gain`, negative → `text-loss`, neutral →
`text-muted-foreground` or `text-foreground`. Never plain green/red.
**Shape**: cards `rounded-lg`, controls (button/input/select) `rounded-md`;
elevation is subtle (`shadow-sm`) — depth comes from surface + border, not heavy
shadows. **Spacing**: cards pad `p-6`; card headers stack `gap-1.5`; form fields
stack `flex flex-col gap-2`; section gaps `gap-4`/`gap-6`. Focus rings
(`ring-2 ring-ring`) are built into the interactive components — you don't add them.

> The shipped `styles.css` is the app's **compiled** Tailwind set: it carries the
> token utilities above plus the layout/typography utilities the app uses. Compose
> from the shipped components and these documented utilities; a rarely-used or
> arbitrary-value utility the app never emitted may not be in the stylesheet.

## Where the truth lives
- `guidelines/design-system.md` — full conventions: type scale, spacing, and every
  token with its light/dark hex.
- `guidelines/recipes.md` — golden copy-paste compositions (interactive Recharts
  chart, signed-money data table) with the token rules that are easy to get wrong.
- Per component: `<Name>.d.ts` (prop contract) + `<Name>.prompt.md` (usage).

## Idiomatic example
```tsx
const { Card, CardHeader, CardTitle, CardContent, Badge, Button } = window.PersonalCFO;

<Card style={{ width: 320 }}>
  <CardHeader>
    <CardTitle>Checking</CardTitle>
  </CardHeader>
  <CardContent className="flex items-center justify-between">
    <span className="text-2xl font-semibold tabular-nums">$5,300.00</span>
    <Badge variant="gain">+2.4%</Badge>
  </CardContent>
</Card>
```
