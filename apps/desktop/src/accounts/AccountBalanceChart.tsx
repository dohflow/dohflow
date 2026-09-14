/// The Account Detail hero chart (personal-cfo-4d8.27.5.7.4, ADR 0050): the account's
/// REALIZED balance as a solid line behind a "today" divider, then the FORWARD median as a
/// dashed continuation wrapped in the P10–P90 uncertainty ribbon that fans out with the
/// horizon. Built to the approved Claude Design mock (Account Detail.dc.html) on the shadcn
/// chart primitive (Recharts), matching FutureCashChart's idiom.

import {
  Area,
  CartesianGrid,
  ComposedChart,
  Line,
  ReferenceDot,
  ReferenceLine,
  XAxis,
  YAxis,
} from "recharts";

import {
  type ChartConfig,
  ChartContainer,
  ChartTooltip,
} from "@/components/ui/chart";
import { compactMoney, formatIsoDate, formatMoney } from "@/lib/format";

import type { DetailChartRow } from "./accountDetailSeries";

/// A dated marker pinned onto the forward path (a payment due, a statement close).
export type ChartMarker = { date: string; value: number; label: string };

function DetailTooltip({
  active,
  payload,
  currency,
  todayIso,
}: {
  active?: boolean;
  payload?: Array<{ payload: DetailChartRow }>;
  currency: string;
  todayIso: string;
}) {
  const row = payload?.[0]?.payload;
  if (!active || !row) return null;
  const realized = row.hist !== undefined && row.date <= todayIso;
  const value = realized ? row.hist : row.p50;
  if (value === undefined) return null;
  const showRange =
    !realized && row.band !== undefined && row.band[0] !== row.band[1];
  return (
    <div className="rounded-md border border-border bg-popover px-3 py-2 text-popover-foreground shadow-md">
      <p className="text-xs text-muted-foreground">
        {row.date === todayIso ? "Today · " : ""}
        {formatIsoDate(row.date)}
      </p>
      <p className="text-sm font-semibold tabular-nums">
        {formatMoney({ minor_units: value, currency })}
      </p>
      {showRange && row.band ? (
        <p className="mt-0.5 text-xs tabular-nums text-muted-foreground">
          Likely {formatMoney({ minor_units: row.band[0], currency })} –{" "}
          {formatMoney({ minor_units: row.band[1], currency })}
        </p>
      ) : (
        <p className="mt-0.5 text-xs text-muted-foreground">
          {realized ? (row.date === todayIso ? "Current balance" : "Realized") : "Projected"}
        </p>
      )}
    </div>
  );
}

/// Renders the merged history + projection rows. `figureLabel` names the plotted figure
/// for assistive tech ("Balance" / "Amount owed").
export function AccountBalanceChart({
  rows,
  currency,
  todayIso,
  markers = [],
  figureLabel,
}: {
  rows: DetailChartRow[];
  currency: string;
  todayIso: string;
  markers?: ChartMarker[];
  figureLabel: string;
}) {
  if (rows.length < 2) {
    return (
      <p className="py-10 text-center text-sm text-muted-foreground">
        No balance history to chart for this account yet — it grows as you record
        activity.
      </p>
    );
  }

  const config: ChartConfig = {
    hist: { label: "Realized", color: "var(--chart-1)" },
    p50: { label: "Median (P50)", color: "var(--chart-1)" },
  };
  const first = rows[0]?.date ?? "";
  const last = rows[rows.length - 1]?.date ?? "";
  const crossesZero = rows.some(
    (r) => (r.hist ?? 0) < 0 || (r.band?.[0] ?? 0) < 0,
  );

  return (
    <figure
      role="img"
      aria-label={`${figureLabel} from ${formatIsoDate(first)} to ${formatIsoDate(
        last,
      )}: realized history, then the projected median with its likely range.`}
    >
      <ChartContainer config={config} className="h-80">
        <ComposedChart data={rows} margin={{ left: 8, right: 16, top: 12, bottom: 0 }}>
          <defs>
            <linearGradient id="acctHistFill" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="var(--chart-1)" stopOpacity={0.2} />
              <stop offset="100%" stopColor="var(--chart-1)" stopOpacity={0} />
            </linearGradient>
          </defs>
          <CartesianGrid vertical={false} stroke="var(--border)" strokeOpacity={0.5} />
          {crossesZero && (
            <ReferenceLine y={0} stroke="var(--loss)" strokeDasharray="4 4" strokeOpacity={0.5} />
          )}
          <XAxis
            dataKey="date"
            tickLine={false}
            axisLine={false}
            tickMargin={8}
            minTickGap={56}
            tick={{ fontSize: 11 }}
            tickFormatter={(value) => formatIsoDate(String(value))}
          />
          <YAxis
            tickLine={false}
            axisLine={false}
            width={56}
            tick={{ fontSize: 11 }}
            tickFormatter={(value) => compactMoney(Number(value), currency)}
          />
          <ChartTooltip
            cursor={{ stroke: "var(--muted-foreground)", strokeOpacity: 0.35 }}
            content={<DetailTooltip currency={currency} todayIso={todayIso} />}
          />
          {/* The P10–P90 ribbon around the projected median (fans out with the horizon). */}
          <Area
            dataKey="band"
            type="monotone"
            stroke="var(--chart-1)"
            strokeOpacity={0.25}
            strokeWidth={1}
            fill="var(--chart-1)"
            fillOpacity={0.12}
            connectNulls
            isAnimationActive={false}
          />
          {/* Soft wash under the realized line. */}
          <Area
            dataKey="hist"
            type="monotone"
            stroke="none"
            fill="url(#acctHistFill)"
            connectNulls
            isAnimationActive={false}
          />
          {/* Realized history: the bold solid line. */}
          <Line
            dataKey="hist"
            type="monotone"
            stroke="var(--chart-1)"
            strokeWidth={2.5}
            dot={false}
            connectNulls
            isAnimationActive={false}
          />
          {/* Projected median: the dashed continuation. */}
          <Line
            dataKey="p50"
            type="monotone"
            stroke="var(--chart-1)"
            strokeWidth={2}
            strokeDasharray="6 5"
            strokeOpacity={0.9}
            dot={false}
            connectNulls
            isAnimationActive={false}
          />
          {/* The today divider between realized and projected. */}
          <ReferenceLine
            x={todayIso}
            stroke="var(--muted-foreground)"
            strokeDasharray="3 3"
            strokeOpacity={0.55}
            label={{
              value: "TODAY",
              position: "top",
              fontSize: 10,
              fontWeight: 700,
              fill: "var(--muted-foreground)",
            }}
          />
          {/* Upcoming-event markers (payment due, statement close). */}
          {markers.map((m) => (
            <ReferenceDot
              key={`${m.date}-${m.label}`}
              x={m.date}
              y={m.value}
              r={4}
              fill="var(--terracotta)"
              stroke="var(--card)"
              strokeWidth={1.5}
              label={{
                value: m.label,
                position: "top",
                fontSize: 10,
                fontWeight: 600,
                fill: "var(--terracotta)",
              }}
            />
          ))}
        </ComposedChart>
      </ChartContainer>
      <figcaption className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span className="inline-flex items-center gap-1.5">
          <span aria-hidden className="h-0 w-4 border-t-2 border-[var(--chart-1)]" />
          Realized
        </span>
        <span className="inline-flex items-center gap-1.5">
          <span aria-hidden className="h-0 w-4 border-t-2 border-dashed border-[var(--chart-1)]" />
          Median (P50)
        </span>
        <span className="inline-flex items-center gap-1.5">
          <span
            aria-hidden
            className="h-2.5 w-4 rounded-sm bg-[var(--chart-1)] opacity-20"
          />
          Likely range (P10–P90)
        </span>
        {markers.length > 0 && (
          <span className="inline-flex items-center gap-1.5">
            <span
              aria-hidden
              className="size-2 rotate-45 rounded-[2px] bg-[var(--terracotta)]"
            />
            Upcoming
          </span>
        )}
      </figcaption>
    </figure>
  );
}
