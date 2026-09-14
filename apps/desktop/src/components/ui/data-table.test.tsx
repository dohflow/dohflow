import { fireEvent, render, screen, within } from "@testing-library/react";
import { Inbox } from "lucide-react";

import { DataTable, type DataTableColumn } from "./data-table";

type Row = { id: string; name: string; amount: number };

const ROWS: Row[] = [
  { id: "a", name: "Alpha", amount: 100 },
  { id: "b", name: "Beta", amount: -250 },
];

const COLUMNS: DataTableColumn<Row>[] = [
  { key: "name", header: "Name", cell: (r) => r.name, sortable: true },
  {
    key: "amount",
    header: "Amount",
    cell: (r) => r.amount,
    align: "right",
    width: "min",
  },
];

describe("DataTable (personal-cfo-4d8.27.4.2)", () => {
  it("renders a header from the column defs and a row per item", () => {
    render(<DataTable columns={COLUMNS} rows={ROWS} rowKey={(r) => r.id} />);
    expect(
      screen.getAllByRole("columnheader").map((h) => h.textContent),
    ).toEqual(["Name", "Amount"]);
    expect(screen.getAllByRole("row")).toHaveLength(3); // header + 2
    expect(screen.getByText("Alpha")).toBeInTheDocument();
  });

  it("spans an expanded row across every column, whatever the column count", () => {
    // This is the invariant the primitive exists for: hand-rolled tables carried a
    // "colSpan MUST equal N" comment that silently broke each time a column was added.
    const columns = [...COLUMNS, { key: "x", header: "X", cell: () => "x" }];
    render(
      <DataTable
        columns={columns}
        rows={ROWS}
        rowKey={(r) => r.id}
        expandedContent={(r) => (r.id === "a" ? <p>detail for {r.name}</p> : null)}
      />,
    );
    const detail = screen.getByText(/detail for Alpha/);
    expect(detail.closest("td")).toHaveAttribute("colspan", String(columns.length));
    // Only the row that returned content gets a detail row.
    expect(screen.queryByText(/detail for Beta/)).not.toBeInTheDocument();
  });

  it("sorts only on columns that opt in", () => {
    const onSortChange = vi.fn();
    render(
      <DataTable
        columns={COLUMNS}
        rows={ROWS}
        rowKey={(r) => r.id}
        sort={{ key: "name", direction: "asc" }}
        onSortChange={onSortChange}
      />,
    );
    // `amount` never declared itself sortable — a table with order-dependent values
    // (running balances) is actively wrong when re-sorted, so this must not be a button.
    const headers = screen.getAllByRole("columnheader");
    expect(within(headers[1]!).queryByRole("button")).toBeNull();

    fireEvent.click(within(headers[0]!).getByRole("button"));
    // Already ascending on this key → the click flips it.
    expect(onSortChange).toHaveBeenCalledWith({ key: "name", direction: "desc" });
    expect(headers[0]).toHaveAttribute("aria-sort", "ascending");
  });

  it("never sorts when the caller supplies no handler", () => {
    render(<DataTable columns={COLUMNS} rows={ROWS} rowKey={(r) => r.id} />);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("shows skeletons in the real column layout while loading", () => {
    const { container } = render(
      <DataTable columns={COLUMNS} rows={[]} rowKey={(r) => r.id} status="loading" />,
    );
    // Skeleton cells match the column count, so the table does not reflow on load.
    expect(container.querySelectorAll(".animate-pulse").length).toBe(
      3 * COLUMNS.length,
    );
    expect(screen.getAllByRole("columnheader")).toHaveLength(COLUMNS.length);
  });

  it("shows the empty state only when ready with no rows", () => {
    const { rerender } = render(
      <DataTable
        columns={COLUMNS}
        rows={[]}
        rowKey={(r) => r.id}
        empty={{ icon: Inbox, title: "Nothing here", description: "Add something." }}
      />,
    );
    expect(screen.getByText("Nothing here")).toBeInTheDocument();

    // Loading is NOT empty — showing "nothing here" before the data lands would be a lie.
    rerender(
      <DataTable
        columns={COLUMNS}
        rows={[]}
        rowKey={(r) => r.id}
        status="loading"
        empty={{ icon: Inbox, title: "Nothing here" }}
      />,
    );
    expect(screen.queryByText("Nothing here")).not.toBeInTheDocument();
  });

  it("surfaces an error instead of the rows", () => {
    render(
      <DataTable
        columns={COLUMNS}
        rows={ROWS}
        rowKey={(r) => r.id}
        status="error"
        error="Could not load."
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Could not load.");
    expect(screen.queryByText("Alpha")).not.toBeInTheDocument();
  });

  it("renders a pinned leading row and a footer slot", () => {
    render(
      <DataTable
        columns={COLUMNS}
        rows={ROWS}
        rowKey={(r) => r.id}
        leadingRow={
          <tr>
            <td colSpan={2}>Today</td>
          </tr>
        }
        footer={<p>1–2 of 2</p>}
      />,
    );
    const rows = screen.getAllByRole("row");
    expect(within(rows[1]!).getByText("Today")).toBeInTheDocument();
    expect(screen.getByText("1–2 of 2")).toBeInTheDocument();
  });

  it("pins a trailing note below the rows, spanning every column", () => {
    // Projected Activity keeps a TODAY row in its data, so `rows.length === 0` — and
    // therefore `empty` — can never fire there. "The filter matched nothing" has to be
    // expressible WITH rows present (personal-cfo-wxy7). The span is the primitive's,
    // not the caller's.
    const columns = [...COLUMNS, { key: "x", header: "X", cell: () => "x" }];
    render(
      <DataTable
        columns={columns}
        rows={ROWS}
        rowKey={(r) => r.id}
        trailingRow={<p>No matching activity</p>}
      />,
    );
    const rows = screen.getAllByRole("row");
    // Header + 2 data rows + the note, in that order.
    expect(rows).toHaveLength(4);
    const note = screen.getByText("No matching activity");
    expect(within(rows[3]!).getByText("No matching activity")).toBeInTheDocument();
    expect(note.closest("td")).toHaveAttribute("colspan", String(columns.length));
  });

  it("applies per-row attributes", () => {
    render(
      <DataTable
        columns={COLUMNS}
        rows={ROWS}
        rowKey={(r) => r.id}
        rowProps={(r) => (r.id === "b" ? { "data-state": "selected" } : {})}
      />,
    );
    const row = screen.getByText("Beta").closest("tr");
    expect(row).toHaveAttribute("data-state", "selected");
  });

  it("always renders a header", () => {
    // `hideHeader` was deleted in personal-cfo-9krd. It existed for the Money Inbox,
    // which does not migrate — its rows are polymorphic with state shared across cells,
    // which a column-def API cannot host — so the slot had no consumer and never would.
    // This asserts the primitive kept ONE way to render a header rather than two.
    render(<DataTable columns={COLUMNS} rows={ROWS} rowKey={(r) => r.id} />);
    expect(screen.getAllByRole("columnheader")).toHaveLength(COLUMNS.length);
  });
});
