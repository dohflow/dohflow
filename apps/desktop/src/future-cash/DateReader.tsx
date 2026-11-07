import { useId } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";

import type { AccountHistoryDto, MoneyDto, MultiSeriesForecastDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { NativeSelect } from "@/components/ui/native-select";
import { formatIsoDate, formatMoney, formatSignedMoney } from "@/lib/format";
import { useSessionState } from "@/lib/sessionViewState";
import { cn } from "@/lib/utils";

const TIER_LABEL: Record<string, string> = {
  spendable: "Spendable",
  reserve: "Reserve",
  unallocated: "Unallocated",
  net: "Net cash",
};
const TIER_ORDER: Record<string, number> = { spendable: 0, reserve: 1, unallocated: 2, net: 3 };

type Band = { p10: MoneyDto; p50: MoneyDto; p90: MoneyDto };

/// The nearest date in `spine` to `wanted` (an ISO date compares as text), for when a restored selection is
/// no longer a date the forecast offers.
function nearest(spine: string[], wanted: string): string {
  let best = spine[0]!;
  for (const date of spine) if (Math.abs(Date.parse(date) - Date.parse(wanted)) < Math.abs(Date.parse(best) - Date.parse(wanted))) best = date;
  return best;
}

/// "Read forecast by date" (personal-cfo-i93n3.27 slice C; Mobile design 2, "Charts and complete reading"): a
/// reading of one date of the SAME horizon, scenario and series data the chart plots, reachable without a chart
/// gesture. A labeled native date control plus Previous day and Next day walk the spine of dates the forecast
/// offers (nothing between them is interpolated). It shows the date, the opening balance, each activity with its
/// signed amount and account, and the closing balance by group and account, with P10, P50 and P90 from the same
/// data (never summed or invented). A date with no activity says so, and its closing is the data's own.
export function DateReader({
  projection,
  history = null,
}: {
  projection: MultiSeriesForecastDto;
  /// The realized past behind the projection for the active lookback (personal-cfo-4d8.27.5.3): each account's
  /// recorded daily closings. Its dates join the spine, and a recorded date reads as recorded balances with no forecast
  /// range. Nothing is interpolated: only dates the data has.
  history?: AccountHistoryDto[] | null;
}) {
  const id = useId();
  const [stored, setSelected] = useSessionState<string | null>("forecast.selectedDate", null);

  const forecastDates = new Set(projection.groups.flatMap((g) => g.closings.map((c) => c.date)));
  // Recorded dates before the forecast starts; a date the forecast also has is read as the forecast (it has the range).
  const recordedDates = new Set(
    (history ?? []).flatMap((account) => account.days.map((d) => d.date)).filter((d) => !forecastDates.has(d)),
  );
  const spine = [...new Set([...forecastDates, ...recordedDates])].sort();
  if (spine.length === 0) return null;
  // A restored date the forecast no longer offers (a shorter horizon, a different scenario) becomes the nearest
  // available one, and the reader says so.
  const wanted = stored ?? spine[0]!;
  const date = spine.includes(wanted) ? wanted : nearest(spine, wanted);
  const moved = stored !== null && date !== stored;
  const index = spine.indexOf(date);

  const recorded = recordedDates.has(date);
  const recordedClosing = (account: AccountHistoryDto, on: string): MoneyDto | null =>
    account.days.find((d) => d.date === on)?.closing ?? null;
  const closingOf = (closings: { date: string; closing: Band }[], on: string): Band | null =>
    closings.find((c) => c.date === on)?.closing ?? null;
  const groups = [...projection.groups].sort((a, b) => (TIER_ORDER[a.tier] ?? 99) - (TIER_ORDER[b.tier] ?? 99));
  const previous = index > 0 ? spine[index - 1]! : null;

  const activities = projection.accounts.flatMap((account) =>
    account.days
      .filter((d) => d.date === date)
      .flatMap((d) => d.events.map((event) => ({ event, accountName: account.name }))),
  );
  const accountsWithClosing = projection.accounts
    .map((account) => ({ account, closing: closingOf(account.days.map((d) => ({ date: d.date, closing: d.closing })), date) }))
    .filter((a) => a.closing !== null);

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">Read forecast by date</CardTitle>
        <CardDescription>One date at a time: the activity on it and the balances it leaves.</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="compact:flex-1"
            disabled={index === 0}
            onClick={() => setSelected(spine[index - 1]!)}
          >
            <ChevronLeft aria-hidden />
            Previous day
          </Button>
          {/* Named by `aria-label`, not a separate (screen-reader-only) label element. */}
          <NativeSelect
            id={`${id}-date`}
            aria-label="Forecast date"
            value={date}
            onChange={(event) => setSelected(event.target.value)}
            // On a phone the date control takes its own full-width row above the two buttons (squeezed between them it
            // showed one letter, "N").
            className="min-w-0 flex-1 compact:order-first compact:w-full compact:max-w-full compact:basis-full compact:[&>select]:min-w-0"
          >
            {spine.map((d) => (
              <option key={d} value={d}>
                {formatIsoDate(d)}
              </option>
            ))}
          </NativeSelect>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="compact:flex-1"
            disabled={index === spine.length - 1}
            onClick={() => setSelected(spine[index + 1]!)}
          >
            Next day
            <ChevronRight aria-hidden />
          </Button>
        </div>
        {/* A change of horizon or scenario can remove the selected date: say which one is shown instead. */}
        <p aria-live="polite" className={cn("text-sm text-muted-foreground", !moved && "sr-only")}>
          {moved ? `Showing ${formatIsoDate(date)}` : ""}
        </p>

        {recorded ? (
          <section aria-label={`Recorded balances for ${formatIsoDate(date)}`} className="flex flex-col gap-4">
            <h3 className="text-base font-semibold">{formatIsoDate(date)}</h3>
            <p className="text-sm text-muted-foreground">Recorded balances.</p>
            <div>
              <h4 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Opening balance</h4>
              {previous === null ? (
                <p className="text-sm text-muted-foreground">The history starts here.</p>
              ) : (
                <dl className="grid grid-cols-[1fr_auto] gap-x-4 text-sm">
                  {(history ?? []).map((account) => {
                    const opening = recordedClosing(account, previous);
                    return opening === null ? null : (
                      <Row key={account.account_id} label={account.name} value={formatMoney(opening)} />
                    );
                  })}
                </dl>
              )}
            </div>
            <div>
              <h4 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Closing balance</h4>
              <dl className="grid grid-cols-[1fr_auto] gap-x-4 gap-y-1 text-sm">
                {(history ?? []).map((account) => {
                  const closing = recordedClosing(account, date);
                  return closing === null ? null : (
                    <Row key={account.account_id} label={account.name} value={formatMoney(closing)} />
                  );
                })}
              </dl>
            </div>
          </section>
        ) : (
        <section aria-label={`Forecast for ${formatIsoDate(date)}`} className="flex flex-col gap-4">
            <h3 className="text-base font-semibold">{formatIsoDate(date)}</h3>
  
            <div>
              <h4 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Opening balance</h4>
              {previous === null ? (
                <p className="text-sm text-muted-foreground">The forecast starts here.</p>
              ) : (
                <dl className="grid grid-cols-[1fr_auto] gap-x-4 text-sm">
                  {groups.map((group) => {
                    const opening = closingOf(group.closings, previous);
                    return opening === null ? null : (
                      <Row key={group.tier} label={TIER_LABEL[group.tier] ?? group.tier} value={formatMoney(opening.p50)} />
                    );
                  })}
                </dl>
              )}
            </div>
  
            <div>
              <h4 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Activity</h4>
              {activities.length === 0 ? (
                <p className="text-sm text-muted-foreground">No activity on this date.</p>
              ) : (
                <ul className="flex flex-col gap-1 text-sm">
                  {activities.map(({ event, accountName }, i) => (
                    <li key={`${event.source_event_id}-${accountName}-${i}`} className="flex flex-wrap justify-between gap-x-4">
                      <span className="min-w-0 break-words">
                        {event.name} <span className="text-muted-foreground">· {accountName}</span>
                      </span>
                      <span className={cn("tabular-nums", event.amount.minor_units < 0 ? "text-loss" : "text-gain")}>
                        {formatSignedMoney(event.amount)}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
  
            <div>
              <h4 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Closing balance</h4>
              <dl className="grid grid-cols-[1fr_auto] gap-x-4 gap-y-1 text-sm">
                {groups.map((group) => {
                  const closing = closingOf(group.closings, date);
                  return closing === null ? null : (
                    <Row key={group.tier} label={TIER_LABEL[group.tier] ?? group.tier} value={closeText(closing)} strong={group.tier === "net"} />
                  );
                })}
                {accountsWithClosing.map(({ account, closing }) => (
                  <Row key={account.account_id ?? account.name} label={account.name} value={closeText(closing!)} />
                ))}
              </dl>
            </div>
          </section>
        )}
      </CardContent>
    </Card>
  );
}

/// The closing balance, with its likely range when the data has one (P10 and P90 from the same data; a
/// collapsed band says nothing extra).
function closeText(band: Band): string {
  const median = formatMoney(band.p50);
  if (band.p10.minor_units === band.p50.minor_units && band.p90.minor_units === band.p50.minor_units) return median;
  return `${median} (P10 ${formatMoney(band.p10)}, P90 ${formatMoney(band.p90)})`;
}

function Row({ label, value, strong }: { label: string; value: string; strong?: boolean }) {
  return (
    <>
      <dt className="min-w-0 break-words text-muted-foreground">{label}</dt>
      <dd className={cn("text-right tabular-nums", strong && "font-semibold")}>{value}</dd>
    </>
  );
}
