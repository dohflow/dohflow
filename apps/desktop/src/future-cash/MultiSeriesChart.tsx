import { type ComponentProps, useState } from "react";
import {
  Area,
  CartesianGrid,
  ComposedChart,
  Line,
  ReferenceArea,
  ReferenceLine,
  XAxis,
  YAxis,
} from "recharts";

import type {
  AccountHistoryDto,
  AccountSeriesDto,
  GroupSeriesDto,
} from "@/bindings";
import {
  type ChartConfig,
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart";
import { compactMoney, formatIsoDate, shortIsoDate } from "@/lib/format";

import { assembleMultiSeriesRows, historyTiersWithData } from "./multiSeriesRows";
import {
  BAND_EDGE_DASH,
  BAND_EDGE_OPACITY,
  BAND_FILL_OPACITY,
} from "./bandLegibility";
import { accountLabel, accountSeriesKey, seriesAsLabelled } from "./seriesKeys";

/// Per-tier styling. Distinct brand colours so each tier reads apart (globals.css
/// full-hex `--chart-*` tokens).
const TIER_META: Record<string, { label: string; color: string }> = {
  spendable: { label: "Spendable", color: "var(--chart-3)" },
  reserve: { label: "Reserve", color: "var(--chart-2)" },
  net: { label: "Net cash", color: "var(--chart-1)" },
  unallocated: { label: "Unallocated", color: "var(--chart-4)" },
};

/// Prominence order (personal-cfo-4d8.25.25): the owner reads the chart to see whether
/// **Spendable** dips negative, so it is the primary (filled, bold) series, Reserve
/// second, Net third. Legend + draw order follow this.
const TIER_ORDER = ["spendable", "reserve", "net", "unallocated"] as const;

/// Per-tier line weight for the prominence ramp.
const TIER_STROKE_WIDTH: Record<string, number> = {
  spendable: 2.5,
  reserve: 2,
  net: 1.5,
  unallocated: 1.5,
};

/// Rotating colours for individual-account lines. At plot time the entries that match
/// a currently-drawn tier's colour are filtered out (see `usablePalette`), so an
/// account line never shares a co-plotted aggregate's hue.
const ACCOUNT_PALETTE = [
  "var(--chart-3)",
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-4)",
];

/// The per-group Future Cash chart (personal-cfo-l916) on the shadcn chart primitive
/// (Recharts), polished for the MLP lovable pass (2pcx): a soft brand **gradient**
/// under the bold net series, labelled comfort-band edges, a zero line whenever the
/// projection dips negative, and a gentle draw-in animation. Tiers that stay flat at
/// zero for the whole horizon are dropped — they'd only add noise. The custom legend
/// + the figure's `aria-label` keep it accessible.
/// The comfort band (ADR 0018 addendum 915.1, personal-cfo-3v6d), in minor units, to shade
/// behind the series. `upper` is `null` when the user hasn't set an upper edge.
export type ChartBand = { lower: number; upper: number | null };

export function MultiSeriesChart({
  groups,
  currency,
  band,
  accounts,
  selection,
  history,
  todayIso,
}: {
  groups: GroupSeriesDto[];
  currency: string;
  band?: ChartBand | null;
  /// Per-account series (personal-cfo-4d8.25.26); only plotted when `selection`
  /// picks them.
  accounts?: AccountSeriesDto[];
  /// The series to plot, as keys (`net`/`spendable`/`reserve`/`unallocated` or
  /// `acct:<id>`). When omitted, every present aggregate tier is plotted (the
  /// pre-picker behaviour), so callers/tests that don't select still work.
  selection?: string[];
  /// Realized per-account history (personal-cfo-4d8.27.5.3). When present, the
  /// chart draws the realized past as SOLID lines up to the TODAY divider and the
  /// projection as DASHED lines beyond it (the ADR 0050 visual language).
  history?: AccountHistoryDto[] | null;
  /// The forecast's first day (household-local today) — the divider between halves.
  todayIso?: string;
}) {
  // The net group is the date spine + data guard even when it isn't plotted.
  const spine = groups.find((g) => g.tier === "net");
  const tierPicked = (tier: string) =>
    selection === undefined || selection.includes(tier);
  // Aggregate tiers to plot, in prominence order. A tier that never leaves zero
  // carries no information — drop it, UNLESS its realized history is nonzero (a
  // just-drained reserve still earns its past). Net is only plotted when picked.
  const histTiers = historyTiersWithData(history);
  const tiers = TIER_ORDER.filter(
    (tier) =>
      tierPicked(tier) &&
      (groups.some(
        (g) =>
          g.tier === tier &&
          g.closings.some((c) => c.closing.p50.minor_units !== 0),
      ) ||
        histTiers.has(tier)),
  );
  // Account colours avoid any currently-plotted TIER colour, so a drilled account
  // line is never the same hue as a co-plotted aggregate (the coherence rule keeps
  // an account off its OWN tier, but sibling tiers stay plotted — adversarial review
  // of 4d8.25.26).
  const activeTierColors = new Set(tiers.map((t) => TIER_META[t]?.color));
  // No fallback to the unfiltered list when this empties out: falling back handed an
  // account line the exact colour of a tier it is plotted against, which is the same
  // "two series, one colour" failure as cycling, just with a different pair
  // (personal-cfo-hnba).
  const usablePalette = ACCOUNT_PALETTE.filter((c) => !activeTierColors.has(c));
  const pickedAccounts = (accounts ?? []).filter(
    (a) =>
      a.account_id !== null &&
      selection !== undefined &&
      selection.includes(accountSeriesKey(a.account_id)),
  );
  // ADR 0054: slots are assigned IN ORDER and never cycled. This used to read
  // `usablePalette[i % usablePalette.length]`, so once the picked accounts outnumbered
  // the free colours, two different accounts were drawn as the same line and the legend
  // showed the same swatch twice — on a chart the household reads to see where its money
  // is going. The surplus is left unplotted and named below instead.
  const colourableAccounts = pickedAccounts.slice(0, usablePalette.length);
  const unplottedAccounts = pickedAccounts.slice(usablePalette.length);
  // Individual-account series — sanitized, STABLE keys (from the account id, not the
  // filtered index) so the legend-hide Set + chart config keep targeting the same
  // account when the selection changes.
  const plottedAccounts = colourableAccounts.map((a, i) => ({
    key: `acct_${(a.account_id ?? "").replace(/[^a-zA-Z0-9]/g, "")}`,
    account_id: a.account_id ?? "",
    label: accountLabel(seriesAsLabelled(a), pickedAccounts.map(seriesAsLabelled)),
    color: usablePalette[i]!,
    days: a.days,
  }));

  const dayCount = spine?.closings.length ?? 0;
  // Legend toggles a series' visibility (personal-cfo-4d8.25.25): the owner can
  // isolate Spendable or Reserve. Hidden tiers keep their legend chip (to toggle
  // back) but drop their line/fill.
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const isHidden = (tier: string) => hidden.has(tier);
  const toggle = (tier: string) =>
    setHidden((prev) => {
      const next = new Set(prev);
      if (next.has(tier)) next.delete(tier);
      else next.add(tier);
      return next;
    });

  if (!spine || dayCount < 2) {
    return (
      <p className="py-10 text-center text-sm text-muted-foreground">
        Not enough data to chart yet.
      </p>
    );
  }

  // The user deselected everything in the picker — guide them back rather than
  // showing an empty grid (adversarial review of 4d8.25.26).
  if (tiers.length === 0 && plottedAccounts.length === 0) {
    return (
      <p className="py-10 text-center text-sm text-muted-foreground">
        {/* Accounts CAN be picked yet unplottable: every free colour went to a tier.
            Saying "nothing selected" there would contradict the picker the user is
            looking at (personal-cfo-hnba). */}
        {unplottedAccounts.length > 0
          ? "No colours left for the selected accounts — deselect a cash tier in “Series” to free one."
          : "No series selected — pick one from “Series”."}
      </p>
    );
  }

  // The date-keyed union of realized history and the forward projection
  // (personal-cfo-4d8.27.5.3): history rows carry `hist_<key>`, forward rows the
  // existing keys plus each tier's `band_<tier>` = [p10, p90] composite cone.
  const {
    rows: data,
    anyNegative,
    historyDayCount,
    bandedTiers,
  } = assembleMultiSeriesRows({
    groups,
    tiers,
    plottedAccounts,
    history,
    todayIso,
  });
  const hasHistory = historyDayCount > 0;
  // The plot's ends, for the half-labelling reference areas.
  const firstDate = data[0]?.date;
  const lastDate = data[data.length - 1]?.date;
  // ONE continuous stroke. The projected half is deliberately NOT dashed
  // (personal-cfo-7c7a): dashes are the chart vocabulary for missing or interrupted data,
  // so on a projection they claim the MEDIAN is imprecise — but the median is exactly
  // computed and the app states it to the cent. What is uncertain is the spread, and the
  // spread is already drawn as the band. Dash plus band double-encodes one fact, and at
  // daily resolution over a 90-day horizon it leaves the line ~40% absent, which reads as
  // thin data rather than as an estimate.
  //
  // The evidence carries the boundary instead: the past has NO band at all, and that
  // absence is itself the claim that nothing on the left is estimated.
  const forwardDash = undefined;

  const config: ChartConfig = {};
  for (const tier of tiers) {
    config[tier] = {
      label: TIER_META[tier]?.label ?? tier,
      color: TIER_META[tier]?.color,
    };
    config[`hist_${tier}`] = {
      label: `${TIER_META[tier]?.label ?? tier} (realized)`,
      color: TIER_META[tier]?.color,
    };
  }
  for (const acc of plottedAccounts) {
    config[acc.key] = { label: acc.label, color: acc.color };
    config[`hist_${acc.key}`] = {
      label: `${acc.label} (realized)`,
      color: acc.color,
    };
  }

  // The plotted series in legend order: aggregate tiers (prominence order) then
  // individual accounts.
  const legendSeries = [
    ...tiers.map((t) => ({ key: t, label: TIER_META[t]?.label ?? t, color: TIER_META[t]?.color })),
    ...plottedAccounts.map((a) => ({ key: a.key, label: a.label, color: a.color })),
  ];

  const bandLabel = { fontSize: 10, fill: "var(--muted-foreground)" };
  // a11y copy derived from what is ACTUALLY plotted: only claim a "(primary)" hero
  // when Spendable is drawn (it is dropped when flat-zero across the horizon —
  // adversarial review of 4d8.25.25).
  const otherLabels = legendSeries
    .map((s) => s.label)
    .filter((l) => l !== TIER_META.spendable?.label);
  const seriesDescription = tiers.includes("spendable")
    ? `${TIER_META.spendable?.label ?? "Spendable"} (primary)${
        otherLabels.length > 0 ? ` plus ${otherLabels.join(", ")}` : ""
      }`
    : legendSeries.map((s) => s.label).join(", ");

  const figureLabel = hasHistory
    ? `Liquid cash by group: ${historyDayCount} days of realized history, then the projected range over the next ${dayCount} days: ${seriesDescription}`
    : `Projected liquid cash by group over the next ${dayCount} days: ${seriesDescription}`;

  return (
    <figure className="flex flex-col gap-2" aria-label={figureLabel}>
      <ChartContainer config={config} className="h-80">
        <ComposedChart data={data} margin={{ left: 8, right: 12, top: 8, bottom: 0 }}>
          <defs>
            {/* The signature fill sits under the hero SPENDABLE series (personal-cfo-4d8.25.25). */}
            <linearGradient id="spendableFill" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="var(--color-spendable)" stopOpacity={0.28} />
              <stop offset="55%" stopColor="var(--color-spendable)" stopOpacity={0.08} />
              <stop offset="100%" stopColor="var(--color-spendable)" stopOpacity={0} />
            </linearGradient>
          </defs>
          <CartesianGrid vertical={false} stroke="var(--border)" strokeOpacity={0.5} />
          {/* Comfort band (915.1): shade the target range behind the series; the lower edge is
              the floor, always drawn; the upper edge only when set.

              NEUTRAL, not a chart slot. It used to be shaded --chart-2 — the same slot
              Reserve plots in — so turning Reserve on drew its line in the reference
              region's own hue, and the two blurred together exactly when both mattered.

              The comfort band is not a categorical series; it is a REFERENCE REGION, a
              target the balance is read against. Giving it a slot also silently spent one
              of the four ADR 0054 allows on something that is not a series. */}
          {band && band.upper !== null && band.upper > band.lower && (
            <ReferenceArea
              y1={band.lower}
              y2={band.upper}
              fill="var(--foreground)"
              fillOpacity={0.035}
              ifOverflow="extendDomain"
            />
          )}
          {band && (
            <ReferenceLine
              y={band.lower}
              stroke="var(--muted-foreground)"
              strokeDasharray="4 4"
              strokeOpacity={0.7}
              ifOverflow="extendDomain"
              label={{
                ...bandLabel,
                value: `Floor ${compactMoney(band.lower, currency)}`,
                position: "insideBottomRight",
              }}
            />
          )}
          {band && band.upper !== null && (
            <ReferenceLine
              y={band.upper}
              stroke="var(--muted-foreground)"
              strokeDasharray="4 4"
              strokeOpacity={0.7}
              ifOverflow="extendDomain"
              label={{
                ...bandLabel,
                value: `Target ${compactMoney(band.upper, currency)}`,
                position: "insideTopLeft",
              }}
            />
          )}
          {/* Ground the eye when any series goes below zero. */}
          {anyNegative && (
            <ReferenceLine y={0} stroke="var(--loss)" strokeOpacity={0.35} />
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
            tick={{ fontSize: 11 }}
            tickFormatter={(value) => compactMoney(Number(value), currency)}
          />
          <ChartTooltip
            cursor={{ stroke: "var(--muted-foreground)", strokeOpacity: 0.35 }}
            content={(tooltipProps) => {
              // A hovered row carries only ONE half's keys (history vs forward) —
              // drop the other half's undefined entries so the card doesn't fill
              // with "—" rows (adversarial review of 4d8.27.5.3).
              const props =
                tooltipProps as ComponentProps<typeof ChartTooltipContent>;
              return (
                <ChartTooltipContent
                  {...props}
                  payload={props.payload?.filter(
                    (entry) => entry.value !== undefined && entry.value !== null,
                  )}
                  currency={currency}
                  labelFormatter={formatIsoDate}
                />
              );
            }}
          />
          {/* Spendable: the gradient wash under the bold hero line, drawn FIRST so it
              sits behind the other lines (personal-cfo-4d8.25.25). `spendableWash`
              carries realized-then-projected values so the wash is continuous across
              the TODAY divider. tooltipType="none" keeps it out of the hover card. */}
          {!isHidden("spendable") && tiers.includes("spendable") && (
            <Area
              dataKey="spendableWash"
              type="monotone"
              stroke="none"
              fill="url(#spendableFill)"
              tooltipType="none"
              isAnimationActive
              animationDuration={500}
            />
          )}
          {/* Each plotted tier's forward P10–P90 band — the composite of the
              per-account variance cones (ADR 0050, personal-cfo-4d8.27.5.3). Only
              tiers whose cone actually has width get an Area.

              The band is the ENTIRE mechanism by which the projected half declares itself
              uncertain (personal-cfo-7c7a). If it washes out, the chart silently becomes a
              confident line — so its strength is measured against the shipped surfaces
              rather than eyeballed; see bandLegibility.test.ts.

              The fill is raised in BOTH themes, not just dark. The premise that a dark
              surface always loses contrast does not hold here: this palette swaps in
              brighter chart tokens for dark, so at the old 10% the WEAKEST case was
              light-mode terracotta (ΔE 3.4), not anything in dark. A dark-only bump would
              have strengthened the half that was already ahead.

              The stippled edge is the part that does not depend on the fill reading at
              all: where the wash is lost — over the gradient, over gridlines, on a bright
              screen — the dotted boundary still carries the band's extent. It is dotted
              rather than solid on purpose: a percentile edge is not a promise about where
              the range ends, and a hard line would claim it is. */}
          {bandedTiers
            .filter((tier) => !isHidden(tier))
            .map((tier) => (
              <Area
                key={`band_${tier}`}
                dataKey={`band_${tier}`}
                type="monotone"
                stroke={`var(--color-${tier})`}
                strokeWidth={1}
                strokeDasharray={BAND_EDGE_DASH}
                strokeOpacity={BAND_EDGE_OPACITY}
                fill={`var(--color-${tier})`}
                fillOpacity={BAND_FILL_OPACITY}
                tooltipType="none"
                isAnimationActive
                animationDuration={500}
              />
            ))}
          {/* The boundary between what happened and what is projected
              (personal-cfo-7c7a).

              LABEL THE GROUND, NOT THE LINE. The projected half sits on a faint foreground
              tint and the two halves are named above the plot; the stroke itself says
              nothing about which side it is on. The tint is the adjustable dial — if the
              halves need more separation, raise it rather than touching the line. */}
          {hasHistory && todayIso && lastDate && (
            <ReferenceArea
              x1={todayIso}
              x2={lastDate}
              fill="var(--foreground)"
              fillOpacity={0.025}
              ifOverflow="extendDomain"
              label={{
                value: "PROJECTED",
                position: "insideTopRight",
                fontSize: 9,
                letterSpacing: "0.08em",
                fill: "var(--muted-foreground)",
              }}
            />
          )}
          {hasHistory && firstDate && todayIso && (
            <ReferenceArea
              x1={firstDate}
              x2={todayIso}
              fill="transparent"
              ifOverflow="extendDomain"
              label={{
                value: "REALIZED",
                position: "insideTopLeft",
                fontSize: 9,
                letterSpacing: "0.08em",
                fill: "var(--muted-foreground)",
              }}
            />
          )}
          {/* A RULE, not a gap. Whitespace at the seam would read as a data break, which is
              a different claim than "the estimate begins here". */}
          {hasHistory && todayIso && (
            <ReferenceLine
              x={todayIso}
              stroke="var(--muted-foreground)"
              strokeOpacity={0.45}
              label={{
                value: "TODAY",
                position: "top",
                fontSize: 10,
                fill: "var(--muted-foreground)",
              }}
            />
          )}
          {/* Realized history: SOLID lines per visible series, up to today.
              Unallocated is forward-only (no fabricated realized-0 line). */}
          {hasHistory &&
            tiers
              .filter((tier) => tier !== "unallocated" && !isHidden(tier))
              .map((tier) => (
                <Line
                  key={`hist_${tier}`}
                  dataKey={`hist_${tier}`}
                  type="monotone"
                  stroke={`var(--color-${tier})`}
                  strokeWidth={TIER_STROKE_WIDTH[tier] ?? 1.5}
                  strokeOpacity={tier === "net" ? 0.7 : 0.9}
                  dot={false}
                  activeDot={{ r: 4, strokeWidth: 2, stroke: "var(--card)" }}
                  isAnimationActive
                  animationDuration={500}
                />
              ))}
          {hasHistory &&
            plottedAccounts
              .filter((acc) => !isHidden(acc.key))
              .map((acc) => (
                <Line
                  key={`hist_${acc.key}`}
                  dataKey={`hist_${acc.key}`}
                  type="monotone"
                  stroke={`var(--color-${acc.key})`}
                  strokeWidth={1.5}
                  strokeOpacity={0.9}
                  dot={false}
                  activeDot={{ r: 4, strokeWidth: 2, stroke: "var(--card)" }}
                  isAnimationActive
                  animationDuration={500}
                />
              ))}
          {/* Secondary lines (Reserve, Net, Unallocated) UNDER the hero, in reverse
              prominence so the least-prominent draws first. */}
          {(["unallocated", "net", "reserve"] as const)
            .filter((tier) => tiers.includes(tier) && !isHidden(tier))
            .map((tier) => (
              <Line
                key={tier}
                dataKey={tier}
                type="monotone"
                stroke={`var(--color-${tier})`}
                strokeWidth={TIER_STROKE_WIDTH[tier] ?? 1.5}
                strokeOpacity={tier === "net" ? 0.7 : 0.9}
                strokeDasharray={tier === "unallocated" ? "5 3" : forwardDash}
                dot={false}
                activeDot={{ r: 4, strokeWidth: 2, stroke: "var(--card)" }}
                isAnimationActive
                animationDuration={500}
              />
            ))}
          {/* Individual-account lines (personal-cfo-4d8.25.26), thin, under the hero. */}
          {plottedAccounts
            .filter((acc) => !isHidden(acc.key))
            .map((acc) => (
              <Line
                key={acc.key}
                dataKey={acc.key}
                type="monotone"
                stroke={`var(--color-${acc.key})`}
                strokeWidth={1.5}
                strokeOpacity={0.9}
                strokeDasharray={forwardDash}
                dot={false}
                activeDot={{ r: 4, strokeWidth: 2, stroke: "var(--card)" }}
                isAnimationActive
                animationDuration={500}
              />
            ))}
          {/* The hero Spendable line, drawn LAST (on top). */}
          {!isHidden("spendable") && tiers.includes("spendable") && (
            <Line
              dataKey="spendable"
              type="monotone"
              stroke="var(--color-spendable)"
              strokeWidth={TIER_STROKE_WIDTH.spendable}
              strokeDasharray={forwardDash}
              dot={false}
              activeDot={{ r: 5, strokeWidth: 2, stroke: "var(--card)" }}
              isAnimationActive
              animationDuration={500}
            />
          )}
        </ComposedChart>
      </ChartContainer>

      <figcaption className="flex flex-wrap items-center gap-x-2 gap-y-1 px-1 text-xs text-muted-foreground">
        {legendSeries.map((series) => {
          const off = isHidden(series.key);
          return (
            <button
              key={series.key}
              type="button"
              onClick={() => toggle(series.key)}
              aria-pressed={!off}
              className={`inline-flex items-center gap-1.5 rounded px-1.5 py-0.5 transition-opacity hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${
                off ? "opacity-40" : ""
              }`}
            >
              <span
                aria-hidden
                className={
                  series.key === "spendable"
                    ? "inline-block h-1 w-4 rounded-full"
                    : "inline-block h-0.5 w-4 rounded-full"
                }
                style={{ backgroundColor: series.color }}
              />
              {series.label}
              <span className="sr-only">{off ? " (hidden — click to show)" : " (click to hide)"}</span>
            </button>
          );
        })}
        {hasHistory && (
          <span className="ml-auto inline-flex items-center gap-3">
            <span className="inline-flex items-center gap-1.5">
              <span
                aria-hidden
                className="inline-block h-0.5 w-4 rounded-full bg-muted-foreground"
              />
              Realized
            </span>
            <span className="inline-flex items-center gap-1.5">
              <span
                aria-hidden
                className="inline-block w-4 border-t-2 border-dashed border-muted-foreground"
              />
              Projected
            </span>
          </span>
        )}
      </figcaption>
      {/* Say what was left out. The palette is four fixed slots and colours are never
          reused (ADR 0054), so past the free slots an account cannot be drawn
          distinguishably — and silently dropping a series the user explicitly picked
          would be its own kind of lie (personal-cfo-hnba). */}
      {unplottedAccounts.length > 0 && (
        <p className="px-1 text-xs text-muted-foreground">
          Not plotted, to keep every line a distinct colour:{" "}
          <span className="font-medium text-foreground">
            {unplottedAccounts
              .map((a) => accountLabel(seriesAsLabelled(a), pickedAccounts.map(seriesAsLabelled)))
              .join(", ")}
          </span>
          . Deselect another series in “Series” to make room.
        </p>
      )}
    </figure>
  );
}
