import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";
import type { AccountViewDto } from "@/bindings";
import { expectDraftKeyIdentity } from "@/test/idempotencyIdentity";
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

describe("a save that stops part-way (g3m.35)", () => {
  const refusal = { status: "error", error: { Validation: "That note is too long." } } as const;

  function fillCard() {
    fireEvent.change(screen.getByLabelText("Account name"), { target: { value: "Sapphire" } });
    fireEvent.change(screen.getByLabelText("Type"), { target: { value: "CreditFacility" } });
    fireEvent.change(screen.getByPlaceholderText(/anything you want to remember/i), {
      target: { value: "Travel card" },
    });
  }

  test("a retry continues the account that was created and never creates a second", async () => {
    mocks.setDebtTerms.mockResolvedValueOnce(refusal);
    mocks.setDebtTerms.mockResolvedValue(ok({ op_seq: 3, replayed: false }));
    mocks.setAccountNote.mockResolvedValue(ok({ op_seq: 2, replayed: false }));
    const onClose = vi.fn();
    renderWithClient(
      <AccountEditorDrawer account={null} defaultCurrency="USD" onClose={onClose} />,
    );
    fillCard();
    fireEvent.click(screen.getByRole("button", { name: /add account/i }));
    await screen.findByRole("button", { name: /try again/i });
    expect(onClose).not.toHaveBeenCalled();
    expect(mocks.createAccount).toHaveBeenCalledTimes(1);
    expect(mocks.setAccountNote).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: /try again/i }));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(mocks.createAccount).toHaveBeenCalledTimes(1); // still one account
    expect(mocks.setAccountNote).toHaveBeenCalledTimes(1); // the saved step is not repeated
    expect(mocks.setDebtTerms).toHaveBeenCalledTimes(2);
    expect(mocks.setDebtTerms.mock.calls[1]?.[0].account_id).toBe("new-id");
  });

  test("says which steps saved and which did not, with the reason, and promises no undo", async () => {
    mocks.setDebtTerms.mockResolvedValue(refusal);
    mocks.setAccountNote.mockResolvedValue(ok({ op_seq: 2, replayed: false }));
    renderWithClient(
      <AccountEditorDrawer account={null} defaultCurrency="USD" onClose={vi.fn()} />,
    );
    fillCard();
    fireEvent.click(screen.getByRole("button", { name: /add account/i }));
    const alert = await screen.findByText(/the account was added, but not everything saved/i);
    const block = alert.parentElement as HTMLElement;
    expect(within(block).getByText("Saved: Account, Notes.")).toBeInTheDocument();
    expect(within(block).getByText(/Didn’t save: Debt\. That note is too long\./)).toBeInTheDocument();
    expect(screen.queryByText(/undo/i)).toBeNull();
    // The drawer now edits the account it created.
    expect(screen.getByRole("dialog", { name: /edit sapphire/i })).toBeInTheDocument();
    expect(screen.getByLabelText("Type")).toBeDisabled();
  });

  test("a step that threw is listed as unconfirmed, never as not saved", async () => {
    mocks.setAccountNote.mockRejectedValueOnce(new Error("connection lost"));
    mocks.setAccountNote.mockResolvedValue(ok({ op_seq: 2, replayed: false }));
    const onClose = vi.fn();
    renderWithClient(
      <AccountEditorDrawer account={null} defaultCurrency="USD" onClose={onClose} />,
    );
    fillCard();
    fireEvent.click(screen.getByRole("button", { name: /add account/i }));
    const heading = await screen.findByText(/the account was added, but not everything could be confirmed/i);
    const block = heading.parentElement as HTMLElement;
    expect(within(block).getByText("Saved: Account.")).toBeInTheDocument();
    expect(within(block).getByText("Couldn’t confirm: Notes. It may or may not have saved.")).toBeInTheDocument();
    expect(within(block).queryByText(/Didn’t save/)).toBeNull();
    expect(within(block).getByText("Not tried yet: Debt.")).toBeInTheDocument();

    // Trying again repeats the unconfirmed step and the rest, and never creates a second account.
    fireEvent.click(screen.getByRole("button", { name: /try again/i }));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(mocks.createAccount).toHaveBeenCalledTimes(1);
    expect(mocks.setAccountNote).toHaveBeenCalledTimes(2);
  });

  test("a refused step and a thrown step never share a bucket", async () => {
    mocks.setDebtTerms.mockResolvedValue(refusal);
    mocks.setAccountNote.mockResolvedValue(ok({ op_seq: 2, replayed: false }));
    renderWithClient(
      <AccountEditorDrawer account={null} defaultCurrency="USD" onClose={vi.fn()} />,
    );
    fillCard();
    fireEvent.click(screen.getByRole("button", { name: /add account/i }));
    const heading = await screen.findByText(/the account was added, but not everything saved/i);
    expect(within(heading.parentElement as HTMLElement).queryByText(/Couldn’t confirm/)).toBeNull();
  });

  test("a create call that threw is retried with the same key", async () => {
    mocks.createAccount.mockRejectedValueOnce(new Error("connection lost"));
    renderWithClient(
      <AccountEditorDrawer account={null} defaultCurrency="USD" onClose={vi.fn()} />,
    );
    fireEvent.change(screen.getByLabelText("Account name"), { target: { value: "Roth IRA" } });
    fireEvent.click(screen.getByRole("button", { name: /add account/i }));
    expect(await screen.findByText(/couldn’t confirm whether this saved/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /add account/i }));
    await waitFor(() => expect(mocks.createAccount).toHaveBeenCalledTimes(2));
    const keys = mocks.createAccount.mock.calls.map((c) => c[0].idempotency_key);
    expect(keys[1]).toBe(keys[0]);
  });

  test("Escape, the backdrop and Close do nothing while a save is in flight", async () => {
    let finish: (value: unknown) => void = () => {};
    mocks.createAccount.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    const onClose = vi.fn();
    renderWithClient(
      <AccountEditorDrawer account={null} defaultCurrency="USD" onClose={onClose} />,
    );
    fireEvent.change(screen.getByLabelText("Account name"), { target: { value: "Roth IRA" } });
    fireEvent.click(screen.getByRole("button", { name: /add account/i }));
    await waitFor(() => expect(screen.getByRole("button", { name: /cancel/i })).toBeDisabled());

    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.click(screen.getByRole("dialog").firstElementChild as HTMLElement);
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).not.toHaveBeenCalled();

    await act(async () => finish(ok({ account_id: "new-id", mutation: { op_seq: 1, replayed: false } })));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });

  test("editing an account: a retry repeats only the steps that did not save", async () => {
    mocks.updateAccount.mockResolvedValue(ok({ op_seq: 4, replayed: false }));
    mocks.setAccountNote.mockResolvedValueOnce(refusal);
    mocks.setAccountNote.mockResolvedValue(ok({ op_seq: 5, replayed: false }));
    const onClose = vi.fn();
    renderWithClient(
      <AccountEditorDrawer
        account={{
          id: "chk-1",
          name: "Checking",
          cashflow_role: "liquid_cash",
          subtype: null,
          active: true,
          balance: { minor_units: 0, currency: "USD" },
          notes: null,
          linked_account_id: null,
          linked_account_name: null,
        }}
        defaultCurrency="USD"
        onClose={onClose}
      />,
    );
    fireEvent.change(screen.getByLabelText("Account name"), { target: { value: "Everyday" } });
    fireEvent.change(screen.getByPlaceholderText(/anything you want to remember/i), {
      target: { value: "Main account" },
    });
    fireEvent.click(screen.getByRole("button", { name: /save account/i }));
    expect(await screen.findByText(/some changes saved and some didn’t/i)).toBeInTheDocument();
    expect(screen.getByText(/Saved: Name\./)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /try again/i }));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(mocks.updateAccount).toHaveBeenCalledTimes(1); // the rename is not sent twice
    expect(mocks.setAccountNote).toHaveBeenCalledTimes(2);
  });
});

describe("the balance step keeps one idempotency key per draft (g3m.40)", () => {
  const CHECKING: AccountViewDto = {
    id: "chk-1",
    name: "Checking",
    cashflow_role: "liquid_cash",
    subtype: null,
    active: true,
    balance: { minor_units: 0, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
  };
  const SAVED = ok({ balance: { minor_units: 12_300, currency: "USD" }, unexplained: null });
  const UNKNOWN =
    "Couldn’t confirm whether this balance was saved. Save again and it won’t be added twice.";
  const SAVED_UNCONFIRMED = "Saved, but the new balance couldn’t be confirmed.";
  const refusal = { status: "error", error: { Validation: "That amount is too large." } } as const;

  const keys = () => mocks.assertBalance.mock.calls.map((c) => c[0].idempotency_key as string);
  const view = (onClose = vi.fn()) =>
    renderWithClient(
      <AccountEditorDrawer account={CHECKING} defaultCurrency="USD" onClose={onClose} />,
    );
  const setBalance = (value: string) =>
    fireEvent.change(screen.getByLabelText("Balance"), { target: { value } });
  const saveButton = () => screen.getByRole("button", { name: /save account|try again/i });
  const save = () => fireEvent.click(saveButton());
  /// Waits for the form to be released after the previous send (it is busy until the call settles).
  const saveWhenReady = async () => {
    await waitFor(() => expect(saveButton()).toBeEnabled());
    save();
  };

  beforeEach(() => {
    // A lost connection: the call rejects, so nobody knows whether it was written.
    mocks.assertBalance.mockRejectedValue(new Error("connection lost"));
    vi.useFakeTimers({ toFake: ["Date"], now: new Date("2026-11-04T12:00:00") });
  });
  afterEach(() => vi.useRealTimers());

  test("an unknown outcome then a retry reuses the key; any changed field, a refusal and a success each start a new draft", async () => {
    let rendered = view();
    await expectDraftKeyIdentity({
      name: "the account editor's balance save",
      open: () => void setBalance("123"),
      send: saveWhenReady,
      keys,
      fields: [
        { name: "amount", change: () => void setBalance("456") },
        { name: "date", change: () => void vi.setSystemTime(new Date("2026-11-05T12:00:00")) },
        // The currency is the account's own and has no control in this drawer, so no UI change can
        // reach it; it is in the identity string all the same (the test below pins that it is sent).
      ],
      refusal: { arrange: () => mocks.assertBalance.mockResolvedValueOnce(refusal) },
      success: {
        arrange: () => mocks.assertBalance.mockResolvedValueOnce(SAVED),
        // A confirmed save closes the drawer, so the same figures come back in a new one.
        reopen: () => {
          rendered.unmount();
          rendered = view();
          void setBalance("123");
        },
      },
    });
  });

  test("the draft carries the account, the figure, the date and the currency it was entered with", async () => {
    view();
    setBalance("123");
    save();
    await waitFor(() => expect(mocks.assertBalance).toHaveBeenCalledTimes(1));
    expect(mocks.assertBalance.mock.calls[0]?.[0]).toMatchObject({
      account_id: "chk-1",
      amount: { minor_units: 12_300, currency: "USD" },
      as_of_date: "2026-11-04",
      idempotency_key: expect.stringMatching(/.+/),
    });
  });

  test("a call that threw says it could not confirm, never 'not saved', and releases the form", async () => {
    const onClose = vi.fn();
    view(onClose);
    setBalance("123");
    save();
    const alert = await screen.findByText(UNKNOWN);
    expect(alert).toHaveAttribute("role", "alert");
    expect(screen.queryByText(/couldn’t confirm whether this saved/i)).toBeNull();
    expect(screen.queryByText(/didn’t save|not saved/i)).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
    // Busy is released: Save is live again and Cancel works.
    expect(screen.getByRole("button", { name: /save account/i })).toBeEnabled();
    expect(screen.getByRole("button", { name: /cancel/i })).toBeEnabled();
  });

  test("a refused balance shows the host's reason, not the unknown-outcome sentence", async () => {
    mocks.assertBalance.mockResolvedValueOnce(refusal);
    view();
    setBalance("123");
    save();
    expect(await screen.findByText("That amount is too large.")).toBeInTheDocument();
    expect(screen.queryByText(UNKNOWN)).toBeNull();
  });

  test("a balance saved but not read back is saved, says so, and keeps the drawer open", async () => {
    mocks.assertBalance.mockResolvedValue(ok({ balance: null, unexplained: null }));
    const onClose = vi.fn();
    view(onClose);
    setBalance("123");
    save();
    const note = await screen.findByText(SAVED_UNCONFIRMED);
    expect(note).toBeInTheDocument();
    expect(screen.getByText("Saved: Balance.")).toBeInTheDocument();
    expect(screen.queryByText(/didn’t save|not saved|may or may not/i)).toBeNull();
    expect(screen.queryByText(UNKNOWN)).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
    // It is stored: nothing is asserted a second time, and the person can finish.
    save();
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(mocks.assertBalance).toHaveBeenCalledTimes(1);
  });

  test("with a rename saved first and notes refused after, the balance outcome lands in its own group", async () => {
    mocks.updateAccount.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
    mocks.assertBalance.mockResolvedValue(ok({ balance: null, unexplained: null }));
    mocks.setAccountNote.mockResolvedValue({ status: "error", error: { Validation: "That note is too long." } });
    view();
    fireEvent.change(screen.getByLabelText("Account name"), { target: { value: "Everyday" } });
    setBalance("123");
    fireEvent.change(screen.getByPlaceholderText(/anything you want to remember/i), {
      target: { value: "Main account" },
    });
    save();
    const heading = await screen.findByText(/some changes saved and some didn’t/i);
    const block = heading.parentElement as HTMLElement;
    expect(within(block).getByText("Saved: Name, Balance.")).toBeInTheDocument();
    expect(within(block).getByText(/Didn’t save: Notes\. That note is too long\./)).toBeInTheDocument();
    expect(within(block).getByText(SAVED_UNCONFIRMED)).toBeInTheDocument();
  });

  test("a balance that threw after a rename saved is listed as unconfirmed, and a retry repeats only it, with its key", async () => {
    mocks.updateAccount.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
    const onClose = vi.fn();
    view(onClose);
    fireEvent.change(screen.getByLabelText("Account name"), { target: { value: "Everyday" } });
    setBalance("123");
    save();
    const heading = await screen.findByText(/some changes saved and some couldn’t be confirmed/i);
    const block = heading.parentElement as HTMLElement;
    expect(within(block).getByText("Saved: Name.")).toBeInTheDocument();
    expect(within(block).getByText("Couldn’t confirm: Balance. It may or may not have saved.")).toBeInTheDocument();
    expect(within(block).queryByText(/Didn’t save/)).toBeNull();

    mocks.assertBalance.mockResolvedValue(SAVED);
    save();
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(mocks.updateAccount).toHaveBeenCalledTimes(1);
    expect(mocks.assertBalance).toHaveBeenCalledTimes(2);
    expect(keys()[1]).toBe(keys()[0]);
  });
});
