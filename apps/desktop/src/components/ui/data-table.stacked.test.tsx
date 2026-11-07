import { fireEvent, render, screen, within } from "@testing-library/react";
import { Inbox } from "lucide-react";

import { COMPACT_QUERY } from "@/lib/compact";
import { DataTable, type DataTableColumn, type DataTableSort } from "./data-table";

function stubMatchMedia(compact: boolean) {
  vi.stubGlobal("matchMedia", ((query: string) => ({
    matches: compact && query === COMPACT_QUERY,
    media: query,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  })) as unknown as typeof window.matchMedia);
}
afterEach(() => vi.unstubAllGlobals());

type Row = { id: string; name: string; date: string; amount: string };
const ROWS: Row[] = [
  { id: "a", name: "Glassmoor Utilities and a very long merchant name that must wrap and never be clipped", date: "Nov 14, 2026", amount: "-$184.32" },
  { id: "b", name: "Northhollow Health Payroll", date: "Nov 17, 2026", amount: "+$2,912.40" },
];
const COLUMNS: DataTableColumn<Row>[] = [
  { key: "name", header: "Activity", cell: (r) => r.name },
  { key: "date", header: "Date", cell: (r) => r.date },
  { key: "amount", header: "Amount", align: "right", cell: (r) => r.amount },
];

function table(extra: Partial<React.ComponentProps<typeof DataTable<Row>>> = {}) {
  return (
    <DataTable<Row> columns={COLUMNS} rows={ROWS} rowKey={(r) => r.id} label="Activity" phoneLayout="stacked" {...extra} />
  );
}

describe("DataTable on a phone (personal-cfo-i93n3.27 slice F)", () => {
  it("is the horizontal table on a desktop window, even when it opts in", () => {
    stubMatchMedia(false);
    render(table());
    expect(screen.getByRole("table", { name: "Activity" })).toBeInTheDocument();
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("is the horizontal table everywhere when it does not opt in, even on a phone", () => {
    stubMatchMedia(true);
    render(table({ phoneLayout: undefined }));
    expect(screen.getByRole("table", { name: "Activity" })).toBeInTheDocument();
  });

  it("is a list of cards on a phone: no table, one item per row", () => {
    stubMatchMedia(true);
    render(table());
    expect(screen.queryByRole("table")).toBeNull();
    const list = screen.getByRole("list", { name: "Activity" });
    expect(within(list).getAllByRole("listitem")).toHaveLength(2);
  });

  it("reads each row's values as label then value, in column order, as a description list", () => {
    stubMatchMedia(true);
    render(table());
    const item = within(screen.getByRole("list")).getAllByRole("listitem")[1]!;
    const terms = [...item.querySelectorAll("dt")].map((t) => t.textContent);
    const values = [...item.querySelectorAll("dd")].map((d) => d.textContent);
    expect(terms).toEqual(["Date", "Amount"]);
    expect(values).toEqual(["Nov 17, 2026", "+$2,912.40"]);
    // The title column is the row's heading; its label is read, not drawn.
    expect(item.querySelector(".sr-only")?.textContent).toBe("Activity: ");
    expect(item).toHaveTextContent("Northhollow Health Payroll");
    // Document order is the reading order: title, then each label before its value.
    const order = [...item.querySelectorAll("*")].map((e) => e.textContent);
    expect(order.indexOf("Date")).toBeLessThan(order.indexOf("Nov 17, 2026"));
  });

  it("never clips: long values wrap and the card has no overflow or truncation", () => {
    stubMatchMedia(true);
    render(table());
    const long = screen.getByText(/very long merchant name/);
    expect(long.className).toContain("break-words");
    const list = screen.getByRole("list");
    expect(list.outerHTML).not.toMatch(/truncate|overflow-x|whitespace-nowrap|min-w-\[/);
  });

  it("renders an expanded detail under its row and keeps per-row data-state and classes", () => {
    stubMatchMedia(true);
    render(
      table({
        expandedContent: (r) => (r.id === "a" ? <p>why this is projected</p> : null),
        rowProps: (r) => ({ className: r.id === "a" ? "accent" : undefined, "data-state": r.id === "b" ? "selected" : undefined }),
      }),
    );
    const [first, second] = within(screen.getByRole("list")).getAllByRole("listitem");
    expect(within(first!).getByText("why this is projected")).toBeInTheDocument();
    expect(within(second!).queryByText("why this is projected")).toBeNull();
    expect(first).toHaveClass("accent");
    expect(second).toHaveAttribute("data-state", "selected");
  });

  it("keeps the loading, error, empty and trailing states", () => {
    stubMatchMedia(true);
    const { rerender } = render(table({ status: "loading" }));
    expect(screen.getAllByRole("listitem")).toHaveLength(3);
    rerender(table({ status: "error", error: "It failed.", errorAction: <button type="button">Retry</button> }));
    expect(screen.getByRole("alert")).toHaveTextContent("It failed.");
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
    rerender(table({ rows: [], empty: { icon: Inbox, title: "Nothing here" } }));
    expect(screen.getByText("Nothing here")).toBeInTheDocument();
    rerender(table({ trailingRow: <p>filtered out everything</p> }));
    expect(screen.getByText("filtered out everything")).toBeInTheDocument();
  });

  it("keeps sorting reachable without headers: a Sort by control and a direction button", () => {
    stubMatchMedia(true);
    const onSortChange = vi.fn();
    const columns: DataTableColumn<Row>[] = COLUMNS.map((c) => (c.key === "date" || c.key === "amount" ? { ...c, sortable: true } : c));
    const sort: DataTableSort = { key: "date", direction: "asc" };
    render(table({ columns, sort, onSortChange }));
    const select = screen.getByRole("combobox", { name: "Sort by" }) as HTMLSelectElement;
    expect([...select.options].map((o) => o.textContent)).toEqual(["Date", "Amount"]);
    fireEvent.change(select, { target: { value: "amount" } });
    expect(onSortChange).toHaveBeenLastCalledWith({ key: "amount", direction: "asc" });
    fireEvent.click(screen.getByRole("button", { name: "Sorted ascending; sort descending" }));
    expect(onSortChange).toHaveBeenLastCalledWith({ key: "date", direction: "desc" });
  });

  it("offers no sort control when no column is sortable (running balances must not be re-sorted)", () => {
    stubMatchMedia(true);
    render(table({ onSortChange: vi.fn() }));
    expect(screen.queryByRole("combobox", { name: "Sort by" })).toBeNull();
  });

  it("keeps the table when a row block of raw table cells is pinned (leadingRow is not stackable)", () => {
    stubMatchMedia(true);
    render(
      table({
        leadingRow: (
          <tr>
            <td colSpan={3}>pinned</td>
          </tr>
        ),
      }),
    );
    expect(screen.getByRole("table")).toBeInTheDocument();
  });
});
