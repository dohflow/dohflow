import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto } from "@/bindings";
import { AccountsView } from "./AccountsView";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  createAccount: vi.fn(),
  updateAccount: vi.fn(),
  archiveAccount: vi.fn(),
  reinstateAccount: vi.fn(),
  assertBalance: vi.fn(),
  setAccountSubtype: vi.fn(),
  setAccountNote: vi.fn(),
  setAccountLink: vi.fn(),
  debtTerms: vi.fn(),
  setDebtTerms: vi.fn(),
  cashTiers: vi.fn(),
  baseCurrency: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    createAccount: mocks.createAccount,
    updateAccount: mocks.updateAccount,
    archiveAccount: mocks.archiveAccount,
    reinstateAccount: mocks.reinstateAccount,
    assertBalance: mocks.assertBalance,
    setAccountSubtype: mocks.setAccountSubtype,
    setAccountNote: mocks.setAccountNote,
    setAccountLink: mocks.setAccountLink,
    debtTerms: mocks.debtTerms,
    setDebtTerms: mocks.setDebtTerms,
    cashTiers: mocks.cashTiers,
    baseCurrency: mocks.baseCurrency,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const money = (minor: number) => ({ minor_units: minor, currency: "USD" });

function account(over: Partial<AccountViewDto> = {}): AccountViewDto {
  return {
    id: crypto.randomUUID(),
    name: "Checking",
    cashflow_role: "liquid_cash",
    subtype: null,
    active: true,
    balance: money(125_000),
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  };
}

const CHECKING = account({ name: "Checking", balance: money(200_000) });
const PROPERTY = account({
  name: "Our house",
  cashflow_role: "real_asset",
  subtype: "property",
  balance: money(5_000_000),
});
const CARD = account({
  name: "Sapphire",
  cashflow_role: "credit_facility",
  subtype: "credit_card",
  balance: money(-40_000),
});

beforeEach(() => {
  vi.clearAllMocks();
  mocks.accountList.mockResolvedValue(ok([CHECKING, PROPERTY, CARD]));
  mocks.cashTiers.mockResolvedValue(
    ok({ spendable: money(200_000), reserve: money(0), net: money(200_000) }),
  );
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.debtTerms.mockResolvedValue(ok(null));
  mocks.createAccount.mockResolvedValue(
    ok({ account_id: "new-id", mutation: { op_seq: 1, replayed: false } }),
  );
  mocks.archiveAccount.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
  mocks.reinstateAccount.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
});

test("shows the net-worth summary computed from assets minus liabilities", async () => {
  renderWithClient(<AccountsView />);
  // Net worth = 2000 + 50000 (assets) − 400 (card) = 51,600.00 (awaited: the value
  // fills in once the account list resolves).
  expect(await screen.findByText("$51,600.00")).toBeInTheDocument();
  expect(screen.getByText("Net worth")).toBeInTheDocument();
  expect(screen.getByText("Total assets")).toBeInTheDocument();
  expect(screen.getByText("Total liabilities")).toBeInTheDocument();
  expect(screen.getByText("Cash on hand")).toBeInTheDocument();
});

test("splits accounts into Assets and Liabilities columns", async () => {
  renderWithClient(<AccountsView />);
  const assets = (await screen.findByText("Assets")).closest("section")!;
  const liabilities = screen.getByText("Liabilities").closest("section")!;
  expect(within(assets).getByText("Checking")).toBeInTheDocument();
  expect(within(assets).getByText("Our house")).toBeInTheDocument();
  expect(within(liabilities).getByText("Sapphire")).toBeInTheDocument();
  // The card is a liability, not an asset.
  expect(within(assets).queryByText("Sapphire")).not.toBeInTheDocument();
});

test("Add account opens the unified editor in create mode", async () => {
  renderWithClient(<AccountsView />);
  fireEvent.click(await screen.findByRole("button", { name: /add account/i }));
  expect(
    await screen.findByRole("dialog", { name: /new account/i }),
  ).toBeInTheDocument();
});

test("clicking an account row opens the editor for that account", async () => {
  renderWithClient(<AccountsView />);
  fireEvent.click(await screen.findByText("Our house"));
  const dialog = await screen.findByRole("dialog", { name: /edit our house/i });
  // Real-asset editor labels the figure "Value", not "Balance".
  expect(within(dialog).getByText("Value")).toBeInTheDocument();
});

test("a liability row shows the amount owed as a positive number", async () => {
  renderWithClient(<AccountsView />);
  const liabilities = (await screen.findByText("Liabilities")).closest("section")!;
  // The Sapphire card is stored at −$400 but shown as a positive $400 owed (both the
  // column total and the row are the positive amount; neither is negative).
  expect(within(liabilities).getAllByText("$400.00").length).toBeGreaterThan(0);
  expect(within(liabilities).queryByText("-$400.00")).not.toBeInTheDocument();
});

test("groups accounts into per-kind sections with icon headers", async () => {
  renderWithClient(<AccountsView />);
  // Section headers by kind (4d8.23.8): Cash / Property & vehicles / Credit cards.
  expect(
    await screen.findByRole("button", { name: /^Cash section$/i }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: /Property & vehicles section/i }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: /Credit cards section/i }),
  ).toBeInTheDocument();
});

test("a section collapses when its header is clicked", async () => {
  renderWithClient(<AccountsView />);
  const cashHeader = await screen.findByRole("button", { name: /^Cash section$/i });
  expect(screen.getByText("Checking")).toBeInTheDocument();
  fireEvent.click(cashHeader);
  await waitFor(() =>
    expect(screen.queryByText("Checking")).not.toBeInTheDocument(),
  );
});

test("archiving a liquid account that still holds money says what it costs first", async () => {
  // personal-cfo-tiqf. ADR 0056 made archived liquid accounts leave the forecast — the
  // safe direction of error — but the accepted cost is that archiving an account with
  // money in it now UNDERSTATES cash. Doing that silently is the failure: the projection
  // drops with no explanation and the forecast looks broken.
  renderWithClient(<AccountsView />);
  fireEvent.click(await screen.findByRole("button", { name: /Archive Checking/i }));

  // Nothing happens until the user confirms…
  expect(mocks.archiveAccount).not.toHaveBeenCalled();
  const notice = await screen.findByRole("alertdialog", { name: /Archive Checking/i });
  // …and the notice names the amount and says restoring brings it back.
  expect(notice).toHaveTextContent("$2,000.00");
  expect(notice).toHaveTextContent(/toward your forecast/i);
  expect(notice).toHaveTextContent(/restoring the account brings it back/i);

  fireEvent.click(screen.getByRole("button", { name: /Confirm archive Checking/i }));
  await waitFor(() =>
    expect(mocks.archiveAccount).toHaveBeenCalledWith(CHECKING.id, expect.anything()),
  );
});

test("cancelling the archive notice leaves the account alone", async () => {
  renderWithClient(<AccountsView />);
  fireEvent.click(await screen.findByRole("button", { name: /Archive Checking/i }));
  await screen.findByRole("alertdialog", { name: /Archive Checking/i });
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await waitFor(() =>
    expect(
      screen.queryByRole("alertdialog", { name: /Archive Checking/i }),
    ).not.toBeInTheDocument(),
  );
  expect(mocks.archiveAccount).not.toHaveBeenCalled();
});

test("archiving an empty account keeps the one-click path", async () => {
  // A zero balance moves no projection, so explaining a consequence that will not happen
  // would be friction that teaches nothing.
  mocks.accountList.mockResolvedValue(
    ok([account({ name: "Old checking", balance: money(0) })]),
  );
  renderWithClient(<AccountsView />);
  fireEvent.click(await screen.findByRole("button", { name: /Archive Old checking/i }));
  await waitFor(() => expect(mocks.archiveAccount).toHaveBeenCalled());
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
});

test("archiving a card does not explain a forecast change that will not happen", async () => {
  // Scoped to liquid_cash: only those balances anchor the projection (ADR 0056).
  mocks.accountList.mockResolvedValue(
    ok([
      account({
        name: "Visa",
        cashflow_role: "credit_facility",
        subtype: "credit_card",
        balance: money(-50_000),
      }),
    ]),
  );
  renderWithClient(<AccountsView />);
  fireEvent.click(await screen.findByRole("button", { name: /Archive Visa/i }));
  await waitFor(() => expect(mocks.archiveAccount).toHaveBeenCalled());
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
});

test("a row's edit control opens the editor", async () => {
  renderWithClient(<AccountsView />);
  fireEvent.click(await screen.findByRole("button", { name: /Edit Checking/i }));
  expect(
    await screen.findByRole("dialog", { name: /edit Checking/i }),
  ).toBeInTheDocument();
});

test("Update balances edits inline in the grouped layout, no layout swap (personal-cfo-4d8.25.24)", async () => {
  renderWithClient(<AccountsView />);
  await screen.findByText("Checking");
  // No Debt / Update-balances TAB — it is an inline toggle now (4d8.23.6/.7).
  expect(screen.queryByRole("button", { name: /^Debt$/i })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /update balances/i }));
  expect(
    await screen.findByRole("button", { name: /done updating/i }),
  ).toBeInTheDocument();
  // The SAME grouped rows stay put; only the value cell becomes an input, and the
  // As-of / Save controls appear.
  expect(screen.getByText("Checking")).toBeInTheDocument();
  expect(screen.getByLabelText("New balance for Checking")).toBeInTheDocument();
  expect(screen.getByLabelText("As of")).toBeInTheDocument();
});

test("leaving batch mode discards unsaved drafts — they do not resurrect (4d8.25.24 review)", async () => {
  renderWithClient(<AccountsView />);
  await screen.findByText("Checking");
  fireEvent.click(screen.getByRole("button", { name: /update balances/i }));
  const input = await screen.findByLabelText("New balance for Checking");
  fireEvent.change(input, { target: { value: "9999" } });
  // Exit batch mode WITHOUT saving, then re-enter.
  fireEvent.click(screen.getByRole("button", { name: /done updating/i }));
  fireEvent.click(screen.getByRole("button", { name: /update balances/i }));
  const reopened = await screen.findByLabelText("New balance for Checking");
  // The stale "9999" draft is gone — the field shows the account's real balance.
  expect(reopened).not.toHaveValue("9999");
  expect(screen.getByText(/0 changes to save/i)).toBeInTheDocument();
});

test("inline batch edit saves through assertBalance with the as-of date (personal-cfo-4d8.25.24)", async () => {
  mocks.assertBalance.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
  renderWithClient(<AccountsView />);
  await screen.findByText("Checking");
  fireEvent.click(screen.getByRole("button", { name: /update balances/i }));
  const input = await screen.findByLabelText("New balance for Checking");
  fireEvent.change(input, { target: { value: "2500" } });
  fireEvent.click(screen.getByRole("button", { name: /^save/i }));
  await waitFor(() => expect(mocks.assertBalance).toHaveBeenCalledTimes(1));
  const arg = mocks.assertBalance.mock.calls[0]?.[0];
  expect(arg.amount.minor_units).toBe(250_000);
  expect(arg.as_of_date).toBeTruthy();
});

test("debt analysis is NOT on Accounts — it moved to the Debt page", async () => {
  // ADR 0049 §5 supersedes ADR 0037's addendum: Accounts owns account identity and
  // balances, the Debt page owns debt ANALYSIS. This asserted the opposite until
  // personal-cfo-4d8.27.9.2 moved the surfaces.
  //
  // Kept rather than deleted, and inverted: a surface showing the same analysis in two
  // places is how two copies drift apart, so the absence is worth guarding.
  renderWithClient(<AccountsView />);
  await screen.findByRole("heading", { name: /accounts/i });
  expect(screen.queryByRole("button", { name: /debt insights/i })).not.toBeInTheDocument();
  expect(screen.queryByText(/payoff/i)).not.toBeInTheDocument();
});

test("a linked account shows a 'Linked to' chip", async () => {
  mocks.accountList.mockResolvedValue(
    ok([
      account({
        name: "Our house",
        cashflow_role: "real_asset",
        subtype: "property",
        balance: money(5_000_000),
        linked_account_id: "mortgage-1",
        linked_account_name: "Home mortgage",
      }),
      account({
        name: "Home mortgage",
        cashflow_role: "loan_liability",
        subtype: "mortgage",
        balance: money(-3_000_000),
        linked_account_name: "Our house",
      }),
    ]),
  );
  renderWithClient(<AccountsView />);
  // Both sides show the link (asset -> liability, liability -> asset).
  expect(await screen.findByText(/Linked to Home mortgage/)).toBeInTheDocument();
  expect(screen.getByText(/Linked to Our house/)).toBeInTheDocument();
});

test("a section's own search narrows just that section", async () => {
  mocks.accountList.mockResolvedValue(
    ok([
      account({ name: "Checking", balance: money(200_000) }),
      account({ name: "Emergency savings", cashflow_role: "liquid_cash", subtype: "savings" }),
    ]),
  );
  renderWithClient(<AccountsView />);
  await screen.findByText("Checking");
  // Open the Cash section's own filter/sort, then search within it.
  fireEvent.click(screen.getByRole("button", { name: /Filter and sort Cash/i }));
  fireEvent.change(screen.getByRole("searchbox", { name: /Search Cash/i }), {
    target: { value: "emergency" },
  });
  await waitFor(() =>
    expect(screen.queryByText("Checking")).not.toBeInTheDocument(),
  );
  expect(screen.getByText("Emergency savings")).toBeInTheDocument();
});
