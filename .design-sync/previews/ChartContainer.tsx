import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
} from "@personal-cfo/desktop";
import { CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts";

// Full-hex chart tokens referenced directly (never hsl(...)) — the ChartContainer
// exposes each series color as --color-<key> for the <Line> to read.
const config = {
  checking: { label: "Checking", color: "var(--chart-1)" },
  savings: { label: "Savings", color: "var(--chart-2)" },
  credit: { label: "Credit", color: "var(--chart-3)" },
};

const data = [
  { month: "Jan", checking: 4200, savings: 12000, credit: -1800 },
  { month: "Feb", checking: 3850, savings: 12500, credit: -2100 },
  { month: "Mar", checking: 5100, savings: 13200, credit: -1500 },
  { month: "Apr", checking: 4600, savings: 13800, credit: -2400 },
  { month: "May", checking: 4950, savings: 14100, credit: -1900 },
  { month: "Jun", checking: 5300, savings: 15000, credit: -1600 },
];

export function BalancesOverTime() {
  return (
    <Card style={{ width: 560 }}>
      <CardHeader>
        <CardTitle>Balances over time</CardTitle>
        <CardDescription>Last 6 months, by account</CardDescription>
      </CardHeader>
      <CardContent>
        <ChartContainer config={config} className="h-72 w-full">
          <LineChart data={data} margin={{ left: 8, right: 8, top: 8 }}>
            <CartesianGrid vertical={false} stroke="var(--border)" />
            <XAxis
              dataKey="month"
              tickLine={false}
              axisLine={false}
              tickMargin={8}
              tick={{ fill: "var(--muted-foreground)", fontSize: 12 }}
            />
            <YAxis
              tickLine={false}
              axisLine={false}
              width={48}
              tick={{ fill: "var(--muted-foreground)", fontSize: 12 }}
              tickFormatter={(v) => `$${(v / 1000).toFixed(0)}k`}
            />
            <ChartTooltip cursor={false} content={<ChartTooltipContent />} />
            <Line
              dataKey="checking"
              type="monotone"
              isAnimationActive={false}
              stroke="var(--color-checking)"
              strokeWidth={2}
              dot={{ r: 3 }}
              activeDot={{ r: 5 }}
            />
            <Line
              dataKey="savings"
              type="monotone"
              isAnimationActive={false}
              stroke="var(--color-savings)"
              strokeWidth={2}
              dot={{ r: 3 }}
              activeDot={{ r: 5 }}
            />
            <Line
              dataKey="credit"
              type="monotone"
              isAnimationActive={false}
              stroke="var(--color-credit)"
              strokeWidth={2}
              dot={{ r: 3 }}
              activeDot={{ r: 5 }}
            />
          </LineChart>
        </ChartContainer>
      </CardContent>
    </Card>
  );
}
