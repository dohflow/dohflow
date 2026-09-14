import { type ReactNode } from "react";
import { ArrowDown, ArrowUp, ChevronsUpDown, type LucideIcon } from "lucide-react";

import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

/// One column of a [`DataTable`].
///
/// `header` may be empty for control columns (a select checkbox, an expander) — the
/// column still occupies a slot so the table's structure stays honest.
export type DataTableColumn<T> = {
  /// Stable identity: the React key, and the sort key when `sortable`.
  key: string;
  header: ReactNode;
  cell: (row: T) => ReactNode;
  /// Right-align numerics; everything else reads left.
  align?: "left" | "right";
  /// `min` shrinks the column to its content (control columns, dates, amounts).
  width?: "auto" | "min";
  /// Classes for the body cells.
  ///
  /// Deliberately NOT shared with the header: `cn` is tailwind-merge, so a cell class
  /// like `align-top` would strip the header's own `align-middle` and leave labelled
  /// headers sitting flush to the top while blank control headers stayed centred.
  className?: string;
  /// Classes for the header cell only.
  headerClassName?: string;
  /// Opt-IN. Sorting is never assumed: a table whose rows carry order-dependent values
  /// (running balances) is actively wrong when re-sorted, so a column has to say it is
  /// safe (personal-cfo-yequ).
  sortable?: boolean;
};

export type DataTableSort = { key: string; direction: "asc" | "desc" };

/// The shared table primitive (personal-cfo-4d8.27.4.2).
///
/// Every list surface used to hand-roll its own header, its own empty state, and its own
/// `colSpan` arithmetic for expander rows — and that arithmetic silently breaks every
/// time a column is added, which is exactly the kind of drift that made the app stop
/// looking like one product (FRONTEND.md). Here the table owns its own structure: the
/// header comes from the column defs, the expander row spans `columns.length` by
/// construction, and the four states are one prop instead of four bespoke branches.
///
/// Deliberately NOT owned here: data fetching, filtering, and pagination *state*. Callers
/// paginate very differently (Transactions pages on the server, Projected Activity in the
/// browser), so pagination is rendered through `footer` rather than assumed.
export function DataTable<T>({
  columns,
  rows,
  rowKey,
  status = "ready",
  error,
  empty,
  expandedContent,
  rowProps,
  leadingRow,
  trailingRow,
  sort,
  onSortChange,
  minWidth,
  footer,
  className,
  label,
}: {
  columns: DataTableColumn<T>[];
  rows: T[];
  rowKey: (row: T) => string;
  /// `loading` shows skeleton rows in the real column layout; `error` replaces the body.
  /// An EMPTY table is `ready` with no rows — emptiness is data, not a status.
  status?: "loading" | "error" | "ready";
  error?: string | null;
  /// Rendered when `status` is `ready` and there are no rows.
  empty?: { icon: LucideIcon; title: string; description?: string; action?: ReactNode };
  /// A row's expanded detail. Returning `null` renders no expander row, so a caller can
  /// decide per row. The primitive owns the `colSpan`.
  expandedContent?: (row: T) => ReactNode | null;
  /// Per-row attributes (e.g. `data-state="selected"`, a left-border accent).
  rowProps?: (row: T) => { className?: string; "data-state"?: string };
  /// Pinned rows rendered before the data (Account Detail's projected "Upcoming"
  /// block). Raw rows, because what is pinned there is real multi-cell content.
  leadingRow?: ReactNode;
  /// A pinned note rendered AFTER the data rows.
  ///
  /// Projected Activity keeps its TODAY anchor *in* its rows — those cells are the same
  /// running balances every other row renders, so they have to come from the column
  /// defs — which means `rows.length` is never 0 there and `empty` can never fire. Its
  /// "the filter matched nothing" notice therefore has to sit below the rows instead.
  ///
  /// Unlike `leadingRow`, this is full-width content and the primitive owns the
  /// `colSpan` (personal-cfo-wxy7).
  trailingRow?: ReactNode;
  sort?: DataTableSort;
  onSortChange?: (sort: DataTableSort) => void;
  minWidth?: string;
  /// Rendered under the table — the pagination slot.
  footer?: ReactNode;
  className?: string;
  /// Accessible name for the table.
  label?: string;
}) {
  const span = columns.length;

  function headerCell(column: DataTableColumn<T>) {
    const alignment = column.align === "right" ? "text-right" : undefined;
    const width = column.width === "min" ? "w-0" : undefined;
    if (!column.sortable || !onSortChange) {
      return (
        <TableHead
          key={column.key}
          className={cn(alignment, width, column.headerClassName)}
        >
          {column.header}
        </TableHead>
      );
    }
    const active = sort?.key === column.key;
    const next: DataTableSort = {
      key: column.key,
      direction: active && sort?.direction === "asc" ? "desc" : "asc",
    };
    const Icon = active
      ? sort?.direction === "asc"
        ? ArrowUp
        : ArrowDown
      : ChevronsUpDown;
    return (
      <TableHead
        key={column.key}
        className={cn(alignment, width, column.headerClassName)}
        aria-sort={
          active ? (sort?.direction === "asc" ? "ascending" : "descending") : "none"
        }
      >
        <button
          type="button"
          onClick={() => onSortChange(next)}
          className={cn(
            "inline-flex items-center gap-1 rounded-sm font-medium transition-colors hover:text-foreground",
            column.align === "right" && "flex-row-reverse",
            active ? "text-foreground" : "text-muted-foreground",
          )}
        >
          {column.header}
          <Icon aria-hidden className="size-3" />
        </button>
      </TableHead>
    );
  }

  return (
    <div className={className}>
      <Table className={minWidth} aria-label={label}>
        {/* Always a header. `hideHeader` existed for the Money Inbox, which was examined
            in personal-cfo-9krd and deliberately does NOT migrate — so the slot had no
            consumer and was deleted rather than kept as decoration (surface-audit.md). */}
        <TableHeader>
          <TableRow className="text-xs text-muted-foreground hover:bg-transparent">
            {columns.map(headerCell)}
          </TableRow>
        </TableHeader>
        <TableBody>
          {leadingRow}
          {status === "loading" &&
            // Skeletons in the REAL column layout, so the table does not reflow when
            // the data lands.
            Array.from({ length: 3 }, (_, index) => (
              <TableRow key={`skeleton-${index}`} className="hover:bg-transparent">
                {columns.map((column) => (
                  <TableCell key={column.key}>
                    <Skeleton className="h-4 w-full" />
                  </TableCell>
                ))}
              </TableRow>
            ))}
          {status === "error" && (
            <TableRow className="hover:bg-transparent">
              <TableCell colSpan={span}>
                <p role="alert" className="py-6 text-center text-sm text-loss">
                  {error ?? "Something went wrong loading this."}
                </p>
              </TableCell>
            </TableRow>
          )}
          {status === "ready" && rows.length === 0 && (
            <TableRow className="hover:bg-transparent">
              <TableCell colSpan={span}>
                {/* A caller that forgets `empty` still gets a sentence rather than a
                    header floating over blank space — which would be exactly the
                    fourth treatment this primitive exists to prevent. */}
                {empty ? (
                  <EmptyState {...empty} />
                ) : (
                  <p className="py-6 text-center text-sm text-muted-foreground">
                    Nothing to show.
                  </p>
                )}
              </TableCell>
            </TableRow>
          )}
          {status === "ready" &&
            rows.flatMap((row) => {
              const key = rowKey(row);
              const extra = rowProps?.(row);
              const detail = expandedContent?.(row) ?? null;
              return [
                <TableRow key={key} {...extra}>
                  {columns.map((column) => (
                    <TableCell
                      key={column.key}
                      className={cn(
                        column.align === "right" && "text-right",
                        column.width === "min" && "w-0",
                        column.className,
                      )}
                    >
                      {column.cell(row)}
                    </TableCell>
                  ))}
                </TableRow>,
                detail === null ? null : (
                  <TableRow key={`${key}-detail`} className="hover:bg-transparent">
                    {/* The primitive owns the span, so adding a column can never leave a
                        detail row misaligned. */}
                    <TableCell colSpan={span} className="p-0">
                      {detail}
                    </TableCell>
                  </TableRow>
                ),
              ];
            })}
          {/* Pinned like `leadingRow`, so it renders whatever the status — and, like the
              expander row, it never carries a hand-maintained span. */}
          {trailingRow ? (
            <TableRow className="hover:bg-transparent">
              <TableCell colSpan={span}>{trailingRow}</TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>
      {footer}
    </div>
  );
}
