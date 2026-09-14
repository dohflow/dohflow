import { useCallback, useEffect, useState } from "react";
import { Loader2 } from "lucide-react";

import type { ForecastDayDto } from "@/bindings";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  formatIsoDate,
  formatMoney,
  formatSignedMoney,
  signedAmountClass,
} from "@/lib/format";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { cn } from "@/lib/utils";
import { classifyCashFlow } from "./cashFlowState";
import { MultiSeriesChart } from "./MultiSeriesChart";
import { SeriesPicker } from "./SeriesPicker";
import { useFutureCashSeries } from "./useFutureCashSeries";
import { ComfortBandSignals } from "./ComfortBandSignals";
import { CoverItNotice } from "./CoverItNotice";
import { NeedsConfirmation } from "./NeedsConfirmation";
import { ProjectedActivityTable } from "./ProjectedActivityTable";
import { FutureCashEntries } from "./FutureCashEntries";
import { MarkedPaidUndoBar } from "./MarkedPaidUndoBar";
import {
  MarkObligationUndoProvider,
  type ConfirmedObligation,
} from "./markObligationUndo";
import { useMarkObligation } from "./useMarkObligation";
import { useCashFlowHistory } from "@/accounts/useCashFlowHistory";
import { useComfortBand } from "@/dashboard/useComfortBand";
import { ScenarioBar } from "./ScenarioBar";
import { ScenarioLayers } from "./ScenarioLayers";
import { ScenarioStack } from "./ScenarioStack";
import { ScenarioEvents } from "./ScenarioEvents";
import {
  DEFAULT_FUTURE_CASH_HORIZON,
  FUTURE_CASH_HORIZONS,
  useFutureCash,
  useFutureCashByAccount,
} from "./useFutureCash";

/// The Future Cash view (personal-cfo-eqzs): the deterministic projection as a
/// chart over a selectable horizon, plus a read-only ledger of the income and
/// bills that shape it. The statistical band, hover-to-explain, scenario overlays,
/// and inline editing layer onto this surface in later beads.
/// Realized-history lookbacks for the chart's past half (personal-cfo-4d8.27.5.3).
/// The backend clamps each account to its earliest real data (the honesty rule), so
/// a longer lookback never fabricates — a young household just sees a shorter line.
const HISTORY_LOOKBACKS = [
  { days: 0, label: "Off" },
  { days: 30, label: "1M" },
  { days: 90, label: "3M" },
  { days: 180, label: "6M" },
  { days: 365, label: "1Y" },
] as const;
const DEFAULT_HISTORY_LOOKBACK = 90;

export function FutureCashView({
  scenarioId: controlledScenarioId,
  onScenarioChange,
}: {
  /// The selected scenario, owned by the shell so that opening one from the Scenarios
  /// tab and switching back to base in the bar are the same piece of state (ADR 0051
  /// §5). Omitted (tests, embeds) → the view owns it locally.
  scenarioId?: string | null;
  onScenarioChange?: (id: string | null) => void;
} = {}) {
  const [horizon, setHorizon] = useState<number>(DEFAULT_FUTURE_CASH_HORIZON);
  const [lookback, setLookback] = useState<number>(DEFAULT_HISTORY_LOOKBACK);
  const [localScenarioId, setLocalScenarioId] = useState<string | null>(null);
  const scenarioId = controlledScenarioId ?? localScenarioId;
  const setScenarioId = onScenarioChange ?? setLocalScenarioId;
  // Further scenarios stacked ON TOP of the selected one (personal-cfo-4d8.27.6.4).
  // The selected scenario stays the one being EDITED (ScenarioBar + ScenarioEvents act on
  // it); these compose over it. Order is the precedence (ADR 0059 §1), and a later entry
  // wins a conflict — which is why the stack is rendered in order rather than as a set.
  const [stackedIds, setStackedIds] = useState<string[]>([]);
  const selection = scenarioId === null ? [] : [scenarioId, ...stackedIds];
  // Always load the base; load the scenario when one is selected (same key as
  // base when `null`, so TanStack dedups). The compare delta reads both. The
  // aggregate drives the stat cards + compare; the per-account/per-group
  // projection (personal-cfo-l8oh) drives the multi-series chart + table.
  const base = useFutureCash(horizon, []);
  const scenarioForecast = useFutureCash(horizon, selection);
  const { forecast, error } = scenarioId ? scenarioForecast : base;
  const baseProjection = useFutureCashByAccount(horizon, []);
  const scenarioProjection = useFutureCashByAccount(horizon, selection);
  const { selection: seriesSelection, setSelection: setSeriesSelection } =
    useFutureCashSeries();
  const { projection } = scenarioId ? scenarioProjection : baseProjection;
  // The realized past behind the projection (4d8.27.5.3). Lookback 0 = off.
  const { history } = useCashFlowHistory(lookback);
  const historyAccounts = lookback > 0 ? (history?.accounts ?? null) : null;

  // The comfort band (915.1) shaded behind the chart, in minor units.
  const { band } = useComfortBand();
  const chartBand =
    band && (band.lower.minor_units > 0 || band.upper !== null)
      ? { lower: band.lower.minor_units, upper: band.upper?.minor_units ?? null }
      : null;

  // A just-marked-paid occurrence leaves the forecast, so its Undo surfaces here as a
  // view-level bar (personal-cfo-5ie.9). Auto-clears after a short window.
  const { unconfirm } = useMarkObligation();
  const [justPaid, setJustPaid] = useState<ConfirmedObligation | null>(null);
  const [undoError, setUndoError] = useState<string | null>(null);
  const publishConfirmed = useCallback((info: ConfirmedObligation) => {
    setUndoError(null);
    setJustPaid(info);
  }, []);
  // Auto-clear a successful confirm after a short window — but NOT while an undo is in flight or
  // an undo error is showing, so a slow/failed undo can't have the bar (and its error/retry)
  // vanish out from under it (5ie.9 review).
  useEffect(() => {
    if (!justPaid || undoError || unconfirm.isPending) return undefined;
    const timer = setTimeout(() => setJustPaid(null), 12_000);
    return () => clearTimeout(timer);
  }, [justPaid, undoError, unconfirm.isPending]);
  async function undoJustPaid() {
    if (!justPaid) return;
    const result = await unconfirm.mutateAsync({
      recurring_event_id: justPaid.eventId,
      scheduled_date: justPaid.scheduledDate,
      idempotency_key: mintIdempotencyKey(),
    });
    if (result.status === "ok") {
      setJustPaid(null);
    } else {
      setUndoError("Couldn't undo — try again.");
    }
  }

  if (error) {
    return (
      <div className="mx-auto w-full max-w-5xl">
        {/* Name what failed AND whether the data is intact (personal-cfo-4fbl, matching
            the distinction drawn for the transactions list in personal-cfo-xu32).

            The forecast is a COMPUTED read over the ledger, so it can fail while every
            account and transaction behind it is perfectly fine. An anonymous red message
            leaves the reader unable to tell a failed projection from a damaged vault —
            on a financial surface that is the difference between "try again" and
            "restore from backup". */}
        <p role="alert" className="text-sm text-loss">
          Couldn&apos;t build your projection. Your vault is readable — this forecast
          failed to compute, so your accounts and transactions are unaffected. ({error})
        </p>
      </div>
    );
  }

  if (forecast === null) {
    return (
      <div className="flex items-center justify-center gap-2 py-16 text-muted-foreground">
        <Loader2 className="size-5 animate-spin" aria-hidden />
        Loading your forecast…
      </div>
    );
  }

  const { currency, starting_balance, days } = forecast;

  // Not "no data" — NOT ENOUGH TO PROJECT, and which prerequisite is missing decides what
  // the reader should do next (personal-cfo-4fbl). Classified only once the projection has
  // arrived: while it is still loading, "no events yet" is indistinguishable from "no
  // events at all", and showing the empty state then would tell the user to go fix
  // something that is not broken.
  const cashFlowState =
    projection === null
      ? "ready"
      : classifyCashFlow({
          liquidAccountCount: projection.accounts.length,
          days,
        });

  // The lowest projected closing balance over the horizon (the "will I dip too
  // low" signal). Falls back to today's balance on an empty horizon.
  const lowest = days.reduce<ForecastDayDto | null>((min, day) => {
    if (min === null) return day;
    return day.closing.p50.minor_units < min.closing.p50.minor_units
      ? day
      : min;
  }, null);
  const endBalance = days.at(-1)?.closing.p50 ?? starting_balance;

  // Compare-vs-base: the scenario's projected end balance minus the base's, shown
  // only when a scenario is active and the base has loaded (personal-cfo-6zep).
  const baseEndMinor =
    base.forecast?.days.at(-1)?.closing.p50.minor_units ??
    base.forecast?.starting_balance.minor_units ??
    null;
  const compareDeltaMinor =
    scenarioId !== null && baseEndMinor !== null
      ? endBalance.minor_units - baseEndMinor
      : null;

  return (
    <MarkObligationUndoProvider value={publishConfirmed}>
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-6">
        <div className="flex items-center justify-between gap-4">
          <h2 className="text-xl font-semibold tracking-tight">Cash Flow</h2>
          <HorizonPicker value={horizon} onChange={setHorizon} />
        </div>

        {justPaid && (
          <MarkedPaidUndoBar
            name={justPaid.name}
            onUndo={() => void undoJustPaid()}
            onDismiss={() => setJustPaid(null)}
            pending={unconfirm.isPending}
            error={undoError}
          />
        )}

        <ScenarioBar
          selectedId={scenarioId}
          onSelect={(id) => {
            setScenarioId(id);
            // Switching the edited scenario clears the stack: a stack composed over a
            // different base is not the same question, and silently keeping it would show
            // a forecast the user did not ask for.
            setStackedIds([]);
          }}
        />
        {scenarioId !== null && (
          <>
            {/* The chips say WHICH scenarios are on and in what order; the pile below says
                what that order does. Chips first: picking the stack precedes reading it. */}
            <ScenarioStack
              primaryId={scenarioId}
              stacked={stackedIds}
              onChange={setStackedIds}
            />
            <ScenarioLayers
              primaryId={scenarioId}
              stacked={stackedIds}
              onReorder={setStackedIds}
            />
          </>
        )}

        {compareDeltaMinor !== null && (
          <div
            role="status"
            className="rounded-md border bg-muted/30 px-4 py-3 text-sm"
          >
            Compared to base at {horizon} days:{" "}
            <span
              className={cn(
                "font-semibold tabular-nums",
                signedAmountClass({ minor_units: compareDeltaMinor, currency }),
              )}
            >
              {formatSignedMoney({ minor_units: compareDeltaMinor, currency })}
            </span>
          </div>
        )}

        <div className="grid gap-4 sm:grid-cols-3">
          <StatCard
            label="Liquid cash today"
            value={formatMoney(starting_balance)}
          />
          <StatCard
            label={
              lowest
                ? `Lowest projected · ${formatIsoDate(lowest.date)}`
                : "Lowest projected"
            }
            value={formatMoney(lowest?.closing.p50 ?? starting_balance)}
            negative={(lowest?.closing.p50.minor_units ?? 0) < 0}
          />
          <StatCard
            label="Projected at horizon end"
            value={formatMoney(endBalance)}
          />
        </div>

        <Card>
          <CardHeader className="flex flex-row items-start justify-between gap-2 space-y-0">
            <div className="flex flex-col gap-1.5">
              <CardTitle>Liquid cash — history &amp; projection</CardTitle>
              <CardDescription>
                {lookback > 0
                  ? "Where your cash has actually been (solid, back to your real data), then where it is headed (dashed, with its likely range) by group."
                  : "Your liquid cash projected forward by group, with its likely range."}
              </CardDescription>
            </div>
            <div className="flex flex-wrap items-center justify-end gap-2">
              <LookbackPicker value={lookback} onChange={setLookback} />
              {projection && (
                <SeriesPicker
                  accounts={projection.accounts}
                  selection={seriesSelection}
                  onChange={(next) => void setSeriesSelection(next)}
                />
              )}
            </div>
          </CardHeader>
          <CardContent>
            {cashFlowState !== "ready" ? (
              <NotEnoughToProject state={cashFlowState} />
            ) : projection ? (
              <MultiSeriesChart
                groups={projection.groups}
                accounts={projection.accounts}
                selection={seriesSelection}
                currency={currency}
                band={chartBand}
                history={historyAccounts}
                todayIso={projection.start_date}
              />
            ) : (
              <ProjectionLoading />
            )}
          </CardContent>
        </Card>

        {/* Above Projected Activity on purpose (ADR 0058 §1): the projection below is
            only as trustworthy as this section is empty, so the fix sits next to the
            figure it corrects. Renders nothing when the queue is clear. */}
        <NeedsConfirmation />

        {projection && <CoverItNotice projection={projection} />}
        <ComfortBandSignals days={days} band={chartBand} currency={currency} />

        <FutureCashEntries currency={currency} />

        {scenarioId !== null && (
          <ScenarioEvents scenarioId={scenarioId} currency={currency} />
        )}

        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-sm">Projected activity</CardTitle>
            <CardDescription>
              Today, then each projected activity, with the running balance per
              account and group. Select a row to see why it is projected.
            </CardDescription>
          </CardHeader>
          <CardContent className="p-0">
            {projection ? (
              cashFlowState !== "ready" ? (
                <div className="px-6 pb-6">
                  <NotEnoughToProject state={cashFlowState} compact />
                </div>
              ) : (
                <ProjectedActivityTable projection={projection} />
              )
            ) : (
              <ProjectionLoading />
            )}
          </CardContent>
        </Card>
      </div>
    </MarkObligationUndoProvider>
  );
}

/// A small inline loader for the projection-backed sections (chart + table), which
/// load independently of the aggregate stats.
/// What to show when the forecast has nothing to project (personal-cfo-4fbl).
///
/// Deliberately NOT "no data". The forecast needs a balance to start from and a schedule to
/// project; each sentence names the missing half and the single action that supplies it, so
/// the state reads as a next step rather than a dead end.
function NotEnoughToProject({
  state,
  compact = false,
}: {
  state: "no-anchor" | "no-schedule";
  /// The Projected Activity card sits directly under the chart, which already carries the
  /// full explanation. Repeating it verbatim would read as two separate problems rather
  /// than one, so the second slot states the fact only.
  compact?: boolean;
}) {
  const [headline, detail] =
    state === "no-anchor"
      ? [
          "No account to project from yet.",
          "A forecast starts from a real balance. Add a checking or savings account and set its balance — everything here is projected forward from that number.",
        ]
      : [
          "Nothing scheduled to project yet.",
          "Your balance is set, but nothing is expected to move it. Add your income and a bill or two, and the projection fills in from there.",
        ];
  if (compact) {
    return <p className="text-sm text-muted-foreground">{headline}</p>;
  }
  return (
    <div className="flex flex-col items-center gap-1.5 py-12 text-center">
      <p className="text-sm font-medium text-foreground">{headline}</p>
      <p className="max-w-md text-sm text-muted-foreground">{detail}</p>
    </div>
  );
}

function ProjectionLoading() {
  return (
    <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted-foreground">
      <Loader2 className="size-4 animate-spin" aria-hidden />
      Loading…
    </div>
  );
}

/// The realized-history lookback control (personal-cfo-4d8.27.5.3), mirroring the
/// horizon picker. "Off" hides the past half entirely.
function LookbackPicker({
  value,
  onChange,
}: {
  value: number;
  onChange: (days: number) => void;
}) {
  return (
    <div
      className="inline-flex rounded-md border p-0.5"
      role="group"
      aria-label="History lookback"
    >
      {HISTORY_LOOKBACKS.map((option) => (
        <button
          key={option.days}
          type="button"
          onClick={() => onChange(option.days)}
          aria-pressed={value === option.days}
          className={cn(
            "rounded px-2.5 py-1 text-xs font-medium transition-colors",
            value === option.days
              ? "bg-primary/10 text-primary"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function HorizonPicker({
  value,
  onChange,
}: {
  value: number;
  onChange: (days: number) => void;
}) {
  return (
    <div
      className="inline-flex rounded-md border p-0.5"
      role="group"
      aria-label="Forecast horizon"
    >
      {FUTURE_CASH_HORIZONS.map((option) => (
        <button
          key={option.days}
          type="button"
          onClick={() => onChange(option.days)}
          aria-pressed={value === option.days}
          className={cn(
            "rounded px-2.5 py-1 text-xs font-medium transition-colors",
            value === option.days
              ? "bg-primary/10 text-primary"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function StatCard({
  label,
  value,
  negative = false,
}: {
  label: string;
  value: string;
  negative?: boolean;
}) {
  return (
    <Card>
      <CardContent className="flex flex-col gap-1 pt-6">
        <span className="text-xs font-medium text-muted-foreground">
          {label}
        </span>
        <span
          className={cn(
            "text-2xl font-semibold tabular-nums",
            negative && "text-loss",
          )}
        >
          {value}
        </span>
      </CardContent>
    </Card>
  );
}
