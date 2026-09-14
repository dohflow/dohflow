import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto, IncomeSourceDto } from "@/bindings";
import { IncomeView } from "./IncomeView";

const mocks = vi.hoisted(() => ({
  incomeCandidates: vi.fn(),
  forecastAssumptionList: vi.fn(),
  scenarioList: vi.fn(),
  accountList: vi.fn(),
  incomeSourceList: vi.fn(),
  createIncomeSource: vi.fn(),
  updateIncomeSource: vi.fn(),
  deleteIncomeSource: vi.fn(),
  archiveIncomeSource: vi.fn(),
  restoreIncomeSource: vi.fn(),
  baseCurrency: vi.fn(),
  setBaseCurrency: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    incomeCandidates: mocks.incomeCandidates,
    accountList: mocks.accountList,
    incomeSourceList: mocks.incomeSourceList,
    forecastAssumptionList: mocks.forecastAssumptionList,
    scenarioList: mocks.scenarioList,
    createIncomeSource: mocks.createIncomeSource,
    updateIncomeSource: mocks.updateIncomeSource,
    deleteIncomeSource: mocks.deleteIncomeSource,
    archiveIncomeSource: mocks.archiveIncomeSource,
    restoreIncomeSource: mocks.restoreIncomeSource,
    baseCurrency: mocks.baseCurrency,
    setBaseCurrency: mocks.setBaseCurrency,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function account(over: Partial<AccountViewDto> = {}): AccountViewDto {
  return {
    id: "0190a000-0000-7000-8000-000000000001",
    name: "Checking",
    cashflow_role: "liquid_cash",
    subtype: null,
    active: true,
    balance: { minor_units: 125_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  };
}

function source(over: Partial<IncomeSourceDto> = {}): IncomeSourceDto {
  return {
    id: "0190c000-0000-7000-8000-000000000001",
    name: "Acme Corp",
    net_amount: { minor_units: 320_000, currency: "USD" },
    frequency: "biweekly",
    anchor_date: "2026-06-12",
    deposit_account_id: null,
    deposit_account_name: null,
    next_pay_date: "2026-06-26",
    active: true,
    created_at: "2026-06-01T00:00:00Z",
    archived_at: null,
    ...over,
  };
}

const mutationOk = () => ok({ op_seq: 1, replayed: false });

beforeEach(() => {
  mocks.incomeCandidates.mockResolvedValue({ status: "ok", data: [] });
  mocks.accountList.mockReset();
  mocks.incomeSourceList.mockReset();
  mocks.forecastAssumptionList.mockReset();
  mocks.forecastAssumptionList.mockResolvedValue(ok([]));
  mocks.scenarioList.mockReset();
  mocks.scenarioList.mockResolvedValue(ok([]));
  mocks.createIncomeSource.mockReset();
  mocks.updateIncomeSource.mockReset();
  mocks.deleteIncomeSource.mockReset();
  mocks.archiveIncomeSource.mockReset();
  mocks.restoreIncomeSource.mockReset();
  mocks.baseCurrency.mockReset();
  mocks.setBaseCurrency.mockReset();
  mocks.accountList.mockResolvedValue(ok([account()]));
  mocks.incomeSourceList.mockResolvedValue(ok([]));
  mocks.updateIncomeSource.mockResolvedValue(mutationOk());
  mocks.deleteIncomeSource.mockResolvedValue(mutationOk());
  mocks.archiveIncomeSource.mockResolvedValue(mutationOk());
  mocks.restoreIncomeSource.mockResolvedValue(mutationOk());
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.setBaseCurrency.mockResolvedValue(ok(null));
});

describe("IncomeView", () => {
  it("shows the empty state when there are no income sources", async () => {
    renderWithClient(<IncomeView />);
    expect(await screen.findByText(/no income yet/i)).toBeInTheDocument();
  });

  it("lists a source with its frequency label, next pay date, and amount", async () => {
    mocks.incomeSourceList.mockResolvedValue(ok([source()]));
    renderWithClient(<IncomeView />);
    expect(await screen.findByText("Acme Corp")).toBeInTheDocument();
    // The subline concatenates the frequency label and the next pay date; the
    // plain `YYYY-MM-DD` renders as a local date (no off-by-one shift).
    expect(screen.getByText(/Biweekly/)).toBeInTheDocument();
    expect(screen.getByText(/Jun 26, 2026/)).toBeInTheDocument();
    expect(screen.getByText(/\$3,200\.00/)).toBeInTheDocument();
  });

  it("creates an income source with the entered fields and refreshes", async () => {
    mocks.accountList.mockResolvedValue(ok([])); // no deposit account available
    mocks.incomeSourceList
      .mockResolvedValueOnce(ok([]))
      .mockResolvedValue(ok([source({ name: "Acme Corp", frequency: "monthly" })]));
    mocks.createIncomeSource.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
    renderWithClient(<IncomeView />);

    fireEvent.click(await screen.findByRole("button", { name: /add income/i }));
    fireEvent.change(screen.getByLabelText(/source name/i), {
      target: { value: "Acme Corp" },
    });
    fireEvent.change(screen.getByLabelText(/net amount/i), {
      target: { value: "3200" },
    });
    fireEvent.change(screen.getByLabelText(/frequency/i), {
      target: { value: "monthly" },
    });
    fireEvent.change(screen.getByLabelText(/anchor pay date/i), {
      target: { value: "2026-06-15" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add income/i }));

    await screen.findByText("Acme Corp");
    expect(mocks.createIncomeSource).toHaveBeenCalledTimes(1);
    const input = mocks.createIncomeSource.mock.calls[0]?.[0];
    expect(input.name).toBe("Acme Corp");
    expect(input.frequency).toBe("monthly");
    expect(input.anchor_date).toBe("2026-06-15");
    expect(input.net_amount.minor_units).toBe(320_000); // 3200.00 → minor units
    expect(input.net_amount.currency).toBe("USD"); // default without an account
    expect(input.deposit_account_id).toBeNull();
  });

  it("links a deposit account and derives its currency", async () => {
    const euro = account({
      id: "0190a000-0000-7000-8000-0000000000ee",
      name: "Euro Checking",
      balance: { minor_units: 0, currency: "EUR" },
    });
    mocks.accountList.mockResolvedValue(ok([euro]));
    mocks.createIncomeSource.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
    renderWithClient(<IncomeView />);

    fireEvent.click(await screen.findByRole("button", { name: /add income/i }));
    // Wait for the account list to populate the deposit dropdown.
    await screen.findByRole("option", { name: "Euro Checking" });
    fireEvent.change(screen.getByLabelText(/source name/i), {
      target: { value: "Euro Job" },
    });
    fireEvent.change(screen.getByLabelText(/net amount/i), {
      target: { value: "1000" },
    });
    fireEvent.change(screen.getByLabelText(/deposit account/i), {
      target: { value: euro.id },
    });
    fireEvent.click(screen.getByRole("button", { name: /add income/i }));

    await waitFor(() => expect(mocks.createIncomeSource).toHaveBeenCalled());
    const input = mocks.createIncomeSource.mock.calls[0]?.[0];
    expect(input.deposit_account_id).toBe(euro.id);
    expect(input.net_amount.currency).toBe("EUR");
  });

  it("edits a source and calls update with its id (tch0)", async () => {
    mocks.incomeSourceList.mockResolvedValue(ok([source({ name: "Acme Corp" })]));
    renderWithClient(<IncomeView />);

    fireEvent.click(await screen.findByRole("button", { name: /edit acme corp/i }));
    fireEvent.change(screen.getByLabelText(/source name/i), {
      target: { value: "Globex" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() => expect(mocks.updateIncomeSource).toHaveBeenCalled());
    const input = mocks.updateIncomeSource.mock.calls[0]?.[0];
    expect(input.income_source_id).toBe(source().id);
    expect(input.name).toBe("Globex");
  });

  it("archives an active source (tch0)", async () => {
    mocks.incomeSourceList.mockResolvedValue(ok([source({ name: "Acme Corp" })]));
    renderWithClient(<IncomeView />);

    fireEvent.click(await screen.findByRole("button", { name: /archive acme corp/i }));
    await waitFor(() =>
      expect(mocks.archiveIncomeSource).toHaveBeenCalledWith(
        source().id,
        expect.stringMatching(/.+/),
      ),
    );
  });

  it("deletes a source behind a two-click confirm (tch0)", async () => {
    mocks.incomeSourceList.mockResolvedValue(ok([source({ name: "Acme Corp" })]));
    renderWithClient(<IncomeView />);

    fireEvent.click(await screen.findByRole("button", { name: /delete acme corp/i }));
    fireEvent.click(
      await screen.findByRole("button", { name: /confirm delete acme corp/i }),
    );
    await waitFor(() =>
      expect(mocks.deleteIncomeSource).toHaveBeenCalledWith(
        source().id,
        expect.stringMatching(/.+/),
      ),
    );
  });

  it("lists archived sources under their own heading and restores them (tch0)", async () => {
    mocks.incomeSourceList.mockResolvedValue(
      ok([
        source({
          name: "Old Job",
          active: false,
          archived_at: "2026-06-20T00:00:00Z",
        }),
      ]),
    );
    renderWithClient(<IncomeView />);

    expect(await screen.findByText("Archived")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /restore old job/i }));
    await waitFor(() =>
      expect(mocks.restoreIncomeSource).toHaveBeenCalledWith(
        source().id,
        expect.stringMatching(/.+/),
      ),
    );
  });
});

describe("applied-scenario overrides (personal-cfo-abhr, ADR 0055)", () => {
  const SOURCE_ID = "0190c000-0000-7000-8000-000000000001";

  it("names the applied scenario overriding an income source's forecast figure", async () => {
    mocks.incomeSourceList.mockResolvedValue(ok([source({ id: SOURCE_ID })]));
    mocks.forecastAssumptionList.mockResolvedValue(
      ok([
        {
          id: "evt-1",
          kind: "income_amount",
          target_entity_id: SOURCE_ID,
          scenario_id: null,
          promoted_from_scenario_id: "scn-9",
          params_json: JSON.stringify({ new_amount_minor: 400_000 }),
          created_at: "2026-08-02T00:00:00Z",
        },
      ]),
    );
    mocks.scenarioList.mockResolvedValue(
      ok([
        {
          id: "scn-9",
          name: "Pay rise",
          description: null,
          status: "draft",
          created_at: "2026-08-01",
          updated_at: "2026-08-01",
          expires_on: null,
          event_count: 1,
          applied_at: "2026-08-02T00:00:00Z",
        },
      ]),
    );
    const onOpenScenario = vi.fn();
    renderWithClient(<IncomeView onOpenScenario={onOpenScenario} />);

    expect(await screen.findByText(/your forecast uses/i)).toBeInTheDocument();
    expect(screen.getByText("$4,000.00")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Pay rise" }));
    expect(onOpenScenario).toHaveBeenCalledWith("scn-9");
  });

  it("says nothing for a source no applied scenario touched", async () => {
    mocks.incomeSourceList.mockResolvedValue(ok([source({ id: SOURCE_ID })]));
    renderWithClient(<IncomeView onOpenScenario={vi.fn()} />);
    await screen.findByText("Acme Corp");
    expect(screen.queryByText(/your forecast uses/i)).not.toBeInTheDocument();
  });
});
