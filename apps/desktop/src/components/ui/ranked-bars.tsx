import { type ReactNode } from "react";
import { ChevronRight, type LucideIcon } from "lucide-react";

import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import { formatMoney } from "@/lib/format";
import { cn } from "@/lib/utils";

/// One bar: a category (or any nominal thing) and the money against it.
export type RankedBar = {
  /// Stable identity — the React key and what `onSelect` hands back.
  key: string;
  label: string;
  /// Minor units, **positive for money spent**.
  value: number;
  /// Whether selecting this bar goes somewhere (a drill level below it). Bars without
  /// it are still selectable; they just carry no affordance.
  drillable?: boolean;
};

/// Ranked horizontal bars (personal-cfo-4d8.27.8.4, built to
/// `docs/design-system/dataviz.md`).
///
/// The form the method picks for "compare magnitude across nominal categories": length is
/// compared precisely where area is not, long category names fit on the left rather than
/// being truncated inside a cell, and every value can be direct-labelled at the tip.
///
/// Two rules here are load-bearing rather than stylistic:
///
/// - **Every bar is the same colour** (`--chart-1`). Shading bars by their value would
///   re-encode length as hue, spend the identity channel on what length already shows,
///   and fail the palette gates by construction (ADR 0054). One series also means no
///   legend — the caller's heading names what is plotted.
/// - **Every value is direct-labelled.** A tooltip must never be the only way to read a
///   number, so there is no hover-gated value here at all and no table-view toggle is
///   needed to make one reachable.
export function RankedBars<T extends RankedBar>({
  bars,
  currency,
  status = "ready",
  error,
  empty,
  onSelect,
  label,
  footer,
  className,
}: {
  bars: T[];
  /// ISO currency for the value labels. The aggregate is scoped to one currency
  /// (ADR 0052 §4), so the caller states which — this only formats.
  currency: string;
  /// `loading` shows skeleton bars in the real layout. An EMPTY chart is `ready` with no
  /// bars — emptiness is data, not a status. Same contract as `DataTable`.
  status?: "loading" | "error" | "ready";
  error?: string | null;
  empty?: { icon: LucideIcon; title: string; description?: string };
  /// Selecting a bar. Bars are real buttons, so this is keyboard-reachable.
  onSelect?: (bar: T) => void;
  /// Accessible name for the figure — say what is plotted and over what.
  label: string;
  footer?: ReactNode;
  className?: string;
}) {
  // The scale is the largest bar, so the longest bar always fills the track. Guarded
  // against a zero/negative max so an all-zero range cannot divide by zero.
  const max = Math.max(0, ...bars.map((bar) => bar.value));

  return (
    <figure className={cn("flex flex-col gap-2", className)} aria-label={label}>
      {status === "loading" && (
        // Skeletons in the real row layout, so nothing reflows when the data lands.
        <div className="flex flex-col gap-2">
          {Array.from({ length: 5 }, (_, index) => (
            <div key={index} className="flex items-center gap-3">
              <Skeleton className="h-4 w-28 shrink-0" />
              <Skeleton className="h-4 flex-1" />
            </div>
          ))}
        </div>
      )}

      {status === "error" && (
        <p role="alert" className="py-6 text-center text-sm text-loss">
          {error ?? "Something went wrong loading this."}
        </p>
      )}

      {status === "ready" && bars.length === 0 && (
        <>
          {empty ? (
            <EmptyState {...empty} />
          ) : (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Nothing to show.
            </p>
          )}
        </>
      )}

      {status === "ready" &&
        bars.map((bar) => {
          // A zero-value bar still renders its row and label; it just has no length.
          const pct = max > 0 ? Math.max(0, (bar.value / max) * 100) : 0;
          const amount = formatMoney({ minor_units: bar.value, currency });
          const interactive = onSelect !== undefined;
          const Row = interactive ? "button" : "div";
          return (
            <Row
              key={bar.key}
              {...(interactive
                ? {
                    type: "button" as const,
                    onClick: () => onSelect(bar),
                    // The name carries the value too, so the bar does not depend on
                    // sighted comparison of lengths.
                    "aria-label": `${bar.label}, ${amount}${
                      bar.drillable ? " — open its subcategories" : ""
                    }`,
                  }
                : {})}
              className={cn(
                "group flex w-full items-center gap-3 rounded-md px-1.5 py-1 text-left",
                interactive &&
                  "transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              )}
            >
              <span className="flex w-32 shrink-0 items-center gap-1 truncate text-sm">
                <span className="truncate">{bar.label}</span>
                {bar.drillable && (
                  <ChevronRight
                    aria-hidden
                    className="size-3 shrink-0 text-muted-foreground"
                  />
                )}
              </span>
              {/* The track is the surface, not a mark — no border, so the bar is
                  separated by space rather than by a stroke. */}
              <span className="flex min-w-0 flex-1 items-center gap-2">
                <span
                  aria-hidden
                  className="h-4 min-w-px rounded-r-[4px] bg-[var(--chart-1)]"
                  style={{ width: `${pct}%` }}
                />
                <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
                  {amount}
                </span>
              </span>
            </Row>
          );
        })}

      {footer}
    </figure>
  );
}
