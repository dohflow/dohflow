import { fireEvent, screen, waitFor } from "@testing-library/react";

import type { RecurringCandidateDto } from "@/bindings";
import { renderWithClient } from "@/test/renderWithClient";

import { FirstForecastWizard } from "./FirstForecastWizard";

const mocks = vi.hoisted(() => ({
  importPreviewColumns: vi.fn(),
  importBatch: vi.fn(),
  markReviewed: vi.fn(),
  dismissRecurringSuggestion: vi.fn(),
  recurringBillHistory: vi.fn(),
  tagList: vi.fn(),
  categoryList: vi.fn(),
  recurringCandidates: vi.fn(),
  incomeCandidates: vi.fn(),
  accountList: vi.fn(),
  incomeSourceList: vi.fn(),
  recurringBillList: vi.fn(),
  baseCurrency: vi.fn(),
  setBaseCurrency: vi.fn(),
  createAccount: vi.fn(),
  createIncomeSource: vi.fn(),
  createRecurringBill: vi.fn(),
  futureCashForecast: vi.fn(),
  cashAvailability: vi.fn(),
  setMinimumCashFloor: vi.fn(),
  connectorConnections: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    importPreviewColumns: mocks.importPreviewColumns,
    importBatch: mocks.importBatch,
    markReviewed: mocks.markReviewed,
    dismissRecurringSuggestion: mocks.dismissRecurringSuggestion,
    recurringBillHistory: mocks.recurringBillHistory,
    tagList: mocks.tagList,
    categoryList: mocks.categoryList,
    recurringCandidates: mocks.recurringCandidates,
    incomeCandidates: mocks.incomeCandidates,
    accountList: mocks.accountList,
    incomeSourceList: mocks.incomeSourceList,
    recurringBillList: mocks.recurringBillList,
    baseCurrency: mocks.baseCurrency,
    setBaseCurrency: mocks.setBaseCurrency,
    createAccount: mocks.createAccount,
    createIncomeSource: mocks.createIncomeSource,
    createRecurringBill: mocks.createRecurringBill,
    futureCashForecast: mocks.futureCashForecast,
    cashAvailability: mocks.cashAvailability,
    setMinimumCashFloor: mocks.setMinimumCashFloor,
    connectorConnections: mocks.connectorConnections,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const usd = (minor: number) => ({ minor_units: minor, currency: "USD" });

const oneAccount = {
  id: "acc-1",
  name: "Checking",
  cashflow_role: "liquid_cash",
  subtype: null,
  active: true,
  balance: usd(50_000),
};

beforeEach(() => {
  for (const fn of Object.values(mocks)) fn.mockReset();
  mocks.incomeCandidates.mockResolvedValue({ status: "ok", data: [] });
  mocks.recurringCandidates.mockResolvedValue({ status: "ok", data: [] });
  mocks.categoryList.mockResolvedValue({ status: "ok", data: [] });
  mocks.tagList.mockResolvedValue({ status: "ok", data: [] });
  mocks.dismissRecurringSuggestion.mockResolvedValue({ status: "ok", data: null });
  mocks.recurringBillHistory.mockResolvedValue({ status: "ok", data: [] });
  mocks.accountList.mockResolvedValue(ok([oneAccount]));
  mocks.incomeSourceList.mockResolvedValue(ok([]));
  mocks.recurringBillList.mockResolvedValue(ok([]));
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.setBaseCurrency.mockResolvedValue(ok(null));
  mocks.connectorConnections.mockResolvedValue(ok([]));
  localStorage.removeItem("pcfo.onboardingPath");
  mocks.futureCashForecast.mockResolvedValue(
    ok({
      currency: "USD",
      starting_balance: usd(50_000),
      start_date: "2026-06-24",
      horizon_days: 30,
      days: [
        {
          date: "2026-06-24",
          closing: { p10: usd(50_000), p50: usd(50_000), p90: usd(50_000) },
          events: [],
        },
      ],
    }),
  );
  mocks.cashAvailability.mockResolvedValue(
    ok({
      currency: "USD",
      accounts: [],
      net_available: usd(50_000),
      net_committed: usd(0),
      net_headroom: usd(50_000),
      floor: usd(0),
      below_floor: false,
    }),
  );
});

describe("FirstForecastWizard", () => {
  it("opens on the welcome step with a currency control", async () => {
    renderWithClient(<FirstForecastWizard onClose={vi.fn()} />);
    expect(
      await screen.findByText(/build your first cash forecast/i),
    ).toBeInTheDocument();
    expect(screen.getByLabelText(/currency/i)).toHaveValue("USD");
  });

  it("skips setup from the header", async () => {
    const onClose = vi.fn();
    renderWithClient(<FirstForecastWizard onClose={onClose} />);
    fireEvent.click(await screen.findByRole("button", { name: /skip for now/i }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("advances to the forecast and lists the still-missing inputs", async () => {
    const onClose = vi.fn();
    renderWithClient(<FirstForecastWizard onClose={onClose} />);

    // Welcome -> Accounts.
    fireEvent.click(await screen.findByRole("button", { name: /next/i }));
    // The path step (kdw6): choose manual, then continue.
    fireEvent.click(
      await screen.findByRole("radio", { name: /enter and import myself/i }),
    );
    fireEvent.click(screen.getByRole("button", { name: /next/i }));
    // The already-present account is summarized; Next is enabled.
    expect(await screen.findByText(/Checking · \$500\.00/)).toBeInTheDocument();

    // Accounts -> Income (skip) -> Bills (skip) -> Forecast. The footer "Skip" is
    // matched exactly so it doesn't collide with the header's "Skip for now".
    fireEvent.click(screen.getByRole("button", { name: /next/i }));
    fireEvent.click(await screen.findByRole("button", { name: "Skip" }));
    fireEvent.click(await screen.findByRole("button", { name: "Skip" }));

    expect(
      await screen.findByRole("heading", { name: /your first forecast/i }),
    ).toBeInTheDocument();
    // Income and bills were skipped, so both are surfaced as next actions.
    expect(screen.getByText(/add your income/i)).toBeInTheDocument();
    expect(screen.getByText(/add recurring bills/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /go to dashboard/i }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("gates Next on the accounts step when the vault is empty", async () => {
    mocks.accountList.mockResolvedValue(ok([]));
    renderWithClient(<FirstForecastWizard onClose={vi.fn()} />);

    fireEvent.click(await screen.findByRole("button", { name: /next/i }));
    fireEvent.click(
      await screen.findByRole("radio", { name: /enter and import myself/i }),
    );
    fireEvent.click(screen.getByRole("button", { name: /next/i }));
    // On the accounts step with no accounts, advancing is disabled.
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /next/i })).toBeDisabled(),
    );
  });

  it("forks after the welcome step and gates Next until a path is chosen (kdw6)", async () => {
    renderWithClient(<FirstForecastWizard onClose={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: /next/i }));
    expect(await screen.findByText(/how will your money get in/i)).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /connect my banks/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /next/i })).toBeDisabled();
    fireEvent.click(screen.getByRole("radio", { name: /enter and import myself/i }));
    expect(screen.getByRole("button", { name: /next/i })).toBeEnabled();
    // The choice sticks for the next time the guide opens.
    expect(localStorage.getItem("pcfo.onboardingPath")).toBe("manual");
  });

  it("connected path shows the Bridge disclosures and the Connections card", async () => {
    renderWithClient(<FirstForecastWizard onClose={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: /next/i }));
    fireEvent.click(await screen.findByRole("radio", { name: /connect my banks/i }));
    fireEvent.click(screen.getByRole("button", { name: /next/i }));
    expect(await screen.findByText(/connect your banks/i)).toBeInTheDocument();
    // The four disclosures.
    expect(screen.getByText(/not affiliated with us/i)).toBeInTheDocument();
    expect(screen.getByText(/it costs money/i)).toBeInTheDocument();
    expect(screen.getByText(/it is optional/i)).toBeInTheDocument();
    expect(screen.getByText(/bridge\.simplefin\.org/i)).toBeInTheDocument();
    // The real link/map surface is right here, in its (empty) loaded state.
    expect(await screen.findByText("No connections yet")).toBeInTheDocument();
    expect(screen.getByText(/link a connection/i)).toBeInTheDocument();
    // …and the manual path stays one click away.
    expect(screen.getByRole("button", { name: /add an account by hand/i })).toBeInTheDocument();
  });

  it("reopens on the last chosen path (kdw6)", async () => {
    localStorage.setItem("pcfo.onboardingPath", "connected");
    renderWithClient(<FirstForecastWizard onClose={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: /next/i }));
    expect(
      await screen.findByRole("radio", { name: /connect my banks/i }),
    ).toBeChecked();
    expect(screen.getByRole("button", { name: /next/i })).toBeEnabled();
  });

  it("surfaces detected recurring charges on the Bills step with promote/dismiss (nhmsg)", async () => {
    mocks.recurringCandidates.mockResolvedValue(
      ok([
        {
          merchant_key: "netflix",
          display: "NETFLIX.COM",
          amount_minor: 1_599,
          amount_min_minor: 1_599,
          amount_max_minor: 1_599,
          currency: "USD",
          frequency: "monthly",
          last_seen: "2026-08-19",
          next_date: "2026-09-19",
          occurrence_count: 4,
          confidence_bps: 9000,
          dominant_category_id: null,
          source_account_names: ["Checking"],
          source_account_id: "acc-1",
          typical_day_of_month: 19,
          observations: [],
        } satisfies RecurringCandidateDto,
      ]),
    );
    renderWithClient(<FirstForecastWizard onClose={() => {}} />);
    // Welcome -> Path (manual) -> Accounts -> Income -> Bills.
    fireEvent.click(await screen.findByRole("button", { name: /next/i }));
    fireEvent.click(await screen.findByRole("radio", { name: /enter and import myself/i }));
    fireEvent.click(screen.getByRole("button", { name: /next/i }));
    await screen.findByText(/Checking · \$500\.00/);
    fireEvent.click(screen.getByRole("button", { name: /next/i }));
    fireEvent.click(await screen.findByRole("button", { name: "Skip" }));
    expect(await screen.findByText(/add your recurring bills/i)).toBeInTheDocument();
    // The shipped suggestion surface, wired in — nothing auto-created.
    expect(await screen.findByText(/NETFLIX\.COM/)).toBeInTheDocument();
    expect(mocks.createRecurringBill).not.toHaveBeenCalled();
    // Dismiss records the shared suppression (the row survives here only
    // because the mock keeps returning the candidate).
    fireEvent.click(screen.getByRole("button", { name: /dismiss netflix/i }));
    await waitFor(() => expect(mocks.dismissRecurringSuggestion).toHaveBeenCalledTimes(1));
    // Promoting opens the suggestion's own bill form BESIDE the wizard's: both
    // must keep working labels (per-instance ids — the IncomeForm lesson).
    fireEvent.click(await screen.findByRole("button", { name: /add as recurring/i }));
    expect(await screen.findAllByLabelText("Bill name")).toHaveLength(2);
  });

  it("manual path offers export guidance and an optional first import (lu4tm)", async () => {
    renderWithClient(<FirstForecastWizard onClose={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: /next/i }));
    fireEvent.click(await screen.findByRole("radio", { name: /enter and import myself/i }));
    fireEvent.click(screen.getByRole("button", { name: /next/i }));
    expect(await screen.findByText(/bring in your history/i)).toBeInTheDocument();
    // Guidance is right there, with the generic guide by default.
    expect(screen.getByLabelText(/where is the money/i)).toBeInTheDocument();
    // The account exists, so the import is one click away — and it is the
    // real import dialog, not a copy.
    fireEvent.click(screen.getByRole("button", { name: /import a file/i }));
    expect(await screen.findByRole("dialog", { name: /import file/i })).toBeInTheDocument();
    expect(mocks.importBatch).not.toHaveBeenCalled();
  });

  it("manual path holds the import until an account exists to import into", async () => {
    mocks.accountList.mockResolvedValue(ok([]));
    renderWithClient(<FirstForecastWizard onClose={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: /next/i }));
    fireEvent.click(await screen.findByRole("radio", { name: /enter and import myself/i }));
    fireEvent.click(screen.getByRole("button", { name: /next/i }));
    expect(await screen.findByRole("button", { name: /import a file/i })).toBeDisabled();
    expect(screen.getByText(/add the account the file belongs to first/i)).toBeInTheDocument();
  });
});
