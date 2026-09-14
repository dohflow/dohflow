import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  Area,
  CartesianGrid,
  ComposedChart,
  Line,
  ReferenceDot,
  ReferenceLine,
  Text,
  XAxis,
  YAxis,
} from "recharts";

import type { ForecastDayDto, ForecastEventDto } from "@/bindings";
import { type ChartConfig, ChartContainer, ChartTooltip } from "@/components/ui/chart";
import {
  compactMoney,
  formatIsoDate,
  formatMoney,
  formatSignedMoney,
  shortIsoDate,
} from "@/lib/format";
import { type LowAnnotationPlacement, placeLowAnnotation, type TickPosition } from "./labelCollision";

const BILL_KINDS = new Set(["recurring_bill", "loan_payment"]);

/// Gap from the low marker's own edge to the "Low" label's near edge, and the
/// label's approximate rendered height (10px font) — both pixel-space inputs
/// to placeLowAnnotation (personal-cfo-4d8.29). Tuned against real screenshots,
/// not guessed: see that bead for the before/after crops.
const LOW_LABEL_GAP = 8;
const LOW_LABEL_HEIGHT = 12;

type Row = {
  date: string;
  p50: number;
  band: [number, number];
  events: ForecastEventDto[];
};

/// The aggregate Future Cash projection (personal-cfo-eqzs/tu2i), rebuilt on the
/// shadcn chart primitive for the MLP lovable pass (2pcx): the P50 balance line over
/// a brand gradient, the P10→P90 uncertainty ribbon (collapsed on Layer-1 data,
/// widening when the Layer-2 spend band activates), soft gain/loss dots on the days
/// money moves, the lowest projected point called out, and a hover card that lists
/// exactly which income and bills land that day — the chart answers "why" in place.
export function FutureCashChart({
  days,
  currency,
}: {
  days: ForecastDayDto[];
  currency: string;
}) {
  /// Collision-avoidance state for the "Low" annotation (personal-cfo-4d8.29).
  /// Two refs collect REAL rendered pixel positions as a side effect during
  /// this render (YAxis's custom `tick` records each tick's y; ReferenceDot's
  /// custom `label` records the marker's y) — read only after commit, in the
  /// layout effect below, never during the render that wrote them. Using
  /// Recharts' own already-computed coordinates (rather than re-deriving its
  /// internal scale/margin math ourselves) means this can't silently drift
  /// from what's actually on screen.
  ///
  /// Recharts' own <ResponsiveContainer> (inside ChartContainer) measures
  /// itself via its OWN internal ResizeObserver + state — invisible from out
  /// here, and crucially its resize-driven re-render does NOT bubble back up
  /// to re-run THIS component's function body or effects (it is a descendant
  /// re-rendering on its own local state, not an ancestor prop/state change).
  /// Without `resizeTick` below, this component would render exactly once,
  /// before the chart has any real size at all, and never get a second
  /// chance to notice the real tick/marker positions once they exist — a
  /// real bug caught by this bead's own component test, not assumed away.
  /// Observing the figure's own box directly (a real ResizeObserver, not a
  /// mock — this only runs in the app, never in tests) gives this component
  /// its OWN resize signal, so it re-renders whenever the chart's actual
  /// container size settles, same as production window resizing already
  /// requires it to.
  const tickPositionsRef = useRef<TickPosition[]>([]);
  const markerYRef = useRef<number | undefined>(undefined);
  // Set below (after the early return, once minVal is known) and read from
  // the layout effect — a ref because the effect must be declared here,
  // unconditionally, before that value exists yet on this render's first pass.
  const preferredSideRef = useRef<"above" | "below">("below");
  const [placement, setPlacement] = useState<LowAnnotationPlacement | undefined>(undefined);
  const [, setResizeTick] = useState(0);
  const figureRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const el = figureRef.current;
    if (!el) return;
    const observer = new ResizeObserver(() => {
      setResizeTick((t) => t + 1);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // No dependency array is deliberate: this must re-derive from the refs
  // EVERY render (the chart's data, and therefore every tick's and the
  // marker's real pixel position, can change on any render, including the
  // resize-driven ones the effect above now triggers) — not just once or
  // when some specific prop changes. It cannot loop forever: `setPlacement`
  // only actually updates state (triggering a re-render) when the computed
  // result differs from the previous one, and re-deriving the SAME already-
  // committed geometry always yields the same result, so this converges
  // after at most one corrective re-render per real data/layout change.
  // eslint-disable-next-line react-hooks/exhaustive-deps -- runs every render by design, see above
  useLayoutEffect(() => {
    const markerY = markerYRef.current;
    const ticks = tickPositionsRef.current;
    tickPositionsRef.current = []; // ready for the next render's collection
    if (markerY === undefined) return;
    const result = placeLowAnnotation(markerY, preferredSideRef.current, ticks, {
      gap: LOW_LABEL_GAP,
      labelHeight: LOW_LABEL_HEIGHT,
    });
    setPlacement((prev) =>
      prev?.y === result.y && prev?.hiddenTickValue === result.hiddenTickValue ? prev : result,
    );
  });

  if (days.length < 2) {
    return (
      <p className="py-10 text-center text-sm text-muted-foreground">
        Not enough data to chart yet.
      </p>
    );
  }

  const data: Row[] = days.map((d) => ({
    date: d.date,
    p50: d.closing.p50.minor_units,
    band: [d.closing.p10.minor_units, d.closing.p90.minor_units],
    events: d.events,
  }));

  const p50 = data.map((d) => d.p50);
  const minVal = Math.min(...p50);
  const minDate = data[p50.indexOf(minVal)]?.date;
  const crossesZero = minVal < 0 && Math.max(...data.flatMap((d) => d.band)) > 0;
  const firstBalance = p50[0] ?? 0;
  const lastBalance = p50[p50.length - 1] ?? 0;
  const preferredSide: "above" | "below" = minVal < 0 ? "above" : "below";
  preferredSideRef.current = preferredSide;

  const config: ChartConfig = {
    p50: { label: "Projected", color: "var(--chart-1)" },
  };

  /// Custom Y-axis tick: records this tick's real pixel position (a plain
  /// mutable-ref push, not a state write — safe during render, read only
  /// after commit) and renders it with Recharts' own `Text` primitive for
  /// pixel-identical output to the default tick — UNLESS it's the one tick
  /// placeLowAnnotation asked to suppress because the "Low" label displaced
  /// it and nothing else was free to move.
  function renderYAxisTick(props: Record<string, unknown>) {
    const { y, payload } = props as { y: number; payload: { value: number } };
    const value = Number(payload.value);
    tickPositionsRef.current.push({ value, y });
    if (placement?.hiddenTickValue === value) {
      return <g />;
    }
    // `tick` being a function (rather than the {fontSize} object the rest of
    // this file's axes still use) means Recharts' own filterProps(tick, ...)
    // returns null for it — fontSize would silently NOT reach here via
    // `{...props}` alone, so it's set explicitly. Fill color still comes
    // from ChartContainer's own CSS rule targeting
    // `.recharts-cartesian-axis-tick text` (a class Recharts applies to the
    // wrapping <g> regardless of what this function renders inside it), so
    // that one doesn't need restating here.
    return (
      <Text {...props} fontSize={11} className="recharts-cartesian-axis-tick-value">
        {compactMoney(value, currency)}
      </Text>
    );
  }

  /// Custom label for the low-value ReferenceDot: `viewBox` carries the dot's
  /// OWN already-computed pixel box ({x, y, width, height} = {cx-r, cy-r, 2r,
  /// 2r}), so the marker's true center is `viewBox.y + viewBox.height / 2`.
  /// Recorded into a ref for the layout effect above; drawn at
  /// `placement.y` once known, or at the same uncorrected default
  /// placeLowAnnotation itself would return with zero ticks (so first paint,
  /// before any tick position is known, still looks reasonable rather than
  /// sitting at y=0).
  function renderLowLabel(props: { viewBox?: { x: number; y: number; width: number; height: number } }) {
    const viewBox = props.viewBox;
    if (!viewBox) return <g />;
    const markerY = viewBox.y + viewBox.height / 2;
    markerYRef.current = markerY;
    const y =
      placement?.y ??
      placeLowAnnotation(markerY, preferredSide, [], {
        gap: LOW_LABEL_GAP,
        labelHeight: LOW_LABEL_HEIGHT,
      }).y;
    return (
      <text
        x={viewBox.x + viewBox.width / 2}
        y={y}
        textAnchor="middle"
        fontSize={10}
        fill={minVal < 0 ? "var(--loss)" : "var(--muted-foreground)"}
      >
        {`Low ${compactMoney(minVal, currency)}`}
      </text>
    );
  }

  return (
    <figure
      ref={figureRef}
      role="img"
      aria-label={`Projected liquid cash over the next ${days.length} days, from ${formatMoney(
        { minor_units: firstBalance, currency },
      )} to ${formatMoney({ minor_units: lastBalance, currency })}`}
    >
      <ChartContainer config={config} className="h-72">
        <ComposedChart data={data} margin={{ left: 8, right: 12, top: 8, bottom: 0 }}>
          <defs>
            <linearGradient id="futureCashFill" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="var(--chart-1)" stopOpacity={0.26} />
              <stop offset="55%" stopColor="var(--chart-1)" stopOpacity={0.07} />
              <stop offset="100%" stopColor="var(--chart-1)" stopOpacity={0} />
            </linearGradient>
          </defs>
          <CartesianGrid vertical={false} stroke="var(--border)" strokeOpacity={0.5} />
          {crossesZero && (
            <ReferenceLine
              y={0}
              stroke="var(--loss)"
              strokeDasharray="4 4"
              strokeOpacity={0.5}
            />
          )}
          <XAxis
            dataKey="date"
            tickLine={false}
            axisLine={false}
            tickMargin={8}
            minTickGap={48}
            tick={{ fontSize: 11 }}
            tickFormatter={(value) => shortIsoDate(String(value))}
          />
          <YAxis
            tickLine={false}
            axisLine={false}
            width={56}
            tick={renderYAxisTick}
          />
          <ChartTooltip
            cursor={{ stroke: "var(--muted-foreground)", strokeOpacity: 0.35 }}
            content={<DayTooltip currency={currency} />}
          />
          {/* P10→P90 uncertainty ribbon — invisible while collapsed on Layer-1 data. */}
          <Area
            dataKey="band"
            type="monotone"
            stroke="none"
            fill="var(--chart-1)"
            fillOpacity={0.1}
            isAnimationActive
            animationDuration={500}
          />
          {/* The gradient wash under the P50 line. */}
          <Area
            dataKey="p50"
            type="monotone"
            stroke="none"
            fill="url(#futureCashFill)"
            isAnimationActive
            animationDuration={500}
          />
          <Line
            dataKey="p50"
            type="monotone"
            stroke="var(--color-p50)"
            strokeWidth={2.25}
            dot={<EventDot />}
            activeDot={{ r: 5, strokeWidth: 2, stroke: "var(--card)" }}
            isAnimationActive
            animationDuration={500}
          />
          {/* The lowest projected point — the number the daily check-in cares about. */}
          {minDate !== undefined && (
            <ReferenceDot
              x={minDate}
              y={minVal}
              r={4}
              fill={minVal < 0 ? "var(--loss)" : "var(--chart-1)"}
              stroke="var(--card)"
              strokeWidth={2}
              label={renderLowLabel}
            />
          )}
        </ComposedChart>
      </ChartContainer>
    </figure>
  );
}

/// A soft dot only on days where money actually moves — green when the day nets
/// in, red when it nets out. Texture, not alarm: small and translucent.
function EventDot({
  cx,
  cy,
  payload,
}: {
  cx?: number;
  cy?: number;
  payload?: Row;
}) {
  if (cx === undefined || cy === undefined || !payload?.events.length) {
    return null;
  }
  const net = payload.events.reduce((sum, e) => sum + e.amount.minor_units, 0);
  const isOutflow =
    net < 0 || payload.events.every((e) => BILL_KINDS.has(e.kind));
  return (
    <circle
      cx={cx}
      cy={cy}
      r={2}
      fill={isOutflow ? "var(--loss)" : "var(--gain)"}
      fillOpacity={0.55}
    />
  );
}

/// The day hover card: closing balance plus the income/bills that land that day.
function DayTooltip({
  active,
  payload,
  label,
  currency,
}: {
  active?: boolean;
  payload?: { payload?: Row }[];
  label?: string;
  currency: string;
}) {
  const row = payload?.[0]?.payload;
  if (!active || !row) return null;
  return (
    <div className="min-w-44 rounded-lg border bg-popover px-3 py-2 text-xs shadow-md">
      <div className="mb-1 flex items-center justify-between gap-4 font-medium text-popover-foreground">
        <span>{label ? formatIsoDate(label) : ""}</span>
        <span className="tabular-nums">
          {formatMoney({ minor_units: row.p50, currency })}
        </span>
      </div>
      {row.events.length > 0 && (
        <div className="mt-1.5 flex flex-col gap-1 border-t pt-1.5">
          {row.events.slice(0, 5).map((event) => (
            <div
              key={event.source_event_id + event.name}
              className="flex items-center justify-between gap-3"
            >
              <span className="truncate text-muted-foreground">{event.name}</span>
              <span
                className={`tabular-nums ${
                  event.amount.minor_units < 0 ? "text-loss" : "text-gain"
                }`}
              >
                {formatSignedMoney(event.amount)}
              </span>
            </div>
          ))}
          {row.events.length > 5 && (
            <span className="text-muted-foreground">
              +{row.events.length - 5} more
            </span>
          )}
        </div>
      )}
    </div>
  );
}
