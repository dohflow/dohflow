import { useEffect, useState } from "react";

import { PAGE_SIZE_OPTIONS, type PageSize, type Pagination } from "@/lib/usePagination";
import {
  toTransactionPageInput,
  type TransactionFilters,
  type TransactionSort,
} from "./filters";
import { useTransactionPage } from "./useTransactions";

const DEFAULT_PAGE_SIZE: PageSize = PAGE_SIZE_OPTIONS[0];

function isPageSize(value: number): value is PageSize {
  return (PAGE_SIZE_OPTIONS as readonly number[]).includes(value);
}

/// Read the sticky page-size choice for `storageKey`, falling back to the default.
/// Mirrors `usePagination`'s (private) storage behaviour — same key format, never
/// throws when storage is unavailable.
function readStoredPageSize(storageKey: string): PageSize {
  try {
    const raw = Number(window.localStorage.getItem(storageKey));
    return Number.isFinite(raw) && isPageSize(raw) ? raw : DEFAULT_PAGE_SIZE;
  } catch {
    return DEFAULT_PAGE_SIZE;
  }
}

/// Server-paged transactions (personal-cfo-3fdd.1): owns the page / page-size
/// state, issues the `transactionPage` query keyed on (filters, sort, page,
/// pageSize), and shapes the result as a `Pagination` so the shared
/// `PaginationControls` renders it unchanged — but with `total` coming from the
/// server DTO instead of an in-memory list. A filter or sort change restarts at
/// the first page; a window stranded past a shrunken dataset (rows deleted on the
/// last page) snaps back once the fresh total arrives.
export function usePagedTransactions(
  filters: TransactionFilters,
  sort: TransactionSort,
  storageKey: string,
) {
  const [pageSize, setPageSizeState] = useState<PageSize>(() =>
    readStoredPageSize(storageKey),
  );
  const [page, setPage] = useState(0);

  const query = useTransactionPage(
    toTransactionPageInput(filters, sort, page, pageSize),
  );

  // A different filter/sort context starts from the first page again. The key is
  // a stable serialization of the state objects (fixed field order).
  const contextKey = JSON.stringify([filters, sort]);
  useEffect(() => {
    setPage(0);
  }, [contextKey]);

  const total = query.data?.total;
  const knownTotal = total ?? 0;
  const pageCount = Math.max(1, Math.ceil(knownTotal / pageSize));
  // Snap back when the server says the window ran past the end: the stale offset
  // returns an empty page plus the real total, and this refetches the last valid
  // page (one extra round trip, only in the shrink case).
  useEffect(() => {
    if (total !== undefined && page > pageCount - 1) setPage(pageCount - 1);
  }, [total, page, pageCount]);

  const setPageSize = (size: PageSize) => {
    setPageSizeState(size);
    setPage(0);
    try {
      window.localStorage.setItem(storageKey, String(size));
    } catch {
      // Storage unavailable — the choice still applies for this session.
    }
  };

  const pagination: Pagination<never> = {
    pageItems: [],
    page,
    setPage,
    pageSize,
    setPageSize,
    pageCount,
    total: knownTotal,
    rangeStart: knownTotal === 0 ? 0 : page * pageSize + 1,
    rangeEnd: Math.min(knownTotal, page * pageSize + pageSize),
  };

  return {
    /// The current page's rows (empty while the first page loads).
    rows: query.data?.rows ?? [],
    /// Total matches across all pages; `undefined` until the first page lands.
    total,
    error: query.error?.message ?? null,
    pagination,
  };
}
