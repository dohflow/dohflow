import type { AccountViewDto, TagViewDto } from "@/bindings";
import { formatIsoDate } from "@/lib/format";

import { EMPTY_FILTERS, type TransactionFilters } from "./filters";

/// One thing narrowing the list, and how to stop narrowing by it.
export type ScopeChip = {
  key: string;
  label: string;
  /// Drill chips are styled to show they came from the chart, so their origin is legible
  /// without a caption.
  fromChart?: boolean;
  clear: () => void;
};

/// Describe the active scope as chips (personal-cfo-3tbn, from the Transactions mock).
///
/// Pure so the description cannot drift from the filters it describes — a strip that said
/// "3 filters" while the query returned something else would be worse than no strip.
///
/// The **drill is one of these chips**, not a separate breadcrumb: the chart's category
/// drill and the bar's category facet are the same `filters.categoryId`, so showing them in
/// two places asks the reader to reconcile one piece of state with itself.
export function scopeChips(
  filters: TransactionFilters,
  set: (next: TransactionFilters) => void,
  ctx: {
    accounts: AccountViewDto[];
    tags: TagViewDto[];
    categoryLabel: (id: string) => string;
    /// Set when the current category came from drilling the chart rather than the facet.
    drilled: boolean;
  },
): ScopeChip[] {
  const chips: ScopeChip[] = [];
  const patch = (p: Partial<TransactionFilters>) => set({ ...filters, ...p });

  if (filters.query.trim() !== "") {
    chips.push({
      key: "query",
      label: `“${filters.query.trim()}”`,
      clear: () => patch({ query: "" }),
    });
  }
  if (filters.accountIds.length > 0) {
    const names = filters.accountIds.map(
      (id) => ctx.accounts.find((a) => a.id === id)?.name ?? "an account",
    );
    chips.push({
      key: "accounts",
      label: names.length === 1 ? (names[0] ?? "") : `${names.length} accounts`,
      clear: () => patch({ accountIds: [] }),
    });
  }
  if (filters.categoryId !== "") {
    chips.push({
      key: "category",
      label:
        filters.categoryId === "uncategorized"
          ? "Uncategorized"
          : ctx.categoryLabel(filters.categoryId),
      fromChart: ctx.drilled,
      clear: () => patch({ categoryId: "" }),
    });
  }
  if (filters.tagId !== "") {
    chips.push({
      key: "tag",
      label: ctx.tags.find((t) => t.id === filters.tagId)?.name ?? "a tag",
      clear: () => patch({ tagId: "" }),
    });
  }
  if (filters.from !== "" || filters.to !== "") {
    const from = filters.from === "" ? "any" : formatIsoDate(filters.from);
    const to = filters.to === "" ? "now" : formatIsoDate(filters.to);
    chips.push({
      key: "dates",
      label: `${from} – ${to}`,
      clear: () => patch({ from: "", to: "" }),
    });
  }
  if (filters.unreviewedOnly) {
    chips.push({
      key: "unreviewed",
      label: "Needs review",
      clear: () => patch({ unreviewedOnly: false }),
    });
  }
  return chips;
}

/// Clearing every chip returns to the unfiltered list — including the drill, since the
/// drill IS a chip. Kept beside `scopeChips` so the two cannot disagree about what "all"
/// means.
export const CLEARED_SCOPE: TransactionFilters = EMPTY_FILTERS;
