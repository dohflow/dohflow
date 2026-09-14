import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AssumptionEventDto, RecurringBillDto } from "@/bindings";
import { ScenarioEvents } from "./ScenarioEvents";

const mocks = vi.hoisted(() => ({
  forecastAssumptionList: vi.fn(),
  createForecastAssumption: vi.fn(),
  deleteForecastAssumption: vi.fn(),
  recurringBillList: vi.fn(),
  incomeSourceList: vi.fn(),
  categoryList: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    forecastAssumptionList: mocks.forecastAssumptionList,
    createForecastAssumption: mocks.createForecastAssumption,
    deleteForecastAssumption: mocks.deleteForecastAssumption,
    recurringBillList: mocks.recurringBillList,
    incomeSourceList: mocks.incomeSourceList,
    categoryList: mocks.categoryList,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function bill(over: Partial<RecurringBillDto> = {}): RecurringBillDto {
  return {
    id: "bill-1",
    name: "Rent",
    bill_type: "rent_mortgage",
    amount: { minor_units: 180_000, currency: "USD" },
    frequency: "monthly",
    anchor_date: "2026-06-01",
    autopay_account_id: null,
    autopay_account_name: null,
    autopay_enabled: false,
    next_due_date: "2026-07-01",
    description: null,
    active: true,
    created_at: "2026-06-01T00:00:00Z",
    archived_at: null,
    category_id: null,
    tag_ids: [],
    ...over,
  };
}

function assumptionEvent(over: Partial<AssumptionEventDto> = {}): AssumptionEventDto {
  return {
    id: "evt-1",
    kind: "exclusion",
    target_entity_id: "bill-1",
    scenario_id: "scn-1",
    promoted_from_scenario_id: null,
    params_json: "{}",
    created_at: "2026-06-20T00:00:00Z",
    ...over,
  };
}

beforeEach(() => {
  mocks.forecastAssumptionList.mockReset().mockResolvedValue(ok([]));
  mocks.createForecastAssumption.mockReset();
  mocks.deleteForecastAssumption.mockReset();
  mocks.recurringBillList.mockReset().mockResolvedValue(ok([]));
  mocks.incomeSourceList.mockReset().mockResolvedValue(ok([]));
});

describe("ScenarioEvents", () => {
  it("adds a one-off addition (money in)", async () => {
    mocks.createForecastAssumption.mockResolvedValue(
      ok(assumptionEvent({ kind: "one_time_event", target_entity_id: null })),
    );
    renderWithClient(<ScenarioEvents scenarioId="scn-1" currency="USD" />);

    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // open
    // "Add money" is the default change kind.
    fireEvent.change(screen.getByLabelText("Label"), {
      target: { value: "Bonus" },
    });
    fireEvent.change(screen.getByLabelText(/Amount/), {
      target: { value: "5000" },
    });
    fireEvent.change(screen.getByLabelText("Date"), {
      target: { value: "2026-08-01" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // submit

    await waitFor(() =>
      expect(mocks.createForecastAssumption).toHaveBeenCalledWith(
        expect.objectContaining({
          kind: "one_time_event",
          scenario_id: "scn-1",
          target_entity_id: null,
          amount: { minor_units: 500_000, currency: "USD" },
          date: "2026-08-01",
          label: "Bonus",
        }),
      ),
    );
  });

  it("overrides a bill amount with positive minor units (bill_amount)", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    mocks.createForecastAssumption.mockResolvedValue(
      ok(assumptionEvent({ kind: "bill_amount" })),
    );
    renderWithClient(<ScenarioEvents scenarioId="scn-1" currency="USD" />);

    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // open
    fireEvent.click(screen.getByRole("button", { name: "Change amount" }));
    await screen.findByLabelText("Item"); // bills loaded → target select shows
    fireEvent.change(screen.getByLabelText(/New amount/), {
      target: { value: "2500" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // submit

    await waitFor(() =>
      expect(mocks.createForecastAssumption).toHaveBeenCalledWith(
        expect.objectContaining({
          kind: "bill_amount",
          scenario_id: "scn-1",
          target_entity_id: "bill-1",
          new_amount_minor: 250_000,
        }),
      ),
    );
  });

  it("allows a $0 amount override — an income/bill drops to 0 (yiau)", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    mocks.createForecastAssumption.mockResolvedValue(
      ok(assumptionEvent({ kind: "bill_amount" })),
    );
    renderWithClient(<ScenarioEvents scenarioId="scn-1" currency="USD" />);

    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // open
    fireEvent.click(screen.getByRole("button", { name: "Change amount" }));
    await screen.findByLabelText("Item");
    fireEvent.change(screen.getByLabelText(/New amount/), {
      target: { value: "0" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // submit

    await waitFor(() =>
      expect(mocks.createForecastAssumption).toHaveBeenCalledWith(
        expect.objectContaining({
          kind: "bill_amount",
          target_entity_id: "bill-1",
          new_amount_minor: 0,
        }),
      ),
    );
  });

  it("still rejects a $0 one-off addition (yiau)", async () => {
    renderWithClient(<ScenarioEvents scenarioId="scn-1" currency="USD" />);

    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // open
    fireEvent.change(screen.getByLabelText("Label"), { target: { value: "Nope" } });
    fireEvent.change(screen.getByLabelText(/Amount/), { target: { value: "0" } });
    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // submit

    expect(await screen.findByText(/greater than zero/i)).toBeInTheDocument();
    expect(mocks.createForecastAssumption).not.toHaveBeenCalled();
  });

  it("excludes a bill (exclusion)", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    mocks.createForecastAssumption.mockResolvedValue(ok(assumptionEvent()));
    renderWithClient(<ScenarioEvents scenarioId="scn-1" currency="USD" />);

    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // open
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    await screen.findByLabelText("Item");
    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // submit

    await waitFor(() =>
      expect(mocks.createForecastAssumption).toHaveBeenCalledWith(
        expect.objectContaining({
          kind: "exclusion",
          scenario_id: "scn-1",
          target_entity_id: "bill-1",
          new_amount_minor: 0,
        }),
      ),
    );
  });

  it("lists changes with a human description and deletes one", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    mocks.forecastAssumptionList.mockResolvedValue(ok([assumptionEvent()]));
    mocks.deleteForecastAssumption.mockResolvedValue(ok(null));
    renderWithClient(<ScenarioEvents scenarioId="scn-1" currency="USD" />);

    expect(await screen.findByText("Remove Rent")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /delete change/i }));
    await waitFor(() =>
      expect(mocks.deleteForecastAssumption).toHaveBeenCalledWith("evt-1"),
    );
  });

  it("overrides an amount within a [start, end] window (w6o9)", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    mocks.createForecastAssumption.mockResolvedValue(
      ok(assumptionEvent({ kind: "bill_amount" })),
    );
    renderWithClient(<ScenarioEvents scenarioId="scn-1" currency="USD" />);

    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // open
    fireEvent.click(screen.getByRole("button", { name: "Change amount" }));
    await screen.findByLabelText("Item");
    fireEvent.change(screen.getByLabelText(/New amount/), {
      target: { value: "1000" },
    });
    fireEvent.change(screen.getByLabelText(/Starting from/i), {
      target: { value: "2026-10-01" },
    });
    fireEvent.change(screen.getByLabelText(/Until/i), {
      target: { value: "2026-11-30" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // submit

    await waitFor(() =>
      expect(mocks.createForecastAssumption).toHaveBeenCalledWith(
        expect.objectContaining({
          kind: "bill_amount",
          new_amount_minor: 100_000,
          effective_date: "2026-10-01",
          end_date: "2026-11-30",
        }),
      ),
    );
  });

  it("rejects an end date before the start date", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    renderWithClient(<ScenarioEvents scenarioId="scn-1" currency="USD" />);

    fireEvent.click(screen.getByRole("button", { name: /add change/i }));
    fireEvent.click(screen.getByRole("button", { name: "Change amount" }));
    await screen.findByLabelText("Item");
    fireEvent.change(screen.getByLabelText(/New amount/), {
      target: { value: "1000" },
    });
    fireEvent.change(screen.getByLabelText(/Starting from/i), {
      target: { value: "2026-11-30" },
    });
    fireEvent.change(screen.getByLabelText(/Until/i), {
      target: { value: "2026-10-01" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add change/i }));

    expect(
      await screen.findByText(/on or after the start date/i),
    ).toBeInTheDocument();
    expect(mocks.createForecastAssumption).not.toHaveBeenCalled();
  });

  it("describes a windowed amount override with from/to dates", async () => {
    mocks.recurringBillList.mockResolvedValue(ok([bill()]));
    mocks.forecastAssumptionList.mockResolvedValue(
      ok([
        assumptionEvent({
          kind: "bill_amount",
          params_json: JSON.stringify({
            new_amount_minor: 100_000,
            effective_date: "2026-10-01",
            end_date: "2026-11-30",
          }),
        }),
      ]),
    );
    renderWithClient(<ScenarioEvents scenarioId="scn-1" currency="USD" />);

    expect(await screen.findByText(/from .+ to .+/)).toBeInTheDocument();
  });

  it("describes a recurring debt-payment overlay (payoff scenario, jgid)", async () => {
    mocks.forecastAssumptionList.mockResolvedValue(
      ok([
        assumptionEvent({
          kind: "recurring_debt_payment",
          target_entity_id: null,
          params_json: JSON.stringify({
            amount_minor: 50_000,
            currency: "USD",
            anchor_date: "2026-08-01",
            label: "Extra debt payment",
          }),
        }),
      ]),
    );
    renderWithClient(<ScenarioEvents scenarioId="scn-1" currency="USD" />);

    // Humanized, not the raw "recurring_debt_payment" token.
    expect(
      await screen.findByText(/Extra debt payment: \$500\.00\/mo from/),
    ).toBeInTheDocument();
    expect(screen.queryByText("recurring_debt_payment")).toBeNull();
  });

  it("plans a category spend cut as a signed monthly delta (4d8.27.6.2)", async () => {
    mocks.categoryList.mockResolvedValue(
      ok([
        {
          id: "cat-dining",
          name: "Dining",
          parent_id: null,
          type: "expense",
          is_system: true,
          forecast_behavior: "variable_regular",
          archived: false,
          color: null,
          icon: null,
        },
        // A fixed category cannot be adjusted — the spend model does not project it.
        {
          id: "cat-rent",
          name: "Rent",
          parent_id: null,
          type: "expense",
          is_system: true,
          forecast_behavior: "deterministic",
          archived: false,
          color: null,
          icon: null,
        },
      ]),
    );
    mocks.createForecastAssumption.mockResolvedValue(ok({ id: "a1" }));
    renderWithClient(<ScenarioEvents scenarioId="sce-1" currency="USD" />);

    fireEvent.click(await screen.findByRole("button", { name: /add change/i }));
    fireEvent.click(screen.getByRole("button", { name: "Spend more or less" }));

    // Only variable-spend categories are offered — a fixed one cannot be adjusted.
    expect(await screen.findByRole("option", { name: "Dining" })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "Rent" })).toBeNull();

    const picker = screen.getByLabelText("Category");
    fireEvent.change(picker, { target: { value: "cat-dining" } });
    fireEvent.change(screen.getByLabelText(/change per month/i), {
      target: { value: "200" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add change/i })); // submit

    await waitFor(() =>
      expect(mocks.createForecastAssumption).toHaveBeenCalledWith(
        expect.objectContaining({
          kind: "variable_spend_override",
          target_entity_id: "cat-dining",
          // "less" is the default, and is stored NEGATIVE.
          new_amount_minor: -20_000,
        }),
      ),
    );
  });
});
