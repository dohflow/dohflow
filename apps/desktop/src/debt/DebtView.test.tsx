import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { DebtView } from "./DebtView";

const mocks = vi.hoisted(() => ({
  cardStatementForecast: vi.fn(),
  futureCashForecast: vi.fn(),
  setCardStatementBalance: vi.fn(),
  cardStatementHistory: vi.fn(),
  debtPayoffComparison: vi.fn(),
  baseCurrency: vi.fn(),
  createScenario: vi.fn(),
  createForecastAssumption: vi.fn(),
  deleteScenario: vi.fn(),
  accountList: vi.fn(),
  transactionPage: vi.fn(),
  categoryList: vi.fn(),
  tagList: vi.fn(),
  transactionSplits: vi.fn(),
  spendByCategory: vi.fn(),
  cashFlowHistory: vi.fn(),
  futureCashByAccount: vi.fn(),
  debtTermsList: vi.fn(),
}));

vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.cardStatementForecast.mockResolvedValue(ok([]));
  mocks.futureCashForecast.mockResolvedValue(ok(null));
  mocks.cardStatementHistory.mockResolvedValue(ok([]));
  mocks.debtPayoffComparison.mockResolvedValue(ok([]));
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.accountList.mockResolvedValue(ok([]));
  mocks.transactionPage.mockResolvedValue(ok({ rows: [], total: 0 }));
  mocks.categoryList.mockResolvedValue(ok([]));
  mocks.tagList.mockResolvedValue(ok([]));
  mocks.transactionSplits.mockResolvedValue(ok([]));
  mocks.spendByCategory.mockResolvedValue(ok([]));
  mocks.cashFlowHistory.mockResolvedValue(
    ok({ currency: "USD", start_date: "2026-07-01", end_date: "2026-08-06", accounts: [] }),
  );
  mocks.debtTermsList.mockResolvedValue(ok([]));
  mocks.futureCashByAccount.mockResolvedValue(
    ok({ currency: "USD", start_date: "2026-08-06", horizon_days: 90, accounts: [], groups: [] }),
  );
});

describe("DebtView (personal-cfo-4d8.27.9.2, ADR 0049 §5 / ADR 0057)", () => {
  // A household WITH a debt account; the no-debt case is covered separately below.
  const anyDebt = {
    id: "card-1",
    name: "Visa",
    cashflow_role: "credit_facility",
    subtype: "credit_card",
    active: true,
    balance: { minor_units: -100_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
  };

  it("is the home for debt analysis — cards and payoff render here", async () => {
    mocks.accountList.mockResolvedValue(ok([anyDebt]));
    renderWithClient(<DebtView />);
    expect(
      await screen.findByRole("heading", { name: "Debt" }),
    ).toBeInTheDocument();
    // Both surfaces that moved off Accounts are present.
    expect(await screen.findByText(/debt payoff plans/i)).toBeInTheDocument();
  });

  it("reads the debt data itself rather than depending on Accounts having loaded it", async () => {
    mocks.accountList.mockResolvedValue(ok([anyDebt]));
    // The move is a MOVE: these surfaces must stand up on their own page. If they only
    // worked because Accounts had already fetched something, the relocation would look
    // fine in isolation and break in the app.
    renderWithClient(<DebtView />);
    await screen.findByRole("heading", { name: "Debt" });
    expect(mocks.cardStatementForecast).toHaveBeenCalled();
    expect(mocks.debtPayoffComparison).toHaveBeenCalled();
  });
});

describe("the debt account selector (personal-cfo-4d8.27.9.3, ADR 0057 §1)", () => {
  const card = (over = {}) => ({
    id: "card-1",
    name: "Visa",
    cashflow_role: "credit_facility",
    subtype: "credit_card",
    active: true,
    balance: { minor_units: -100_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  });
  const cycle = (accountId: string, name: string) => ({
    account_id: accountId,
    account_name: name,
    currency: "USD",
    credit_limit_minor: 1_000_000,
    repayment_philosophy: "pay_statement_balance",
    estimate_basis: "card_history",
    estimate_mape_bps: 0,
    estimate_sample_cycles: 1,
    stored_statements: [],
    cycles: [],
  });

  it("defaults to all debts, and scopes the page when one is unticked", async () => {
    mocks.accountList.mockResolvedValue(
      ok([card(), card({ id: "loan-1", name: "Car loan", cashflow_role: "loan_liability", subtype: "auto_loan" })]),
    );
    mocks.cardStatementForecast.mockResolvedValue(ok([cycle("card-1", "Visa")]));
    renderWithClient(<DebtView />);

    // Defaulting to all is the contract: the page opens on "your debt", not an empty
    // state asking you to choose — and now says so in words, with nothing to open.
    expect(
      await screen.findByText("Everything below covers all 2 of your debts."),
    ).toBeInTheDocument();

    // The chips are the control; no dropdown to open.
    fireEvent.click(screen.getByRole("button", { name: /Car loan/i }));
    // Unticking the loan leaves the card — NOT "only the loan". An empty selection means
    // ALL, so the first tick has to start from everything and remove.
    expect(
      await screen.findByText("Everything below covers Visa only."),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.transactionPage).toHaveBeenLastCalledWith(
        expect.objectContaining({ account_ids: ["card-1"] }),
      ),
    );
  });

  it("shows the scope even for a single debt", async () => {
    // With one chip the bar is a STATEMENT of scope rather than a control, which is worth
    // more than the saved row — the old dropdown hid itself here and left the page
    // silent about what it covered.
    mocks.accountList.mockResolvedValue(ok([card()]));
    mocks.cardStatementForecast.mockResolvedValue(ok([cycle("card-1", "Visa")]));
    renderWithClient(<DebtView />);
    expect(
      await screen.findByText("Everything below covers your one debt."),
    ).toBeInTheDocument();
  });

  it("says nothing changed on disk when the read fails", async () => {
    // A failed read used to render empty surfaces, which reads as "you have no debt"
    // rather than "we could not look".
    mocks.accountList.mockResolvedValue({
      status: "error",
      error: { kind: "Internal", message: "boom" },
    });
    renderWithClient(<DebtView />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Could not read your debt data.");
    expect(alert).toHaveTextContent(/nothing has changed on disk/i);
  });

  it("says so when the household has no debt accounts", async () => {
    mocks.accountList.mockResolvedValue(ok([]));
    renderWithClient(<DebtView />);
    expect(
      await screen.findByText(/no credit cards or loans yet/i),
    ).toBeInTheDocument();
  });

  it("excludes archived debts from the scope bar", async () => {
    // Same reasoning as ADR 0056: a retired debt is not part of the picture, and
    // offering it would invite selecting a debt that no longer exists.
    mocks.accountList.mockResolvedValue(
      ok([card(), card({ id: "old-1", name: "Closed card", active: false })]),
    );
    mocks.cardStatementForecast.mockResolvedValue(ok([cycle("card-1", "Visa")]));
    renderWithClient(<DebtView />);
    expect(
      await screen.findByText("Everything below covers your one debt."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Closed card")).not.toBeInTheDocument();
  });
});

describe("the embedded transaction list is pinned to the selection (4d8.27.9.5, ADR 0057 §3)", () => {
  const debt = (over = {}) => ({
    id: "card-1",
    name: "Visa",
    cashflow_role: "credit_facility",
    subtype: "credit_card",
    active: true,
    balance: { minor_units: -100_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  });

  it("queries only the selected debt accounts", async () => {
    mocks.accountList.mockResolvedValue(
      ok([
        debt(),
        debt({ id: "loan-1", name: "Car loan", cashflow_role: "loan_liability", subtype: "auto_loan" }),
      ]),
    );
    renderWithClient(<DebtView />);
    await screen.findByRole("heading", { name: "Debt" });

    // Defaulting to all debts means BOTH ids — and, critically, not the household's
    // other accounts.
    await waitFor(() =>
      expect(mocks.transactionPage).toHaveBeenCalledWith(
        expect.objectContaining({ account_ids: ["card-1", "loan-1"] }),
      ),
    );
  });

  it("does not offer an account facet that could widen past the scope", async () => {
    // The page's selector owns the account scope (ADR 0057 §3). A second control in the
    // embedded filter bar is how two controls come to disagree about what is shown — and
    // "Clear filters" resetting accountIds to [] would mean ALL accounts, silently
    // showing the whole vault under a heading that says "Activity on these debts".
    mocks.accountList.mockResolvedValue(ok([debt()]));
    renderWithClient(<DebtView />);
    await screen.findByRole("heading", { name: "Debt" });

    fireEvent.click(await screen.findByRole("button", { name: /^filters$/i }));
    // The embedded bar has no Account facet at all.
    expect(
      screen.queryByRole("combobox", { name: "Account" }),
    ).not.toBeInTheDocument();

    // …and clearing keeps the scope rather than widening to everything.
    const clear = screen.queryByRole("button", { name: /clear/i });
    if (clear) fireEvent.click(clear);
    await waitFor(() =>
      expect(mocks.transactionPage).toHaveBeenLastCalledWith(
        expect.objectContaining({ account_ids: ["card-1"] }),
      ),
    );
  });
});

describe("visualizations are chosen by account role (4d8.27.9.6, ADR 0057 §2)", () => {
  const acct = (over = {}) => ({
    id: "card-1",
    name: "Visa",
    cashflow_role: "credit_facility",
    subtype: "credit_card",
    active: true,
    balance: { minor_units: -100_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  });

  it("gives cards a spending breakdown and loans none", async () => {
    // The breakdown is the thing you can only ask of an account you spend FROM. A loan
    // has no card spending to break down, so offering the section would be furniture.
    mocks.accountList.mockResolvedValue(
      ok([acct({ id: "loan-1", name: "Car loan", cashflow_role: "loan_liability", subtype: "auto_loan" })]),
    );
    renderWithClient(<DebtView />);
    await screen.findByRole("heading", { name: "Debt" });

    // The page heading renders before accounts land, so wait for the section itself.
    expect(await screen.findByText("Loans")).toBeInTheDocument();
    expect(screen.queryByText("Credit cards")).not.toBeInTheDocument();
    expect(screen.queryByText(/where the card spending went/i)).not.toBeInTheDocument();
  });

  it("renders BOTH sections for a selection spanning cards and loans", async () => {
    // ADR 0057 §2: compose per role rather than forcing a revolving balance and an
    // amortizing balance onto one axis.
    mocks.accountList.mockResolvedValue(
      ok([
        acct(),
        acct({ id: "loan-1", name: "Car loan", cashflow_role: "loan_liability", subtype: "auto_loan" }),
      ]),
    );
    renderWithClient(<DebtView />);
    await screen.findByRole("heading", { name: "Debt" });

    expect(await screen.findByText("Credit cards")).toBeInTheDocument();
    expect(screen.getByText("Loans")).toBeInTheDocument();
  });

  it("scopes the card spending breakdown to the selected cards, as a set", async () => {
    // This is what widening spend_by_category to an account SET was for
    // (personal-cfo-4d8.27.9.4). Charting one card while the list below showed several
    // is the disagreement ADR 0052 §2 forbids.
    mocks.accountList.mockResolvedValue(
      ok([acct(), acct({ id: "card-2", name: "Amex" })]),
    );
    renderWithClient(<DebtView />);
    await screen.findByRole("heading", { name: "Debt" });

    await waitFor(() =>
      expect(mocks.spendByCategory).toHaveBeenCalledWith(
        expect.objectContaining({ account_ids: ["card-1", "card-2"] }),
      ),
    );
  });
});

describe("payoff tools scope to the selection (4d8.27.9.7, ADR 0057 §1)", () => {
  const acct = (over = {}) => ({
    id: "card-1",
    name: "Visa",
    cashflow_role: "credit_facility",
    subtype: "credit_card",
    active: true,
    balance: { minor_units: -100_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  });

  it("passes the selected debts into the payoff simulation", async () => {
    // The scope reaches the SIMULATION, not its output: snowball and avalanche order the
    // debts and route the extra budget among them, so filtering plans afterwards would
    // report a payoff order and a debt-free month the selection does not have.
    mocks.accountList.mockResolvedValue(
      ok([
        acct(),
        acct({ id: "loan-1", name: "Car loan", cashflow_role: "loan_liability", subtype: "auto_loan" }),
      ]),
    );
    renderWithClient(<DebtView />);
    await screen.findByRole("heading", { name: "Debt" });

    await waitFor(() =>
      expect(mocks.debtPayoffComparison).toHaveBeenCalledWith(
        expect.any(Number),
        ["card-1", "loan-1"],
      ),
    );
  });
});

describe("stat row + balances-and-terms table (personal-cfo-g43x)", () => {
  const card = (over = {}) => ({
    id: "card-1",
    name: "Visa",
    cashflow_role: "credit_facility",
    subtype: "credit_card",
    active: true,
    balance: { minor_units: -500_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  });
  const cardTerms = (over = {}) => ({
    account_id: "card-1",
    apr_bps: 2199,
    statement_close_day: 5,
    payment_due_day: 25,
    grace_period_days: 21,
    credit_limit_minor: 1_000_000,
    repayment_philosophy: "pay_minimum",
    fixed_amount_minor: null,
    min_payment_percent_bps: null,
    min_payment_floor_minor: null,
    paying_source_account_id: null,
    original_principal_minor: null,
    ...over,
  });

  it("states what the scoped debts cost right now", async () => {
    mocks.accountList.mockResolvedValue(ok([card()]));
    mocks.debtTermsList.mockResolvedValue(ok([cardTerms()]));
    renderWithClient(<DebtView />);

    // Scope the assertions to the table/stat area — the scope bar also shows the balance,
    // so a bare getByText("$5,000.00") would pass off the chip and prove nothing.
    expect(await screen.findByText("Balances and terms")).toBeInTheDocument();
    expect(screen.getAllByText("21.99%").length).toBeGreaterThan(0);
    // 1% of $5,000 = $50 minimum (the ADR 0035 §5 default, since nothing is recorded).
    expect(screen.getAllByText("$50.00").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Day 25").length).toBeGreaterThan(0);
    expect(screen.getByText("Total owed")).toBeInTheDocument();
  });

  it("says a total is not known rather than omitting a debt from it", async () => {
    // A debt with no terms on record cannot contribute a minimum or an interest figure.
    // Quietly leaving it out would produce a confident total that is too low.
    mocks.accountList.mockResolvedValue(
      ok([card(), card({ id: "card-2", name: "Store card" })]),
    );
    mocks.debtTermsList.mockResolvedValue(ok([cardTerms()]));
    renderWithClient(<DebtView />);

    await screen.findByText("Balances and terms");
    expect(screen.getAllByText("Not known").length).toBeGreaterThan(0);
    expect(screen.getByText("A debt has no terms recorded")).toBeInTheDocument();
  });

  it("does not render the stat row before the terms arrive", async () => {
    // A stat row totalling a half-loaded set reads as final while being wrong.
    let resolve: ((v: unknown) => void) | undefined;
    mocks.accountList.mockResolvedValue(ok([card()]));
    mocks.debtTermsList.mockReturnValue(
      new Promise((r) => {
        resolve = r;
      }),
    );
    renderWithClient(<DebtView />);
    await screen.findByRole("heading", { name: "Debt" });
    expect(screen.queryByText("Balances and terms")).not.toBeInTheDocument();

    resolve?.(ok([cardTerms()]));
    expect(await screen.findByText("Balances and terms")).toBeInTheDocument();
  });
});

describe("projected statement + section order (personal-cfo-08fj)", () => {
  const card = (over = {}) => ({
    id: "card-1",
    name: "Visa",
    cashflow_role: "credit_facility",
    subtype: "credit_card",
    active: true,
    balance: { minor_units: -500_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  });
  const cardTerms = () => ({
    account_id: "card-1",
    apr_bps: 2199,
    statement_close_day: 5,
    payment_due_day: 25,
    grace_period_days: 21,
    credit_limit_minor: 1_000_000,
    repayment_philosophy: "pay_minimum",
    fixed_amount_minor: null,
    min_payment_percent_bps: null,
    min_payment_floor_minor: null,
    paying_source_account_id: null,
    original_principal_minor: null,
  });
  const forecastWithCycle = () => ({
    account_id: "card-1",
    account_name: "Visa",
    currency: "USD",
    credit_limit_minor: 1_000_000,
    repayment_philosophy: "pay_minimum",
    estimate_basis: "card_history",
    estimate_mape_bps: 0,
    estimate_sample_cycles: 1,
    stored_statements: [],
    window_open: "2026-08-01",
    cycles: [
      {
        close_date: "2026-08-05",
        due_date: "2026-08-25",
        carried_opening_balance_minor: 400_000,
        known_charges_minor: 50_000,
        projected_variable_minor: 30_000,
        accrued_interest_minor: 9_000,
        statement_balance_minor: 489_000,
        minimum_due_minor: 4_890,
      },
    ],
  });

  it("shows the next cycle's projected statement in the terms table", async () => {
    mocks.accountList.mockResolvedValue(ok([card()]));
    mocks.debtTermsList.mockResolvedValue(ok([cardTerms()]));
    mocks.cardStatementForecast.mockResolvedValue(ok([forecastWithCycle()]));
    renderWithClient(<DebtView />);

    await screen.findByText("Balances and terms");
    // The NEXT cycle's statement — the one about to close, not a later one.
    expect(screen.getAllByText("$4,890.00").length).toBeGreaterThan(0);
  });

  it("does not invent a statement for a loan", async () => {
    // A loan has no statement at all; a card with no recorded cycle has none to project.
    // Neither has a figure, and inventing one would be worse than saying so.
    mocks.accountList.mockResolvedValue(
      ok([card({ id: "loan-1", name: "Car loan", cashflow_role: "loan_liability", subtype: "auto_loan" })]),
    );
    mocks.debtTermsList.mockResolvedValue(
      ok([{ ...cardTerms(), account_id: "loan-1" }]),
    );
    mocks.cardStatementForecast.mockResolvedValue(ok([]));
    renderWithClient(<DebtView />);

    await screen.findByText("Balances and terms");
    expect(screen.getByText("Projected statement")).toBeInTheDocument();
    expect(screen.getAllByText("—").length).toBeGreaterThan(0);
  });

  it("puts payoff above the backward-looking sections", async () => {
    // The mock's ordering rationale: paydown and payoff answer "how does this end", which
    // is what the page is for; spending and activity are context for it.
    mocks.accountList.mockResolvedValue(ok([card()]));
    mocks.debtTermsList.mockResolvedValue(ok([cardTerms()]));
    renderWithClient(<DebtView />);

    const payoff = await screen.findByText(/debt payoff plans/i);
    const activity = await screen.findByText("Activity on these debts");
    // compareDocumentPosition: FOLLOWING (4) means activity comes after payoff.
    expect(
      payoff.compareDocumentPosition(activity) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("has no standalone card-spending breakdown (removed as redundant, emgh)", async () => {
    mocks.accountList.mockResolvedValue(ok([card()]));
    mocks.debtTermsList.mockResolvedValue(ok([cardTerms()]));
    renderWithClient(<DebtView />);
    // Anchor on the visualizations card's OWN subtitle so the absence checks
    // run after it has actually mounted (review: an earlier anchor resolved
    // against DebtPayoffCompare's copy before the async accounts landed,
    // making the assertions vacuously green against the old code).
    expect(await screen.findByText(/what you have owed/i)).toBeInTheDocument();
    expect(screen.queryByText(/where the card spending went/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/card debts only/i)).not.toBeInTheDocument();
  });
});
