import { AlertTriangle, Loader2, ShieldCheck } from "lucide-react";

import type { CashAvailabilityDto } from "@/bindings";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { formatMoney } from "@/lib/format";
import { cn } from "@/lib/utils";
import { useCashAvailability } from "./useCashAvailability";

/// The household "safe to spend" headline (ADR 0029, personal-cfo-fqbm): net
/// headroom = available cash minus what the next 30 days of bills commit. A
/// below-floor warning fires when that headroom dips under the household
/// minimum-cash floor — a forward-looking signal, not just "are you negative
/// today". Self-contained (owns its own query) so it loads independently of the
/// dashboard forecast.
export function SafeToSpendCard() {
  const { availability, error } = useCashAvailability();

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">Safe to spend</CardTitle>
        <CardDescription>
          Available cash after the next 30 days of bills.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {error ? (
          <p role="alert" className="text-sm text-loss">
            {error}
          </p>
        ) : availability === null ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" aria-hidden />
            Loading…
          </div>
        ) : (
          <SafeToSpend availability={availability} />
        )}
      </CardContent>
    </Card>
  );
}

function SafeToSpend({ availability }: { availability: CashAvailabilityDto }) {
  const { net_available, net_committed, net_headroom, floor, below_floor } =
    availability;
  const headroomNegative = net_headroom.minor_units < 0;
  const hasFloor = floor.minor_units > 0;
  // Negative headroom is the strongest signal (red); merely below the floor is a
  // warning (amber); otherwise the default ink.
  const headroomColor = headroomNegative
    ? "text-loss"
    : below_floor
      ? "text-warning"
      : undefined;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-1">
        <span
          className={cn(
            "text-4xl font-semibold tabular-nums tracking-tight",
            headroomColor,
          )}
        >
          {formatMoney(net_headroom)}
        </span>
        <span className="text-sm tabular-nums text-muted-foreground">
          {formatMoney(net_available)} available − {formatMoney(net_committed)}{" "}
          committed
        </span>
      </div>

      {hasFloor &&
        (below_floor ? (
          <div className="flex items-center gap-2 self-start rounded-md border border-warning/30 bg-warning/10 px-3 py-1.5 text-sm text-warning">
            <AlertTriangle className="size-4 shrink-0" aria-hidden />
            <span>Below your {formatMoney(floor)} floor.</span>
          </div>
        ) : (
          <div className="flex items-center gap-2 self-start rounded-md border border-gain/25 bg-gain/10 px-3 py-1.5 text-sm text-gain">
            <ShieldCheck className="size-4 shrink-0" aria-hidden />
            <span>Above your {formatMoney(floor)} floor.</span>
          </div>
        ))}
    </div>
  );
}
