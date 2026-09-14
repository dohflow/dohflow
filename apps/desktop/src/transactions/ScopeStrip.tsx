import { BarChart3, X } from "lucide-react";

import { formatMoney } from "@/lib/format";
import { cn } from "@/lib/utils";

import type { ScopeChip } from "./scopeChips";

/// The one place the list's scope is stated (personal-cfo-3tbn, ADR 0052 §2).
///
/// **Sticky**, because categorizing is a long scroll and a scope that scrolls away is how a
/// row gets mis-filed against a filter the reader has forgotten is on.
///
/// Carries the counts as well as the chips: rows shown of the total, and the filtered
/// outflow and inflow. Those are computed off the same filtered set the chart totals, so the
/// two are *checkable* against each other rather than merely asserted to agree.
export function ScopeStrip({
  chips,
  shown,
  total,
  outflowMinor,
  inflowMinor,
  currency,
  onClearAll,
}: {
  chips: ScopeChip[];
  shown: number;
  total: number;
  outflowMinor: number;
  inflowMinor: number;
  currency: string;
  onClearAll: () => void;
}) {
  const money = (minor: number) =>
    formatMoney({ minor_units: Math.abs(minor), currency });

  return (
    <div
      className={cn(
        "sticky top-0 z-20 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-lg border bg-card px-3 py-2 shadow-sm",
        // A drill tints the border, so an active chart drill is visible even before the
        // chips are read.
        chips.some((c) => c.fromChart) && "border-primary/40",
      )}
    >
      <div className="flex flex-1 flex-wrap items-center gap-2">
        {chips.length === 0 ? (
          <span className="text-sm text-muted-foreground">
            Showing everything, newest first.
          </span>
        ) : (
          chips.map((chip) => (
            <span
              key={chip.key}
              className={cn(
                "flex items-center gap-1 rounded-full border py-0.5 pl-2.5 pr-1 text-xs",
                chip.fromChart
                  ? "border-primary/40 bg-primary/10 text-foreground"
                  : "border-border bg-muted/60 text-muted-foreground",
              )}
            >
              {chip.fromChart && (
                <BarChart3 className="size-3 shrink-0 text-primary" aria-hidden />
              )}
              {chip.label}
              <button
                type="button"
                onClick={chip.clear}
                aria-label={`Remove ${chip.label}`}
                className="rounded-full p-0.5 hover:bg-muted hover:text-foreground"
              >
                <X className="size-3" aria-hidden />
              </button>
            </span>
          ))
        )}
        {chips.length > 1 && (
          <button
            type="button"
            onClick={onClearAll}
            className="text-xs text-muted-foreground underline underline-offset-2 hover:text-foreground"
          >
            Clear all
          </button>
        )}
      </div>

      <div className="flex items-center gap-3 text-xs tabular-nums text-muted-foreground">
        <span>
          {shown === total
            ? `${total} transaction${total === 1 ? "" : "s"}`
            : `${shown} of ${total} transactions`}
        </span>
        <span className="text-loss">{money(outflowMinor)} out</span>
        <span className="text-gain">{money(inflowMinor)} in</span>
      </div>
    </div>
  );
}

