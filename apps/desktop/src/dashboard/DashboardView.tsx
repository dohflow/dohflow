import type { ReactNode } from "react";
import {
  ArrowDownCircle,
  ArrowUpCircle,
  ArrowUpRight,
  CalendarClock,
  Loader2,
} from "lucide-react";

import type { ForecastDayDto, ForecastEventDto, MoneyDto } from "@/bindings";
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
import { cn } from "@/lib/utils";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/PageHeader";
import { FutureCashChart } from "@/future-cash/FutureCashChart";
import { SafeToSpendCard } from "./SafeToSpendCard";
import { ReadinessCard } from "./ReadinessCard";
import {
  DASHBOARD_HORIZON_DAYS,
  DASHBOARD_UPCOMING_DAYS,
  useDashboard,
} from "./useDashboard";

/// A forecast event paired with the calendar day it lands on (the wire keeps the
/// date on the day, not the event).
type DatedEvent = ForecastEventDto & { date: string };

const BILL_KINDS = new Set(["recurring_bill", "loan_payment"]);

function sumMinorUnits(events: DatedEvent[]): number {
  return events.reduce((total, event) => total + event.amount.minor_units, 0);
}

/// The dashboard: liquid cash today, the 90-day Future Cash chart, and the
/// upcoming income/bills — all derived from a single deterministic forecast
/// query (plan §18.2, personal-cfo-1vd7). The chart mirrors the Future Cash tab
/// and is clickable: `onOpenCashFlow` navigates there (personal-cfo-d5qy).
export function DashboardView({
  onOpenCashFlow,
}: {
  onOpenCashFlow?: () => void;
} = {}) {
  const { forecast, error } = useDashboard();

  if (error) {
    return (
      <div className="mx-auto w-full max-w-4xl">
        <p role="alert" className="text-sm text-loss">
          {error}
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
  // The chart spans the full horizon; the "upcoming" lists stay a near-term
  // 30-day window (personal-cfo-d5qy).
  const upcomingEvents: DatedEvent[] = days
    .slice(0, DASHBOARD_UPCOMING_DAYS)
    .flatMap((day) => day.events.map((event) => ({ ...event, date: day.date })));
  const income = upcomingEvents.filter((event) => event.kind === "income");
  const bills = upcomingEvents.filter((event) => BILL_KINDS.has(event.kind));

  const incomeTotal: MoneyDto = { minor_units: sumMinorUnits(income), currency };
  const billsTotal: MoneyDto = { minor_units: sumMinorUnits(bills), currency };

  // The lowest projected balance over the horizon — the "will I dip too low"
  // signal (a safe-to-spend proxy). Falls back to today's balance on an empty
  // horizon.
  const lowest = days.reduce<ForecastDayDto | null>((min, day) => {
    if (min === null) return day;
    return day.closing.p50.minor_units < min.closing.p50.minor_units
      ? day
      : min;
  }, null);
  const endBalance = days.at(-1)?.closing.p50 ?? starting_balance;

  const today = new Intl.DateTimeFormat(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  }).format(new Date());

  return (
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-6">
      <PageHeader title="Dashboard" subtitle={today} />

      <SafeToSpendCard />

      <ReadinessCard />

      <div className="grid gap-4 sm:grid-cols-3">
        <StatCard label="Liquid cash today" value={formatMoney(starting_balance)} />
        <StatCard
          label="Lowest projected"
          value={formatMoney(lowest?.closing.p50 ?? starting_balance)}
          caption={lowest ? `on ${formatIsoDate(lowest.date)}` : undefined}
          negative={(lowest?.closing.p50.minor_units ?? 0) < 0}
        />
        <StatCard
          label={`In ${DASHBOARD_HORIZON_DAYS} days`}
          value={formatMoney(endBalance)}
        />
      </div>

      <Card>
        <CardHeader className="flex flex-row items-start justify-between space-y-0">
          <div className="flex flex-col gap-1.5">
            <CardTitle>Cash Flow · next {DASHBOARD_HORIZON_DAYS} days</CardTitle>
            <CardDescription>
              Projected liquid-cash balance, folding in upcoming income and bills.
            </CardDescription>
          </div>
          <span className="flex items-center gap-1 text-xs font-medium text-muted-foreground">
            Open Cash Flow
            <ArrowUpRight className="size-3.5" aria-hidden />
          </span>
        </CardHeader>
        <CardContent>
          <button
            type="button"
            onClick={() => onOpenCashFlow?.()}
            aria-label="Open the Cash Flow tab"
            className="block w-full rounded-md transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
          >
            <FutureCashChart days={days} currency={currency} />
          </button>
        </CardContent>
      </Card>

      <div className="grid gap-4 md:grid-cols-2">
        <UpcomingList
          title="Upcoming income"
          icon={<ArrowUpCircle className="size-4 text-gain" aria-hidden />}
          total={incomeTotal}
          events={income}
          emptyText="No income expected in the next 30 days."
        />
        <UpcomingList
          title="Upcoming bills"
          icon={<ArrowDownCircle className="size-4 text-loss" aria-hidden />}
          total={billsTotal}
          events={bills}
          emptyText="No bills due in the next 30 days."
        />
      </div>
    </div>
  );
}

function StatCard({
  label,
  value,
  caption,
  negative = false,
}: {
  label: string;
  value: string;
  caption?: string;
  negative?: boolean;
}) {
  return (
    <Card>
      <CardContent className="flex flex-col gap-1.5 p-6">
        <span className="truncate text-xs font-medium uppercase tracking-wide text-muted-foreground">
          {label}
        </span>
        <span
          className={cn(
            "text-2xl font-semibold tabular-nums tracking-tight",
            negative && "text-loss",
          )}
        >
          {value}
        </span>
        {caption && (
          <span className="text-xs text-muted-foreground">{caption}</span>
        )}
      </CardContent>
    </Card>
  );
}

function UpcomingList({
  title,
  icon,
  total,
  events,
  emptyText,
}: {
  title: string;
  icon: ReactNode;
  total: MoneyDto;
  events: DatedEvent[];
  emptyText: string;
}) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-3">
        <CardTitle className="flex items-center gap-2 text-sm">
          {icon}
          {title}
        </CardTitle>
        {events.length > 0 && (
          <span
            className={cn(
              "text-base font-semibold tabular-nums",
              signedAmountClass(total),
            )}
          >
            {formatSignedMoney(total)}
          </span>
        )}
      </CardHeader>
      <CardContent className="p-0">
        {events.length === 0 ? (
          <EmptyState icon={CalendarClock} title={emptyText} className="pb-8 pt-2" />
        ) : (
          <ul>
            {events.map((event, index) => (
              <li
                key={`${event.source_event_id}-${event.date}-${index}`}
                className="flex items-center justify-between border-t px-6 py-2.5 text-sm transition-colors hover:bg-muted/30"
              >
                <div>
                  <div className="font-medium">{event.name || "—"}</div>
                  <div className="text-xs text-muted-foreground">
                    {formatIsoDate(event.date)}
                  </div>
                </div>
                <span
                  className={cn(
                    "tabular-nums",
                    signedAmountClass(event.amount),
                  )}
                >
                  {formatSignedMoney(event.amount)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}
