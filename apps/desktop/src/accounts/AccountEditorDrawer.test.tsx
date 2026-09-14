import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";
import type { AccountViewDto } from "@/bindings";
import { AccountEditorDrawer } from "./AccountEditorDrawer";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  createAccount: vi.fn(),
  updateAccount: vi.fn(),
  setAccountSubtype: vi.fn(),
  setAccountNote: vi.fn(),
  setAccountLink: vi.fn(),
  assertBalance: vi.fn(),
  debtTerms: vi.fn(),
  setDebtTerms: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    createAccount: mocks.createAccount,
    updateAccount: mocks.updateAccount,
    setAccountSubtype: mocks.setAccountSubtype,
    setAccountNote: mocks.setAccountNote,
    setAccountLink: mocks.setAccountLink,
    assertBalance: mocks.assertBalance,
    debtTerms: mocks.debtTerms,
    setDebtTerms: mocks.setDebtTerms,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.accountList.mockResolvedValue(ok([]));
  mocks.debtTerms.mockResolvedValue(ok(null));
  mocks.createAccount.mockResolvedValue(
    ok({ account_id: "new-id", mutation: { op_seq: 1, replayed: false } }),
  );
  mocks.setDebtTerms.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
  mocks.setAccountLink.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
});

function open() {
  renderWithClient(
    <AccountEditorDrawer account={null} defaultCurrency="USD" onClose={vi.fn()} />,
  );
}

test("create mode shows no Debt section for a cash account", () => {
  open();
  expect(screen.getByRole("dialog", { name: /new account/i })).toBeInTheDocument();
  expect(screen.queryByText("Debt")).not.toBeInTheDocument();
});

test("choosing a credit-card type reveals the inline Debt section", () => {
  open();
  fireEvent.change(screen.getByLabelText("Type"), {
    target: { value: "CreditFacility" },
  });
  expect(screen.getByText("Debt")).toBeInTheDocument();
  expect(screen.getByText("APR (%)")).toBeInTheDocument();
  expect(screen.getByText(/credit limit/i)).toBeInTheDocument();
});

test("a property account labels its figure 'Value'", () => {
  open();
  fireEvent.change(screen.getByLabelText("Type"), {
    target: { value: "RealAsset" },
  });
  expect(screen.getByText(/opening value/i)).toBeInTheDocument();
});

test("creating an account sends the chosen role", async () => {
  open();
  fireEvent.change(screen.getByLabelText("Account name"), {
    target: { value: "Roth IRA" },
  });
  fireEvent.change(screen.getByLabelText("Type"), {
    target: { value: "InvestmentAsset" },
  });
  fireEvent.click(screen.getByRole("button", { name: /add account/i }));
  await waitFor(() =>
    expect(mocks.createAccount).toHaveBeenCalledWith(
      expect.objectContaining({ name: "Roth IRA", cashflow_role: "InvestmentAsset" }),
    ),
  );
});

test("a liability is entered as a positive amount owed and stored negative", async () => {
  open();
  fireEvent.change(screen.getByLabelText("Account name"), {
    target: { value: "Sapphire" },
  });
  fireEvent.change(screen.getByLabelText("Type"), {
    target: { value: "CreditFacility" },
  });
  // The figure field is labeled "Amount owed" for a liability, not "Balance".
  expect(screen.getByText("Opening amount owed")).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Opening amount owed"), {
    target: { value: "10000" },
  });
  fireEvent.click(screen.getByRole("button", { name: /add account/i }));
  await waitFor(() =>
    expect(mocks.createAccount).toHaveBeenCalledWith(
      expect.objectContaining({
        cashflow_role: "CreditFacility",
        // Entered $10,000 owed → stored as −1,000,000 minor (credit normal balance).
        opening_balance: expect.objectContaining({ minor_units: -1_000_000 }),
      }),
    ),
  );
});

test("editing a liability shows the owed amount as positive", () => {
  renderWithClient(
    <AccountEditorDrawer
      account={{
        id: "card-1",
        name: "Sapphire",
        cashflow_role: "credit_facility",
        subtype: "credit_card",
        active: true,
        balance: { minor_units: -1_000_000, currency: "USD" },
        notes: null,
        linked_account_id: null,
        linked_account_name: null,
      }}
      defaultCurrency="USD"
      onClose={vi.fn()}
    />,
  );
  const figure = screen.getByLabelText("Amount owed") as HTMLInputElement;
  expect(figure.value).toBe("10000");
});

test("a property account shows the Link section; a cash account does not", () => {
  open();
  // Cash (default) has no Link section.
  expect(screen.queryByText("Link")).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Type"), {
    target: { value: "RealAsset" },
  });
  expect(screen.getByText("Link")).toBeInTheDocument();
  expect(screen.getByText("Financed by")).toBeInTheDocument();
});

test("the Link picker lists only loans (not credit cards)", async () => {
  const liability = (
    id: string,
    name: string,
    role: string,
  ): AccountViewDto => ({
    id,
    name,
    cashflow_role: role,
    subtype: null,
    active: true,
    balance: { minor_units: -30_000_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
  });
  mocks.accountList.mockResolvedValue(
    ok([
      liability("mortgage-1", "Home mortgage", "loan_liability"),
      liability("card-1", "Sapphire card", "credit_facility"),
    ]),
  );
  open();
  fireEvent.change(screen.getByLabelText("Type"), {
    target: { value: "RealAsset" },
  });
  // A loan is offered; a credit card is not a valid financing target (4d8.23.4).
  expect(
    await screen.findByRole("option", { name: "Home mortgage" }),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("option", { name: "Sapphire card" }),
  ).not.toBeInTheDocument();
});

test("New loan opens an inline create that adds a loan_liability", async () => {
  mocks.accountList.mockResolvedValue(ok([]));
  mocks.createAccount.mockResolvedValue(
    ok({ account_id: "loan-new", mutation: { op_seq: 1, replayed: false } }),
  );
  open();
  fireEvent.change(screen.getByLabelText("Type"), {
    target: { value: "RealAsset" },
  });
  fireEvent.click(screen.getByRole("button", { name: /new loan/i }));
  const dialog = await screen.findByRole("dialog", { name: /new loan/i });
  fireEvent.change(within(dialog).getByLabelText("Loan name"), {
    target: { value: "Auto loan" },
  });
  fireEvent.change(within(dialog).getByLabelText("Amount owed"), {
    target: { value: "25000" },
  });
  fireEvent.click(within(dialog).getByRole("button", { name: /create loan/i }));
  await waitFor(() =>
    expect(mocks.createAccount).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "Auto loan",
        cashflow_role: "LoanLiability",
        // $25,000 owed → stored −2,500,000 (liability negative balance).
        opening_balance: expect.objectContaining({ minor_units: -2_500_000 }),
      }),
    ),
  );
});

test("a loan account offers Original principal, not Credit limit", () => {
  open();
  fireEvent.change(screen.getByLabelText("Type"), {
    target: { value: "LoanLiability" },
  });
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getByText(/original principal/i)).toBeInTheDocument();
  expect(within(dialog).queryByText(/credit limit/i)).not.toBeInTheDocument();
});
