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
import { useCompactLayout } from "@/lib/compact";
import { NativeSelect } from "@/components/ui/native-select";
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
  errorAction,
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
  phoneLayout,
}: {
  columns: DataTableColumn<T>[];
  rows: T[];
  rowKey: (row: T) => string;
  /// `loading` shows skeleton rows in the real column layout; `error` replaces the body.
  /// An EMPTY table is `ready` with no rows — emptiness is data, not a status.
  status?: "loading" | "error" | "ready";
  error?: string | null;
  /// The one action that answers the error (a Retry), shown with it.
  errorAction?: ReactNode;
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
  /// How the table reads on a phone (personal-cfo-i93n3.27 slice F). Omitted: it stays the horizontal
  /// table everywhere, as before (a table opts in; `.28`'s Accounts and Activity will). `"stacked"`: on a
  /// compact screen (`useCompactLayout()`: a different element tree, not a restyled table) each row is a
  /// card of labeled values in column order, with nothing clipped and no horizontal scroll. A table with a
  /// `leadingRow` (raw table cells) keeps the table layout.
  phoneLayout?: "stacked";
}) {
  const span = columns.length;
  const compact = useCompactLayout();
  if (phoneLayout === "stacked" && compact && leadingRow === undefined) {
    return (
      <StackedRows
        columns={columns}
        rows={rows}
        rowKey={rowKey}
        status={status}
        error={error}
        errorAction={errorAction}
        empty={empty}
        expandedContent={expandedContent}
        rowProps={rowProps}
        trailingRow={trailingRow}
        sort={sort}
        onSortChange={onSortChange}
        footer={footer}
        className={className}
        label={label}
      />
    );
  }

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
                <div className="flex flex-col items-center gap-3 py-6">
                  <p role="alert" className="text-center text-sm text-loss">
                    {error ?? "Something went wrong loading this."}
                  </p>
                  {errorAction}
                </div>
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

/// The phone presentation of a [`DataTable`] (`phoneLayout="stacked"`): a list of rows, each a card.
///
/// Reading order is label then value, in column order, as a real description list (`dl`), so VoiceOver reads
/// "Date, Nov 17, 2026" rather than a bare cell. The first column is the row's title (its label is read but not
/// drawn); a column with an empty header (a control column) has no label. Values wrap, never clip, and money
/// stays complete. When a column is `sortable` the headers are gone, so a "Sort by" control and a direction
/// button above the list keep sorting reachable.
function StackedRows<T>({
  columns,
  rows,
  rowKey,
  status,
  error,
  errorAction,
  empty,
  expandedContent,
  rowProps,
  trailingRow,
  sort,
  onSortChange,
  footer,
  className,
  label,
}: {
  columns: DataTableColumn<T>[];
  rows: T[];
  rowKey: (row: T) => string;
  status: "loading" | "error" | "ready";
  error?: string | null;
  errorAction?: ReactNode;
  empty?: { icon: LucideIcon; title: string; description?: string; action?: ReactNode };
  expandedContent?: (row: T) => ReactNode | null;
  rowProps?: (row: T) => { className?: string; "data-state"?: string };
  trailingRow?: ReactNode;
  sort?: DataTableSort;
  onSortChange?: (sort: DataTableSort) => void;
  footer?: ReactNode;
  className?: string;
  label?: string;
}) {
  const sortable = columns.filter((column) => column.sortable && typeof column.header === "string");
  const [title, ...rest] = columns;

  return (
    <div className={className}>
      {onSortChange && sortable.length > 0 && (
        <div className="flex items-center gap-2 px-4 pb-2">
          <label className="sr-only" htmlFor="stacked-sort-by">
            Sort by
          </label>
          <NativeSelect
            id="stacked-sort-by"
            size="sm"
            className="min-w-0 flex-1"
            value={sort?.key ?? ""}
            onChange={(event) =>
              onSortChange({ key: event.target.value, direction: sort?.direction ?? "asc" })
            }
          >
            {sort === undefined && <option value="">Sort by…</option>}
            {sortable.map((column) => (
              <option key={column.key} value={column.key}>
                {column.header as string}
              </option>
            ))}
          </NativeSelect>
          <button
            type="button"
            disabled={sort === undefined}
            aria-label={sort?.direction === "desc" ? "Sorted descending; sort ascending" : "Sorted ascending; sort descending"}
            onClick={() => sort && onSortChange({ key: sort.key, direction: sort.direction === "asc" ? "desc" : "asc" })}
            className="inline-flex shrink-0 items-center gap-1 rounded-md border px-3 text-sm text-muted-foreground"
          >
            {sort?.direction === "desc" ? <ArrowDown aria-hidden className="size-4" /> : <ArrowUp aria-hidden className="size-4" />}
          </button>
        </div>
      )}
      <ul role="list" aria-label={label} className="flex flex-col">
        {status === "loading" &&
          Array.from({ length: 3 }, (_, index) => (
            <li key={`skeleton-${index}`} className="border-b px-4 py-3">
              <Skeleton className="mb-2 h-4 w-2/3" />
              <Skeleton className="h-4 w-full" />
            </li>
          ))}
        {status === "error" && (
          <li className="px-4">
            <div className="flex flex-col items-center gap-3 py-6">
              <p role="alert" className="text-center text-sm text-loss">
                {error ?? "Something went wrong loading this."}
              </p>
              {errorAction}
            </div>
          </li>
        )}
        {status === "ready" && rows.length === 0 && (
          <li className="px-4">
            {empty ? (
              <EmptyState {...empty} />
            ) : (
              <p className="py-6 text-center text-sm text-muted-foreground">Nothing to show.</p>
            )}
          </li>
        )}
        {status === "ready" &&
          rows.map((row) => {
            const extra = rowProps?.(row);
            const detail = expandedContent?.(row) ?? null;
            return (
              <li
                key={rowKey(row)}
                data-state={extra?.["data-state"]}
                className={cn("flex flex-col border-b", extra?.className)}
              >
                <div className="flex flex-col gap-1.5 px-4 py-3">
                  {title && (
                    <div className="min-w-0 font-medium [overflow-wrap:anywhere]">
                      {typeof title.header === "string" && <span className="sr-only">{title.header}: </span>}
                      {title.cell(row)}
                    </div>
                  )}
                  <dl className="flex flex-col gap-1 text-sm">
                    {rest.map((column) => (
                      <div key={column.key} className="flex items-baseline justify-between gap-4">
                        {typeof column.header === "string" && column.header !== "" ? (
                          <dt className="min-w-0 text-muted-foreground [overflow-wrap:anywhere]">{column.header}</dt>
                        ) : (
                          <dt className="sr-only">{column.key}</dt>
                        )}
                        <dd className={cn("min-w-0 text-right tabular-nums [overflow-wrap:anywhere]", column.className)}>
                          {column.cell(row)}
                        </dd>
                      </div>
                    ))}
                  </dl>
                </div>
                {detail}
              </li>
            );
          })}
        {trailingRow ? <li className="px-4 py-3">{trailingRow}</li> : null}
      </ul>
      {footer}
    </div>
  );
}
