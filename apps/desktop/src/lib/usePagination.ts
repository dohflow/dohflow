import { useMemo, useState } from "react";

/// The selectable page sizes; the first is the default (the feedback's "last 10").
export const PAGE_SIZE_OPTIONS = [10, 25, 50, 100] as const;
export type PageSize = (typeof PAGE_SIZE_OPTIONS)[number];

const DEFAULT_PAGE_SIZE: PageSize = PAGE_SIZE_OPTIONS[0];

function isPageSize(value: number): value is PageSize {
  return (PAGE_SIZE_OPTIONS as readonly number[]).includes(value);
}

/// Read the sticky page-size choice for `storageKey`, falling back to the default.
/// Storage can be unavailable (a restricted webview, private mode), so never throw.
function readStoredPageSize(storageKey: string): PageSize {
  try {
    const raw = Number(window.localStorage.getItem(storageKey));
    return Number.isFinite(raw) && isPageSize(raw) ? raw : DEFAULT_PAGE_SIZE;
  } catch {
    return DEFAULT_PAGE_SIZE;
  }
}

export type Pagination<T> = {
  /// The current page's slice of `items`.
  pageItems: T[];
  /// Zero-based current page, clamped to the valid range.
  page: number;
  setPage: (page: number) => void;
  pageSize: PageSize;
  setPageSize: (size: PageSize) => void;
  pageCount: number;
  total: number;
  /// 1-based inclusive display range (`rangeStart`–`rangeEnd` of `total`).
  rangeStart: number;
  rangeEnd: number;
};

/// Client-side pagination with a localStorage-backed (sticky) page size
/// (personal-cfo-4d8.12). The whole list is already in memory at single-household
/// scale, so we slice it here; a windowed offset/limit read path is deferred until a
/// dataset is large enough to need it. `storageKey` namespaces the sticky size per
/// surface (e.g. the transactions list vs the projected-activity table).
export function usePagination<T>(items: T[], storageKey: string): Pagination<T> {
  const [pageSize, setPageSizeState] = useState<PageSize>(() =>
    readStoredPageSize(storageKey),
  );
  const [requestedPage, setPage] = useState(0);

  const total = items.length;
  const pageCount = Math.max(1, Math.ceil(total / pageSize));
  // Clamp so a shrinking list (rows removed, page size raised) never strands us on an
  // empty page past the end.
  const page = Math.min(Math.max(0, requestedPage), pageCount - 1);

  const pageItems = useMemo(
    () => items.slice(page * pageSize, page * pageSize + pageSize),
    [items, page, pageSize],
  );

  const setPageSize = (size: PageSize) => {
    setPageSizeState(size);
    setPage(0);
    try {
      window.localStorage.setItem(storageKey, String(size));
    } catch {
      // Storage unavailable — the choice still applies for this session.
    }
  };

  return {
    pageItems,
    page,
    setPage,
    pageSize,
    setPageSize,
    pageCount,
    total,
    rangeStart: total === 0 ? 0 : page * pageSize + 1,
    rangeEnd: Math.min(total, page * pageSize + pageSize),
  };
}
