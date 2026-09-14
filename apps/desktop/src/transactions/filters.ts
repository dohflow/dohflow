import type { TransactionPageInput, TransactionRowDto } from "@/bindings";

/// How the visible transaction list is ordered (feedback 2026-07-03: sortable tables).
export type TransactionSort =
  | "newest"
  | "oldest"
  | "amount_desc"
  | "amount_asc";

/// A running balance is only true in DATE order. Sorted by amount, each row's figure would
/// still be individually correct while the column as a whole read as nonsense — a sequence
/// of balances that never happened in that sequence. So the column goes blank instead
/// (personal-cfo-ttuy).
export const isDateOrdered = (sort: TransactionSort): boolean =>
  sort === "newest" || sort === "oldest";

export const SORT_OPTIONS: { value: TransactionSort; label: string }[] = [
  { value: "newest", label: "Newest first" },
  { value: "oldest", label: "Oldest first" },
  { value: "amount_desc", label: "Largest amount" },
  { value: "amount_asc", label: "Smallest amount" },
];

/// The filterable dimensions of the transactions list (feedback 2026-07-03): free-text
/// search spans the title fields AND notes; the rest narrow by facet. Empty string /
/// null means "no constraint".
export type TransactionFilters = {
  query: string;
  /// Restrict to these accounts; **empty means no constraint**. A set rather than a
  /// single id because the Debt page scopes its embedded list to a multi-account
  /// selection (ADR 0057 §3) — the filter bar's account facet is the one-element case of
  /// this field, not a separate mechanism.
  accountIds: string[];
  /// A category id, or the sentinel `"uncategorized"`, or "" for all.
  categoryId: string;
  tagId: string;
  /// Only transactions confirmed as paying this recurring bill (UUID) — the bill's
  /// linked-payment history (personal-cfo-4d8.24.7.1). Set programmatically (the bill
  /// detail panel), not a filter-bar facet; "" for no constraint. Evaluated server-side
  /// via `confirmed_obligations`, so the client-side reference below cannot mirror it.
  recurringEventId: string;
  /// Inclusive `YYYY-MM-DD` bounds; "" for open-ended.
  from: string;
  to: string;
  unreviewedOnly: boolean;
};

export const EMPTY_FILTERS: TransactionFilters = {
  query: "",
  accountIds: [],
  categoryId: "",
  tagId: "",
  recurringEventId: "",
  from: "",
  to: "",
  unreviewedOnly: false,
};

/// How many facets (beyond search) are constraining the list — drives the filter
/// button's badge so a collapsed panel never hides active narrowing.
export function activeFilterCount(filters: TransactionFilters): number {
  return [
    filters.accountIds.length > 0,
    filters.categoryId !== "",
    filters.tagId !== "",
    filters.from !== "",
    filters.to !== "",
    filters.unreviewedOnly,
  ].filter(Boolean).length;
}

/// Build the server-side page query from the UI's filter / sort / page state
/// (personal-cfo-3fdd.1). Empty-string facets become null ("no constraint"); the
/// SQL in db-worker mirrors the pure functions below, which stay as the reference
/// semantics (and drive the vitest fakes).
export function toTransactionPageInput(
  filters: TransactionFilters,
  sort: TransactionSort,
  page: number,
  pageSize: number,
): TransactionPageInput {
  const query = filters.query.trim();
  return {
    query: query === "" ? null : query,
    account_ids: filters.accountIds,
    // Only ask for balances when they would be true. This also keeps the extra read off
    // every amount-sorted page.
    with_balances: isDateOrdered(sort),
    category_id: filters.categoryId || null,
    tag_id: filters.tagId || null,
    recurring_event_id: filters.recurringEventId || null,
    from_date: filters.from || null,
    to_date: filters.to || null,
    unreviewed_only: filters.unreviewedOnly,
    sort,
    limit: pageSize,
    offset: page * pageSize,
  };
}

/// Case-insensitive match across everything a user might remember about a row:
/// merchant/memo, their note, and the account name (feedback: "search should span
/// the notes as well, not just transaction names"). The SQL in db-worker's
/// `read_transaction_page` mirrors this semantics; the pure functions here remain
/// the reference implementation the vitest fakes page with.
export function matchesQuery(txn: TransactionRowDto, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (needle === "") return true;
  return [txn.memo, txn.counterparty, txn.note, txn.account_name].some(
    (field) => field !== null && field.toLowerCase().includes(needle),
  );
}

/// Apply every active constraint. Dates compare on the row's `YYYY-MM-DD` prefix so a
/// timestamped row lands on its calendar day (both bounds inclusive).
export function filterTransactions(
  transactions: TransactionRowDto[],
  filters: TransactionFilters,
): TransactionRowDto[] {
  return transactions.filter((txn) => {
    if (!matchesQuery(txn, filters.query)) return false;
    if (
      filters.accountIds.length > 0 &&
      !filters.accountIds.includes(txn.account_id)
    )
      return false;
    // CATEGORY IS SERVER-ONLY (ADR 0052 §2). The real filter spans a category's whole
    // SUBTREE and reaches SPLIT LINES, and it lets splits override the parent
    // categorization — none of which a `TransactionRowDto` can answer: it carries
    // neither the taxonomy nor its split lines. This exact-match approximation is kept
    // only so the in-memory fakes stay usable; do not treat it as the reference
    // semantics the way the other facets are.
    if (filters.categoryId === "uncategorized") {
      if (txn.category_id !== null) return false;
    } else if (filters.categoryId && txn.category_id !== filters.categoryId) {
      return false;
    }
    if (filters.tagId && !txn.tag_ids.includes(filters.tagId)) return false;
    const day = txn.occurred_at.slice(0, 10);
    if (filters.from && day < filters.from) return false;
    if (filters.to && day > filters.to) return false;
    if (filters.unreviewedOnly && txn.reviewed) return false;
    return true;
  });
}

/// Order the (already filtered) list. Stable input order (most-recent-first from the
/// read model) is preserved between equal keys by sorting a copy.
export function sortTransactions(
  transactions: TransactionRowDto[],
  sort: TransactionSort,
): TransactionRowDto[] {
  const sorted = [...transactions];
  switch (sort) {
    case "newest":
      sorted.sort((a, b) => b.occurred_at.localeCompare(a.occurred_at));
      break;
    case "oldest":
      sorted.sort((a, b) => a.occurred_at.localeCompare(b.occurred_at));
      break;
    case "amount_desc":
      sorted.sort((a, b) => b.amount.minor_units - a.amount.minor_units);
      break;
    case "amount_asc":
      sorted.sort((a, b) => a.amount.minor_units - b.amount.minor_units);
      break;
  }
  return sorted;
}
