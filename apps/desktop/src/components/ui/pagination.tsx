import { ChevronLeft, ChevronRight } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  PAGE_SIZE_OPTIONS,
  type PageSize,
  type Pagination,
} from "@/lib/usePagination";
import { cn } from "@/lib/utils";

/// Pagination controls for a list or table (personal-cfo-4d8.12): the display range,
/// a sticky page-size selector (10 / 25 / 50 / 100), and prev / next. Pair with
/// `usePagination`. `noun` labels the row count (e.g. "transactions").
export function PaginationControls<T>({
  pagination,
  noun = "rows",
  className,
}: {
  pagination: Pagination<T>;
  noun?: string;
  className?: string;
}) {
  const {
    page,
    pageCount,
    pageSize,
    setPage,
    setPageSize,
    total,
    rangeStart,
    rangeEnd,
  } = pagination;

  return (
    <div
      className={cn(
        "flex flex-wrap items-center justify-between gap-3 text-sm text-muted-foreground",
        className,
      )}
    >
      <span className="tabular-nums">
        {total === 0
          ? `No ${noun}`
          : `${rangeStart}–${rangeEnd} of ${total} ${noun}`}
      </span>
      <div className="flex items-center gap-3">
        <label className="flex items-center gap-1.5">
          <span>Show</span>
          <select
            aria-label="Rows per page"
            value={pageSize}
            onChange={(event) => setPageSize(Number(event.target.value) as PageSize)}
            className="h-8 rounded-md border border-input bg-background px-2 text-sm text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          >
            {PAGE_SIZE_OPTIONS.map((size) => (
              <option key={size} value={size}>
                {size}
              </option>
            ))}
          </select>
        </label>
        <div className="flex items-center gap-1">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={page <= 0}
            onClick={() => setPage(page - 1)}
            aria-label="Previous page"
          >
            <ChevronLeft className="size-4" aria-hidden />
          </Button>
          <span className="px-1 tabular-nums text-foreground">
            Page {page + 1} of {pageCount}
          </span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={page >= pageCount - 1}
            onClick={() => setPage(page + 1)}
            aria-label="Next page"
          >
            <ChevronRight className="size-4" aria-hidden />
          </Button>
        </div>
      </div>
    </div>
  );
}
