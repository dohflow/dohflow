# Component recipes (golden snippets)

Copy-pasteable references for the components whose **shadcn API + DohFlow
tokens** are easy to get wrong. These are the canonical shapes — Claude Design (and
we) should match them rather than re-deriving. For everything simpler (button, card,
badge), the conventions in [`design-system.md`](./design-system.md) §5 + the
[gallery](./component-gallery.html) are enough.

> **Token rule that matters most:** our chart colors in
> [`globals.css`](../../apps/desktop/src/styles/globals.css) are **full hex**
> (`--chart-1: #006341`), not HSL channel triplets. So in `ChartConfig` use
> `color: "var(--chart-1)"` — **never** shadcn's default `hsl(var(--chart-1))`,
> which would break our palette.

> **Repo status:** only `button`, `card`, `input`, `label` exist in
> `apps/desktop/src/components/ui/` today. The `chart`, `dialog`, `table`, and
> `form` primitives below are standard shadcn components we adopt when first needed
> (`npx shadcn@latest add chart dialog table form`). These recipes are how they
> should look in our system. Forms target **RHF + Zod** per ADR 0020 / bead
> `personal-cfo-o9cz`; until that lands, new forms match the existing hand-rolled
> `useState` pattern (see `BillsView.tsx`).

---

## 1. Interactive multi-line chart with dots (shadcn + Recharts)

shadcn charts wrap **Recharts**. `ChartContainer` reads `ChartConfig` and exposes
each series color as `--color-<key>`; the `<Line>` references that var.

```tsx
import { CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts";
import {
  Card, CardContent, CardDescription, CardHeader, CardTitle,
} from "@/components/ui/card";
import {
  type ChartConfig,
  ChartContainer,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart";

// Series → label + color. Use the hex token var directly (NOT hsl(...)).
const chartConfig = {
  checking: { label: "Checking", color: "var(--chart-1)" },
  savings:  { label: "Savings",  color: "var(--chart-2)" },
  credit:   { label: "Credit",   color: "var(--chart-3)" },
} satisfies ChartConfig;

const data = [
  { month: "Jan", checking: 4200, savings: 12000, credit: -1800 },
  { month: "Feb", checking: 3850, savings: 12500, credit: -2100 },
  { month: "Mar", checking: 5100, savings: 13200, credit: -1500 },
  { month: "Apr", checking: 4600, savings: 13800, credit: -2400 },
  { month: "May", checking: 4950, savings: 14100, credit: -1900 },
  { month: "Jun", checking: 5300, savings: 15000, credit: -1600 },
];

export function BalancesChart() {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Balances over time</CardTitle>
        <CardDescription>Last 6 months</CardDescription>
      </CardHeader>
      <CardContent>
        <ChartContainer config={chartConfig} className="h-[260px] w-full">
          <LineChart data={data} margin={{ left: 8, right: 8, top: 8 }}>
            <CartesianGrid vertical={false} stroke="var(--border)" />
            <XAxis
              dataKey="month" tickLine={false} axisLine={false} tickMargin={8}
              tick={{ fill: "var(--muted-foreground)", fontSize: 12 }}
            />
            <YAxis
              tickLine={false} axisLine={false} width={48}
              tick={{ fill: "var(--muted-foreground)", fontSize: 12 }}
              // money axis: keep numerals tabular + compact
              tickFormatter={(v) => `$${(v / 1000).toFixed(0)}k`}
            />
            <ChartTooltip cursor={false} content={<ChartTooltipContent />} />
            <ChartLegend content={<ChartLegendContent />} />
            {/* one <Line> per series; dots on by default, larger on hover */}
            <Line dataKey="checking" type="monotone" stroke="var(--color-checking)"
              strokeWidth={2} dot={{ r: 3 }} activeDot={{ r: 5 }} />
            <Line dataKey="savings" type="monotone" stroke="var(--color-savings)"
              strokeWidth={2} dot={{ r: 3 }} activeDot={{ r: 5 }} />
            <Line dataKey="credit" type="monotone" stroke="var(--color-credit)"
              strokeWidth={2} dot={{ r: 3 }} activeDot={{ r: 5 }} />
          </LineChart>
        </ChartContainer>
      </CardContent>
    </Card>
  );
}
```

Notes:
- **Interactive** = `ChartTooltip` (hover) + `ChartLegend` + `activeDot`. Set
  `dot={false}` for a clean line; `dot={{ r: 3 }}` for visible points.
- Axis/grid colors come from **tokens** (`--muted-foreground`, `--border`), so the
  chart re-themes for dark mode automatically.
- More series → add to `chartConfig` and one `<Line>` each, taking the **next unused**
  slot in order (`--chart-3`, then `--chart-4`). There are **four** slots and they are
  **never cycled** — [ADR 0054](../adr/0054-categorical-chart-palette.md). A fifth series
  folds into an explicit "Other", facets into small multiples, or is capped with the
  surface saying so; `i % palette.length` paints two different series the same colour and
  shows the same swatch twice in the legend. (This line used to say "cycling", which is
  where two shipped charts learned the habit.)

---

## 2. Data table (signed money, right-aligned, tabular)

```tsx
import {
  Table, TableBody, TableCell, TableHead, TableHeader, TableRow,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { formatSignedMoney, signedAmountClass } from "@/lib/format";
import { cn } from "@/lib/utils";

export function TransactionsTable({ rows }: { rows: Txn[] }) {
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Date</TableHead>
          <TableHead>Description</TableHead>
          <TableHead>Category</TableHead>
          <TableHead className="text-right">Amount</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.map((t) => (
          <TableRow key={t.id}>
            <TableCell className="tabular-nums text-muted-foreground">{t.date}</TableCell>
            <TableCell className="font-medium">{t.description}</TableCell>
            <TableCell><Badge variant="secondary">{t.category}</Badge></TableCell>
            {/* signedAmountClass → text-gain / text-loss; always tabular-nums */}
            <TableCell className={cn("text-right tabular-nums", signedAmountClass(t.amount))}>
              {formatSignedMoney(t.amount)}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}
```

Rules: money columns are **right-aligned + `tabular-nums`**; color via
`signedAmountClass` (never hard-coded green/red); header text is muted/uppercase-ish.

---

## 3. Dialog / confirmation modal (destructive)

```tsx
import {
  Dialog, DialogContent, DialogDescription, DialogFooter,
  DialogHeader, DialogTitle, DialogTrigger,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

export function DeleteAccountDialog({ name, count, onConfirm }: {
  name: string; count: number; onConfirm: () => void;
}) {
  return (
    <Dialog>
      <DialogTrigger asChild>
        <Button variant="ghost">Delete account</Button>
      </DialogTrigger>
      <DialogContent className="sm:max-w-[440px]">
        <DialogHeader>
          <DialogTitle>Delete this account?</DialogTitle>
          <DialogDescription>
            This removes “{name}” and its {count.toLocaleString()} transactions.
            This action cannot be undone.
          </DialogDescription>
        </DialogHeader>
        {/* tinted loss callout reinforces irreversibility */}
        <div className="flex gap-3 rounded-md border border-loss/30 bg-loss/10 p-3 text-sm">
          <span className="font-semibold text-loss">Irreversible</span>
          <span className="text-muted-foreground">Export a backup first if unsure.</span>
        </div>
        <DialogFooter>
          <Button variant="outline">Cancel</Button>
          <Button variant="destructive" onClick={onConfirm}>Delete account</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
```

`DialogContent` is a `popover`-surface card on a dimmed `foreground/50` overlay;
footer actions right-aligned, destructive action last.

---

## 4. Form (React Hook Form + Zod — ADR 0020 target)

```tsx
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import {
  Form, FormControl, FormField, FormItem, FormLabel, FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { dollarsToMinorUnits } from "@/lib/format";

// One Zod schema per form. Rust re-validates and is authoritative; this is UX only.
const billSchema = z.object({
  name: z.string().min(1, "Required"),
  amount: z.string().regex(/^\$?\d+(\.\d{2})?$/, "Enter a dollar amount"),
  dueDate: z.string().regex(/^\d{4}-\d{2}-\d{2}$/, "YYYY-MM-DD"),
});
type BillForm = z.infer<typeof billSchema>;

export function AddBillForm({ onSubmit }: { onSubmit: (v: AddBillInput) => void }) {
  const form = useForm<BillForm>({
    resolver: zodResolver(billSchema),
    defaultValues: { name: "", amount: "", dueDate: "" },
  });

  return (
    <Form {...form}>
      <form
        className="flex flex-col gap-4"
        onSubmit={form.handleSubmit((v) =>
          onSubmit({ ...v, amount: dollarsToMinorUnits(v.amount) }))}
      >
        <FormField name="name" control={form.control} render={({ field }) => (
          <FormItem>
            <FormLabel>Bill name</FormLabel>
            <FormControl><Input placeholder="e.g. Internet" {...field} /></FormControl>
            <FormMessage />
          </FormItem>
        )} />
        <FormField name="amount" control={form.control} render={({ field }) => (
          <FormItem>
            <FormLabel>Amount</FormLabel>
            <FormControl><Input className="tabular-nums" placeholder="$0.00" {...field} /></FormControl>
            <FormMessage />
          </FormItem>
        )} />
        <FormField name="dueDate" control={form.control} render={({ field }) => (
          <FormItem>
            <FormLabel>Due date</FormLabel>
            <FormControl><Input className="tabular-nums" placeholder="2026-07-01" {...field} /></FormControl>
            <FormMessage />
          </FormItem>
        )} />
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost">Cancel</Button>
          <Button type="submit">Add bill</Button>
        </div>
      </form>
    </Form>
  );
}
```

Rules: money parsed with `dollarsToMinorUnits` (integer minor units cross the wire,
never a float — ADR 0021); `FormMessage` shows the Zod error; `FormLabel` +
`FormControl` keep label/aria wiring correct.
