import { fireEvent, screen, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type {
  ForecastDayDto,
  ForecastEventDto,
  MultiSeriesForecastDto,
} from "@/bindings";
import { ProjectedActivityTable } from "./ProjectedActivityTable";

// A bill row's expand panel renders the mark-paid control, which reads accounts + bills. Mock
// them empty so the control self-hides and these tests stay focused on the table itself.
const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  recurringBillList: vi.fn(),
  confirmObligationEarly: vi.fn(),
  unconfirmObligation: vi.fn(),
  createForecastAssumption: vi.fn(),
  forecastAssumptionList: vi.fn(),
  deleteForecastAssumption: vi.fn(),
}));
vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    recurringBillList: mocks.recurringBillList,
    confirmObligationEarly: mocks.confirmObligationEarly,
    unconfirmObligation: mocks.unconfirmObligation,
    createForecastAssumption: mocks.createForecastAssumption,
    forecastAssumptionList: mocks.forecastAssumptionList,
    deleteForecastAssumption: mocks.deleteForecastAssumption,
  },
}));
const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function money(minor: number) {
  return { minor_units: minor, currency: "USD" };
}

function event(name: string, kind: string, minor: number): ForecastEventDto {
  return {
    source_event_id: `evt-${name}`,
    name,
    kind,
    amount: money(minor),
    assumption_basis: { kind: "recurring_schedule", frequency: "monthly" },
  };
}

function day(date: string, minor: number, events: ForecastEventDto[] = []): ForecastDayDto {
  const closing = money(minor);
  return { date, closing: { p10: closing, p50: closing, p90: closing }, events };
}

function projection(): MultiSeriesForecastDto {
  const days = [
    day("2026-06-20", 250_000),
    day("2026-07-01", 70_000, [event("Rent", "recurring_bill", -180_000)]),
    day("2026-07-05", 220_000, [event("Acme Corp", "income", 150_000)]),
  ];
  const closings = days.map((d) => ({ date: d.date, closing: d.closing }));
  return {
    currency: "USD",
    start_date: "2026-06-20",
    horizon_days: 90,
    accounts: [
      { account_id: "acc-1", name: "Checking", subtype: "checking", tier: "spendable", days },
    ],
    groups: [
      { tier: "spendable", closings },
      { tier: "net", closings },
    ],
  };
}

/// Two paychecks on the SAME day — the case the feedback called out (both rows used to
/// show the day-end total). Opening 500.00 → +3,000.00 → +2,500.00 = 6,000.00.
function sameDayProjection(): MultiSeriesForecastDto {
  const days = [
    day("2026-06-20", 50_000),
    day("2026-06-25", 600_000, [
      event("Paycheck A", "income", 300_000),
      event("Paycheck B", "income", 250_000),
    ]),
  ];
  const closings = days.map((d) => ({ date: d.date, closing: d.closing }));
  return {
    currency: "USD",
    start_date: "2026-06-20",
    horizon_days: 90,
    accounts: [
      { account_id: "acc-1", name: "SoFi Checking", subtype: "checking", tier: "spendable", days },
    ],
    groups: [
      { tier: "spendable", closings },
      { tier: "net", closings },
    ],
  };
}

/// 12 activities (one per day) so pagination kicks in past the default page of 10.
function manyProjection(): MultiSeriesForecastDto {
  const days: ForecastDayDto[] = [day("2026-06-20", 100_000)];
  let balance = 100_000;
  for (let i = 1; i <= 12; i++) {
    balance += 10_000;
    days.push(
      day(`2026-07-${String(i).padStart(2, "0")}`, balance, [
        event(`Paycheck ${i}`, "income", 10_000),
      ]),
    );
  }
  const closings = days.map((d) => ({ date: d.date, closing: d.closing }));
  return {
    currency: "USD",
    start_date: "2026-06-20",
    horizon_days: 90,
    accounts: [
      { account_id: "acc-1", name: "Checking", subtype: "checking", tier: "spendable", days },
    ],
    groups: [
      { tier: "spendable", closings },
      { tier: "net", closings },
    ],
  };
}

describe("ProjectedActivityTable", () => {
  beforeEach(() => {
    window.localStorage.clear();
    mocks.accountList.mockResolvedValue(ok([]));
    mocks.recurringBillList.mockResolvedValue(ok([]));
  });

  it("renders Activity/Date/Amount + tier + Net columns, accounts hidden by default", () => {
    renderWithClient(<ProjectedActivityTable projection={projection()} />);

    const headers = screen.getAllByRole("columnheader").map((h) => h.textContent);
    expect(headers).toEqual(["Activity", "Date", "Amount", "Spendable", "Net cash"]);
    // The per-account column is deferred to the column-picker cog (personal-cfo-inaw).
    expect(screen.queryByText("Checking")).toBeNull();

    // TODAY is the first body row; the activities follow.
    const rows = screen.getAllByRole("row").slice(1); // drop the header row
    expect(within(rows[0]!).getByText("Today")).toBeInTheDocument();
    expect(screen.getByText("Rent")).toBeInTheDocument();
    expect(screen.getByText("Acme Corp")).toBeInTheDocument();
  });

  it("steps the running balance per row within a day (not the day-end total on every row)", () => {
    renderWithClient(<ProjectedActivityTable projection={sameDayProjection()} />);
    const rows = screen.getAllByRole("row").slice(1); // [Today, KP, KB]

    // TODAY = the opening balance.
    expect(within(rows[0]!).getAllByText("$500.00").length).toBeGreaterThan(0);
    // First paycheck lands on the intra-day running total, NOT the day-end total.
    expect(within(rows[1]!).getAllByText("$3,500.00").length).toBeGreaterThan(0);
    expect(within(rows[1]!).queryByText("$6,000.00")).toBeNull();
    // Second paycheck reconciles to the day-end closing.
    expect(within(rows[2]!).getAllByText("$6,000.00").length).toBeGreaterThan(0);
  });

  it("expands a row to its typed explanation (vkge)", () => {
    renderWithClient(<ProjectedActivityTable projection={projection()} />);

    const rentRow = screen.getByRole("button", { name: /Rent/ });
    expect(rentRow).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(rentRow);
    expect(rentRow).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Recurring schedule · Monthly")).toBeInTheDocument();
  });

  it("filters by activity name, keeping TODAY pinned and true running balances (yequ)", () => {
    renderWithClient(<ProjectedActivityTable projection={projection()} />);

    fireEvent.change(
      screen.getByRole("searchbox", { name: /filter projected activity/i }),
      { target: { value: "acme" } },
    );

    // Rent is hidden; TODAY stays pinned first.
    expect(screen.queryByText("Rent")).toBeNull();
    const rows = screen.getAllByRole("row").slice(1);
    expect(within(rows[0]!).getByText("Today")).toBeInTheDocument();
    // Acme keeps its true per-day balance ($2,200 — computed AFTER the hidden Rent
    // row), proving filtering hides rows without recomputing running balances.
    expect(within(rows[1]!).getByText("Acme Corp")).toBeInTheDocument();
    expect(within(rows[1]!).getAllByText("$2,200.00").length).toBeGreaterThan(0);
  });

  it("filters by activity kind via the type select (yequ)", () => {
    renderWithClient(<ProjectedActivityTable projection={projection()} />);

    fireEvent.change(
      screen.getByRole("combobox", { name: /filter by activity type/i }),
      { target: { value: "income" } },
    );
    expect(screen.getByText("Acme Corp")).toBeInTheDocument();
    expect(screen.queryByText("Rent")).toBeNull();
  });

  it("shows a muted no-match row when the filter hides every activity", () => {
    renderWithClient(<ProjectedActivityTable projection={projection()} />);

    fireEvent.change(
      screen.getByRole("searchbox", { name: /filter projected activity/i }),
      { target: { value: "zzz" } },
    );
    expect(screen.getByText("Today")).toBeInTheDocument();
    expect(screen.getByText(/no matching activity/i)).toBeInTheDocument();
  });

  it("paginates the activities (default 10) while keeping TODAY pinned", () => {
    renderWithClient(<ProjectedActivityTable projection={manyProjection()} />);
    // Page 1: TODAY pinned + the first 10 activities.
    expect(screen.getByText("Today")).toBeInTheDocument();
    expect(screen.getByText("Paycheck 1")).toBeInTheDocument();
    expect(screen.getByText("Paycheck 10")).toBeInTheDocument();
    expect(screen.queryByText("Paycheck 11")).toBeNull();
    expect(screen.getByText(/of 12 activities/)).toBeInTheDocument();

    // Next page: TODAY still pinned, the remaining activities show.
    fireEvent.click(screen.getByRole("button", { name: /next page/i }));
    expect(screen.getByText("Today")).toBeInTheDocument();
    expect(screen.getByText("Paycheck 11")).toBeInTheDocument();
    expect(screen.getByText("Paycheck 12")).toBeInTheDocument();
    expect(screen.queryByText("Paycheck 1")).toBeNull();
  });

  it("warns when a row overdraws an individual account the group columns hide (4d8.27.7.1)", () => {
    // Two accounts in one tier: checking is drained by rent while savings keeps the
    // GROUP positive — exactly the case the owner flagged as an invisible overdraft.
    const checkingDays = [
      day("2026-06-20", 10_000),
      day("2026-07-01", -8_000, [event("Rent", "recurring_bill", -18_000)]),
      // Still negative after this one — it must NOT re-warn (only the crossing does).
      day("2026-07-05", -20_000, [event("Utilities", "recurring_bill", -12_000)]),
    ];
    const savingsDays = [
      day("2026-06-20", 500_000),
      day("2026-07-01", 500_000),
      day("2026-07-05", 500_000),
    ];
    const groupClosings = [
      { date: "2026-06-20", closing: money(510_000) },
      { date: "2026-07-01", closing: money(492_000) },
      { date: "2026-07-05", closing: money(480_000) },
    ];
    renderWithClient(
      <ProjectedActivityTable
        projection={{
          currency: "USD",
          start_date: "2026-06-20",
          horizon_days: 90,
          accounts: [
            { account_id: "acc-1", name: "Checking", subtype: "checking", tier: "spendable", days: checkingDays },
            { account_id: "acc-2", name: "Savings", subtype: "savings", tier: "spendable", days: savingsDays },
          ],
          groups: [
            { tier: "spendable", closings: groupClosings.map((c) => ({ ...c, closing: { p10: c.closing, p50: c.closing, p90: c.closing } })) },
            { tier: "net", closings: groupClosings.map((c) => ({ ...c, closing: { p10: c.closing, p50: c.closing, p90: c.closing } })) },
          ],
        }}
      />,
    );
    // The Spendable column never goes negative, but Checking does.
    expect(screen.getByText(/Overdraws Checking to/)).toBeInTheDocument();
    // …and only on the crossing row, not on every row that stays negative.
    expect(screen.getAllByText(/Overdraws/)).toHaveLength(1);
  });

  it("does not warn for the synthetic Unallocated bucket, which is negative by design", () => {
    const unallocDays = [
      day("2026-06-20", 0),
      day("2026-07-01", -18_000, [event("Rent", "recurring_bill", -18_000)]),
    ];
    const closings = unallocDays.map((d) => ({ date: d.date, closing: d.closing }));
    renderWithClient(
      <ProjectedActivityTable
        projection={{
          currency: "USD",
          start_date: "2026-06-20",
          horizon_days: 90,
          accounts: [
            { account_id: null, name: "Unallocated cash", subtype: null, tier: "unallocated", days: unallocDays },
          ],
          groups: [
            { tier: "unallocated", closings },
            { tier: "net", closings },
          ],
        }}
      />,
    );
    expect(screen.queryByText(/Overdraws/)).toBeNull();
  });

  it("adds an individual account column on request, default off (4d8.27.7.2)", () => {
    renderWithClient(<ProjectedActivityTable projection={projection()} />);
    // Default: grouped columns only.
    expect(
      screen.getAllByRole("columnheader").map((h) => h.textContent),
    ).toEqual(["Activity", "Date", "Amount", "Spendable", "Net cash"]);

    fireEvent.click(screen.getByRole("button", { name: /accounts/i }));
    fireEvent.click(
      within(screen.getByRole("group", { name: "Account columns" })).getByRole(
        "checkbox",
        { name: "Checking" },
      ),
    );
    expect(
      screen.getAllByRole("columnheader").map((h) => h.textContent),
    ).toContain("Checking");
  });

  it("names the account a row moves money out of (4d8.27.7.5)", () => {
    renderWithClient(<ProjectedActivityTable projection={projection()} />);
    fireEvent.click(screen.getByRole("button", { name: /Rent/ }));
    expect(screen.getByText(/comes out of/i)).toBeInTheDocument();
    expect(screen.getAllByText("Checking").length).toBeGreaterThanOrEqual(1);
  });

  it("adjusts ONE occurrence's amount, scoped to that date only (4d8.27.7.4)", async () => {
    mocks.createForecastAssumption.mockResolvedValue(ok({ id: "a1" }));
    renderWithClient(<ProjectedActivityTable projection={projection()} />);
    fireEvent.click(screen.getByRole("button", { name: /Rent/ }));
    fireEvent.click(screen.getByRole("button", { name: /adjust this amount/i }));
    fireEvent.change(screen.getByLabelText(/amount for this occurrence only/i), {
      target: { value: "1830.00" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await vi.waitFor(() =>
      expect(mocks.createForecastAssumption).toHaveBeenCalledWith(
        expect.objectContaining({
          kind: "bill_amount",
          target_entity_id: "evt-Rent",
          new_amount_minor: 183_000,
          // The window IS the single occurrence — later months keep the base amount.
          effective_date: "2026-07-01",
          end_date: "2026-07-01",
          scenario_id: null,
        }),
      ),
    );
  });

  it("shows a rotating chevron on expandable rows, and none on TODAY (4d8.27.7.3)", () => {
    renderWithClient(<ProjectedActivityTable projection={projection()} />);
    const toggle = screen.getByRole("button", { name: /Rent/ });
    const chevron = toggle.querySelector("svg");
    expect(chevron).not.toBeNull();
    expect(chevron).not.toHaveClass("rotate-90");
    fireEvent.click(toggle);
    expect(toggle.querySelector("svg")).toHaveClass("rotate-90");
    // TODAY is the opening anchor, not an activity — no disclosure affordance.
    const todayRow = screen.getAllByRole("row")[1];
    expect(within(todayRow!).queryByRole("button")).toBeNull();
  });

  it("surfaces an existing adjustment and can reset it (4d8.27.7.4)", async () => {
    // A base override already exists for this occurrence — the user must be able to SEE
    // and UNDO it, or adjusting is a one-way door on their real forecast.
    mocks.forecastAssumptionList.mockResolvedValue(
      ok([
        {
          id: "assump-1",
          kind: "bill_amount",
          target_entity_id: "evt-Rent",
          scenario_id: null,
          params_json: JSON.stringify({
            effective_date: "2026-07-01",
            end_date: "2026-07-01",
            new_amount_minor: 183_000,
          }),
          created_at: "2026-06-21T00:00:00Z",
        },
      ]),
    );
    mocks.deleteForecastAssumption.mockResolvedValue(ok(null));
    renderWithClient(<ProjectedActivityTable projection={projection()} />);
    fireEvent.click(screen.getByRole("button", { name: /Rent/ }));
    expect(await screen.findByText(/adjusted for this date/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /reset/i }));
    await vi.waitFor(() =>
      expect(mocks.deleteForecastAssumption).toHaveBeenCalledWith("assump-1"),
    );
  });
});
