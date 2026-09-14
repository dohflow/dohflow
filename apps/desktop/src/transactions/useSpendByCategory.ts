import { useQuery } from "@tanstack/react-query";

import { commands, type SpendByCategoryInput } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";
import type { TransactionFilters } from "./filters";

/// How far back the chart looks when the list carries no date bound.
///
/// ADR 0052 §4: the list defaults to NO date bound, which is right for a paged list and
/// wrong for an aggregate — unbounded, the chart would silently total all history on
/// first paint and the user would have no idea what period they were reading. The chart
/// therefore picks a window and, crucially, SAYS which one.
export const DEFAULT_SPEND_DAYS = 90;

/// `YYYY-MM-DD`, `days` before today, in the browser's local calendar.
///
/// Local rather than UTC because the household reads this against its own calendar, and
/// the same reasoning `formatIsoDate` uses: a UTC-midnight conversion shifts the boundary
/// by a day for anyone west of Greenwich.
function isoDaysAgo(days: number): string {
  const d = new Date();
  d.setDate(d.getDate() - days);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function isoToday(): string {
  return isoDaysAgo(0);
}

/// The range the chart is actually charting, given the list's filters.
export type SpendRange = {
  from: string;
  to: string;
  /// True when the range is the chart's own default rather than the user's dates — the
  /// surface says so, so nobody mistakes a 90-day total for all time.
  isDefault: boolean;
};

export function spendRange(filters: TransactionFilters): SpendRange {
  const from = filters.from || isoDaysAgo(DEFAULT_SPEND_DAYS);
  const to = filters.to || isoToday();
  return { from, to, isDefault: filters.from === "" && filters.to === "" };
}

/// Build the aggregate's input from the SAME filter state the list reads (ADR 0052 §2).
///
/// The category facet is deliberately absent: on this surface a category selection is the
/// chart's DRILL LEVEL (`parent_id`), not a filter over the aggregate. Sending it as both
/// would collapse the breakdown to a single bar and constrain the same thing twice.
export function toSpendInput(
  filters: TransactionFilters,
  parentId: string | null,
): SpendByCategoryInput {
  const { from, to } = spendRange(filters);
  const query = filters.query.trim();
  return {
    from,
    to,
    parent_id: parentId,
    query: query === "" ? null : query,
    account_ids: filters.accountIds,
    tag_id: filters.tagId || null,
    unreviewed_only: filters.unreviewedOnly,
  };
}

/// Spend by category for the current filters, at the given drill level
/// (personal-cfo-4d8.27.8.4).
export function useSpendByCategory(
  filters: TransactionFilters,
  parentId: string | null,
) {
  const input = toSpendInput(filters, parentId);
  const query = useQuery({
    queryKey: queryKeys.spendByCategory(input),
    queryFn: () =>
      ipcQuery(
        commands.spendByCategory(input),
        "Could not load your spending breakdown.",
      ),
  });
  return {
    /// `null` until the first read lands — callers treat that as loading rather than
    /// as "no spending", which would be a claim we cannot make yet.
    rows: query.data?.rows ?? null,
    /// What the same query excluded, so the surface can explain the gap between this chart
    /// and the list beside it (personal-cfo-90eg). `null` while loading, for the same
    /// reason as `rows`.
    excluded:
      query.data === undefined
        ? null
        : {
            uncategorizedMinor: query.data.uncategorized_minor,
            transfersMinor: query.data.transfers_minor,
          },
    error: query.error?.message ?? null,
    range: spendRange(filters),
  };
}
