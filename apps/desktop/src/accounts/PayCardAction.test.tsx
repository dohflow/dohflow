import { QueryClient } from "@tanstack/react-query";
import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { useState } from "react";

import { queryKeys } from "@/lib/query";
import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto, CardStatementForecastDto } from "@/bindings";
import { PayCardAction } from "./PayCardAction";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  transactionList: vi.fn(),
  recordTransfer: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const usd = (m: number) => ({ minor_units: m, currency: "USD" });

const card: CardStatementForecastDto = {
  account_id: "card1",
  account_name: "Visa",
  currency: "USD",
  credit_limit_minor: 1_000_000,
  repayment_philosophy: "pay_statement_balance",
    estimate_mape_bps: 0,
  cycles: [
    {
      close_date: "2026-07-25",
      due_date: "2026-08-15",
      carried_opening_balance_minor: 120_000,
      known_charges_minor: 0,
      projected_variable_minor: 0,
      accrued_interest_minor: 0,
      statement_balance_minor: 80_000,
      minimum_due_minor: 2_500,
      full_pay_minor: 120_000,
      forecast_payment_minor: 80_000,
      statement_is_actual: false,
      is_closed: true,
    },
  ],
  stored_statements: [],
  estimate_basis: "none",
  estimate_sample_cycles: 0,
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.transactionList.mockResolvedValue(ok([]));
  mocks.recordTransfer.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: "chk",
        name: "Checking",
        cashflow_role: "liquid_cash",
        subtype: "checking",
        active: true,
        balance: usd(500_000),
      },
    ] as AccountViewDto[]),
  );
});

test("pays the statement balance by default to the card", async () => {
  renderWithClient(<PayCardAction card={card} />);

  fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
  fireEvent.click(await screen.findByRole("button", { name: /confirm payment/i }));

  await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(1));
  const input = mocks.recordTransfer.mock.calls[0]?.[0];
  expect(input.dest_account_id).toBe("card1");
  expect(input.source_account_id).toBe("chk");
  expect(input.amount.minor_units).toBe(80_000); // the statement balance
});

test("a quick-pick sets the amount (current balance)", async () => {
  renderWithClient(<PayCardAction card={card} />);

  fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
  fireEvent.click(await screen.findByRole("button", { name: /current balance/i }));
  fireEvent.click(screen.getByRole("button", { name: /confirm payment/i }));

  await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(1));
  expect(mocks.recordTransfer.mock.calls[0]?.[0].amount.minor_units).toBe(120_000);
});

test("shows no source when no liquid account holds the card's currency", async () => {
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: "eur",
        name: "Euro",
        cashflow_role: "liquid_cash",
        subtype: "checking",
        active: true,
        balance: { minor_units: 500_000, currency: "EUR" },
      },
    ] as AccountViewDto[]),
  );
  renderWithClient(<PayCardAction card={card} />);
  fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
  expect(await screen.findByText(/no liquid account holds funds/i)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /confirm payment/i })).toBeNull();
});

test("does not offer an investment account as a source (liquidate to cash first)", async () => {
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: "brk",
        name: "Brokerage",
        cashflow_role: "investment_asset",
        subtype: "brokerage",
        active: true,
        balance: usd(900_000),
      },
    ] as AccountViewDto[]),
  );
  renderWithClient(<PayCardAction card={card} />);

  fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
  // A brokerage isn't a valid direct card payer, so the tool offers no source.
  expect(await screen.findByText(/no liquid account holds funds/i)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /confirm payment/i })).toBeNull();
});

const openTool = async () => {
  fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
  return screen.findByRole("button", { name: /confirm payment/i });
};
const keyOf = (call: number) => mocks.recordTransfer.mock.calls[call]?.[0].idempotency_key;
const UNKNOWN =
  "Couldn’t confirm whether the payment was recorded. Confirm again and it won’t be recorded twice.";

describe("a payment in flight (6wk.24)", () => {
  test("Cancel is disabled and Escape does nothing until the outcome returns, then the tool closes with focus on Pay card", async () => {
    let finish: (value: unknown) => void = () => {};
    mocks.recordTransfer.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    renderWithClient(<PayCardAction card={card} />);
    const confirm = await openTool();
    fireEvent.click(confirm);

    await waitFor(() => expect(screen.getByRole("button", { name: /confirm payment/i })).toBeDisabled());
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("button", { name: /confirm payment/i })).toBeInTheDocument();

    await act(async () => finish(ok({ op_seq: 1, replayed: false })));
    await waitFor(() => expect(screen.queryByRole("button", { name: /confirm payment/i })).toBeNull());
    await waitFor(() => expect(screen.getByRole("button", { name: /pay card/i })).toHaveFocus());
  });
});

describe("a payment that does not come back clean (6wk.24)", () => {
  test("a typed refusal shows the app's reason, releases the buttons, and a retry sends the same key", async () => {
    mocks.recordTransfer.mockResolvedValue({
      status: "error",
      error: { Validation: "That account does not have enough to cover this." },
    });
    renderWithClient(<PayCardAction card={card} />);
    fireEvent.click(await openTool());
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "That account does not have enough to cover this.",
    );
    expect(screen.getByRole("button", { name: /confirm payment/i })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();

    fireEvent.click(screen.getByRole("button", { name: /confirm payment/i }));
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(2));
    expect(keyOf(1)).toBe(keyOf(0));
  });

  test("a thrown call says the outcome is unknown, re-reads accounts, releases busy, and the same payment keeps its key", async () => {
    mocks.recordTransfer.mockRejectedValueOnce(new Error("connection lost"));
    renderWithClient(<PayCardAction card={card} />);
    const confirm = await openTool();
    const reads = mocks.accountList.mock.calls.length;
    fireEvent.click(confirm);

    expect(await screen.findByRole("alert")).toHaveTextContent(UNKNOWN);
    await waitFor(() => expect(mocks.accountList.mock.calls.length).toBeGreaterThan(reads));
    await waitFor(() => expect(screen.getByRole("button", { name: /confirm payment/i })).toBeEnabled());
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
    // The warning stays on screen until the person acts.
    expect(screen.getByRole("alert")).toHaveTextContent(UNKNOWN);

    fireEvent.click(screen.getByRole("button", { name: /confirm payment/i }));
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(2));
    expect(keyOf(1)).toBe(keyOf(0));
    await waitFor(() => expect(screen.queryByRole("button", { name: /confirm payment/i })).toBeNull());
  });

  test("Confirm stays held until the re-read finishes", async () => {
    mocks.recordTransfer.mockRejectedValueOnce(new Error("connection lost"));
    renderWithClient(<PayCardAction card={card} />);
    const confirm = await openTool();
    let release: (value: unknown) => void = () => {};
    const stored = await mocks.accountList();
    mocks.accountList.mockReturnValue(new Promise((resolve) => (release = resolve)));
    fireEvent.click(confirm);
    await screen.findByRole("alert");
    expect(screen.getByRole("button", { name: /confirm payment/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    await act(async () => release(stored));
    await waitFor(() => expect(screen.getByRole("button", { name: /confirm payment/i })).toBeEnabled());
  });

  test("a changed amount is a new draft with a new key", async () => {
    mocks.recordTransfer.mockRejectedValueOnce(new Error("connection lost"));
    renderWithClient(<PayCardAction card={card} />);
    fireEvent.click(await openTool());
    await waitFor(() => expect(screen.getByRole("button", { name: /confirm payment/i })).toBeEnabled());
    fireEvent.change(screen.getByLabelText("Amount"), { target: { value: "500" } });
    fireEvent.click(screen.getByRole("button", { name: /confirm payment/i }));
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(2));
    expect(keyOf(1)).not.toBe(keyOf(0));
  });

  test("after a recorded payment the next one is a new draft, even for the same amount", async () => {
    renderWithClient(<PayCardAction card={card} />);
    fireEvent.click(await openTool());
    await waitFor(() => expect(screen.queryByRole("button", { name: /confirm payment/i })).toBeNull());
    fireEvent.click(await openTool());
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(2));
    expect(keyOf(1)).not.toBe(keyOf(0));
  });
});

describe("every opened state can be left, with focus back on Pay card (6wk.24)", () => {
  async function expectEscapeAndCancelClose(ready: () => Promise<unknown>) {
    for (const how of ["Escape", "Cancel"] as const) {
      fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
      await ready();
      if (how === "Escape") {
        fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
      } else {
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
      }
      const pay = await screen.findByRole("button", { name: /pay card/i });
      await waitFor(() => expect(pay).toHaveFocus());
    }
  }

  test("ready to pay", async () => {
    renderWithClient(<PayCardAction card={card} />);
    await expectEscapeAndCancelClose(() => screen.findByRole("button", { name: /confirm payment/i }));
  });

  test("no account to pay from", async () => {
    mocks.accountList.mockResolvedValue(ok([] as AccountViewDto[]));
    renderWithClient(<PayCardAction card={card} />);
    await expectEscapeAndCancelClose(() => screen.findByText(/no liquid account holds funds/i));
  });

  test("a re-render does not pull focus from a control the person chose outside the tool", async () => {
    function Beside() {
      const [n, setN] = useState(0);
      return (
        <>
          <button onClick={() => setN(n + 1)}>Outside {n}</button>
          <PayCardAction card={card} />
        </>
      );
    }
    renderWithClient(<Beside />);
    await openTool();
    const outside = screen.getByRole("button", { name: /^Outside/ });
    outside.focus();
    fireEvent.click(outside);
    expect(await screen.findByRole("button", { name: "Outside 1" })).toHaveFocus();
  });
});

const CHECKING = {
  id: "chk",
  name: "Checking",
  cashflow_role: "liquid_cash",
  subtype: "checking",
  active: true,
  balance: usd(500_000),
} as AccountViewDto;
const NO_SOURCE = /no liquid account holds funds/i;

describe("what the opened tool says about the accounts it pays from (6wk.24)", () => {
  test("while the accounts load it says so, not that nothing can pay, and can still be left", async () => {
    mocks.accountList.mockReturnValue(new Promise(() => {}));
    renderWithClient(<PayCardAction card={card} />);
    fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
    expect(await screen.findByRole("status")).toHaveTextContent("Loading your accounts…");
    expect(screen.queryByText(NO_SOURCE)).toBeNull();
    expect(screen.queryByRole("button", { name: /confirm payment/i })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.getByRole("button", { name: /pay card/i })).toHaveFocus());
  });

  test("a failed read shows the existing message with Retry, never 'no account', and can be left", async () => {
    mocks.accountList.mockRejectedValue(new Error("boom"));
    renderWithClient(<PayCardAction card={card} />);
    fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load accounts.");
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
    expect(screen.queryByText(NO_SOURCE)).toBeNull();
    expect(screen.queryByText("Loading your accounts…")).toBeNull();
    expect(screen.queryByRole("button", { name: /confirm payment/i })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.getByRole("button", { name: /pay card/i })).toHaveFocus());
  });

  test("Retry reads Retrying… and keeps the failure while it runs, keeps it if it fails again, and opens the tool when it works", async () => {
    mocks.accountList.mockRejectedValue(new Error("boom"));
    renderWithClient(<PayCardAction card={card} />);
    fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
    await screen.findByRole("alert");

    let fail: (reason: unknown) => void = () => {};
    mocks.accountList.mockReturnValueOnce(new Promise((_, reject) => (fail = reject)));
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    const pending = await screen.findByRole("button", { name: "Retrying…" });
    expect(pending).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent("Could not load accounts.");
    expect(screen.queryByText("Loading your accounts…")).toBeNull();
    await act(async () => fail(new Error("still down")));
    expect(await screen.findByRole("button", { name: "Retry" })).toBeEnabled();
    expect(screen.getByRole("alert")).toHaveTextContent("Could not load accounts.");

    mocks.accountList.mockResolvedValue(ok([CHECKING]));
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("button", { name: /confirm payment/i })).toBeInTheDocument();
  });

  test("accounts read earlier do not stand in for a read that just failed", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    renderWithClient(<PayCardAction card={card} />, client);
    await openTool();

    mocks.accountList.mockRejectedValue(new Error("boom"));
    await act(async () => {
      await client.refetchQueries({ queryKey: ["accounts"] });
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load accounts.");
    expect(screen.queryByRole("button", { name: /confirm payment/i })).toBeNull();
    expect(screen.queryByText(NO_SOURCE)).toBeNull();
  });

  test("no account to pay from is said only after a read that worked", async () => {
    mocks.accountList.mockResolvedValue(ok([] as AccountViewDto[]));
    renderWithClient(<PayCardAction card={card} />);
    fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
    expect(await screen.findByText(NO_SOURCE)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText("Loading your accounts…")).toBeNull();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
  });

  test("an eligible account opens the payment form", async () => {
    renderWithClient(<PayCardAction card={card} />);
    expect(await openTool()).toBeEnabled();
    expect(screen.queryByText(NO_SOURCE)).toBeNull();
    expect(screen.queryByText("Loading your accounts…")).toBeNull();
  });

  test("an unknown payment outcome stays on screen when the accounts re-read then fails", async () => {
    mocks.recordTransfer.mockRejectedValueOnce(new Error("connection lost"));
    renderWithClient(<PayCardAction card={card} />);
    const confirm = await openTool();
    mocks.accountList.mockRejectedValue(new Error("boom"));
    fireEvent.click(confirm);
    await waitFor(() => expect(screen.getAllByRole("alert")).toHaveLength(2));
    expect(screen.getByText(UNKNOWN)).toBeInTheDocument();
    expect(screen.getByText("Could not load accounts.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
  });
});

const SAVINGS = {
  id: "sav",
  name: "Savings",
  cashflow_role: "liquid_cash",
  subtype: "savings",
  active: true,
  balance: usd(300_000),
} as AccountViewDto;
const amountInput = () => screen.getByLabelText("Amount") as HTMLInputElement;
const sourceSelect = () => screen.getByLabelText("From account") as HTMLSelectElement;
const quickPick = (name: RegExp) => screen.getByRole("button", { name });

describe("the draft is frozen while a payment is being recorded (6wk.24, Studio R1)", () => {
  test("amount, source and every quick pick are locked while the call is pending, and a retry after an unknown outcome sends the same payment with the same key", async () => {
    mocks.accountList.mockResolvedValue(ok([CHECKING, SAVINGS]));
    let fail: (reason: unknown) => void = () => {};
    mocks.recordTransfer.mockReturnValueOnce(new Promise((_, reject) => (fail = reject)));
    renderWithClient(<PayCardAction card={card} />);
    const confirm = await openTool();
    fireEvent.click(confirm);
    await waitFor(() => expect(confirm).toBeDisabled());

    // The draft the call carried is on screen, and nothing can change it under that call.
    expect(amountInput()).toBeDisabled();
    expect(sourceSelect()).toBeDisabled();
    for (const name of [/statement balance/i, /minimum due/i, /current balance/i]) {
      expect(quickPick(name)).toBeDisabled();
    }
    fireEvent.click(quickPick(/minimum due/i));
    expect(amountInput().value).toBe("800");
    expect(sourceSelect().value).toBe("chk");

    await act(async () => fail(new Error("connection lost")));
    expect(await screen.findByRole("alert")).toHaveTextContent(UNKNOWN);
    await waitFor(() => expect(screen.getByRole("button", { name: /confirm payment/i })).toBeEnabled());
    expect(amountInput().value).toBe("800");

    fireEvent.click(screen.getByRole("button", { name: /confirm payment/i }));
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(2));
    expect(mocks.recordTransfer.mock.calls[1]?.[0].amount.minor_units).toBe(80_000);
    expect(mocks.recordTransfer.mock.calls[1]?.[0].source_account_id).toBe("chk");
    expect(keyOf(1)).toBe(keyOf(0));
  });

  test("the draft stays locked through the outcome re-read, then opens again", async () => {
    mocks.accountList.mockResolvedValue(ok([CHECKING, SAVINGS]));
    mocks.recordTransfer.mockRejectedValueOnce(new Error("connection lost"));
    renderWithClient(<PayCardAction card={card} />);
    const confirm = await openTool();
    let release: (value: unknown) => void = () => {};
    const stored = await mocks.accountList();
    mocks.accountList.mockReturnValue(new Promise((resolve) => (release = resolve)));
    fireEvent.click(confirm);
    expect(await screen.findByRole("alert")).toHaveTextContent(UNKNOWN);

    expect(amountInput()).toBeDisabled();
    expect(sourceSelect()).toBeDisabled();
    expect(quickPick(/minimum due/i)).toBeDisabled();
    fireEvent.click(quickPick(/minimum due/i));
    expect(amountInput().value).toBe("800");

    await act(async () => release(stored));
    await waitFor(() => expect(amountInput()).toBeEnabled());
    expect(sourceSelect()).toBeEnabled();
    expect(quickPick(/minimum due/i)).toBeEnabled();
  });

  test("a deliberate quick pick after the outcome is a new payment with a new key", async () => {
    mocks.recordTransfer.mockRejectedValueOnce(new Error("connection lost"));
    renderWithClient(<PayCardAction card={card} />);
    fireEvent.click(await openTool());
    await waitFor(() => expect(amountInput()).toBeEnabled());
    fireEvent.click(quickPick(/minimum due/i));
    expect(amountInput().value).toBe("25");
    fireEvent.click(screen.getByRole("button", { name: /confirm payment/i }));
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(2));
    expect(mocks.recordTransfer.mock.calls[1]?.[0].amount.minor_units).toBe(2_500);
    expect(keyOf(1)).not.toBe(keyOf(0));
  });

  test("a changed source after the outcome is a new payment with a new key", async () => {
    mocks.accountList.mockResolvedValue(ok([CHECKING, SAVINGS]));
    mocks.recordTransfer.mockRejectedValueOnce(new Error("connection lost"));
    renderWithClient(<PayCardAction card={card} />);
    fireEvent.click(await openTool());
    await waitFor(() => expect(sourceSelect()).toBeEnabled());
    fireEvent.change(sourceSelect(), { target: { value: "sav" } });
    fireEvent.click(screen.getByRole("button", { name: /confirm payment/i }));
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(2));
    expect(mocks.recordTransfer.mock.calls[1]?.[0].source_account_id).toBe("sav");
    expect(keyOf(1)).not.toBe(keyOf(0));
  });
});

describe("what is read again after an unknown outcome, and leaving while it is read (6wk.24 Lows)", () => {
  test("the forecast is read again as well as the accounts", async () => {
    mocks.recordTransfer.mockRejectedValueOnce(new Error("connection lost"));
    const forecastRead = vi.fn().mockResolvedValue("forecast");
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    await client.prefetchQuery({ queryKey: ["forecast", "probe"], queryFn: forecastRead });
    expect(forecastRead).toHaveBeenCalledTimes(1);
    renderWithClient(<PayCardAction card={card} />, client);
    fireEvent.click(await openTool());
    await waitFor(() => expect(forecastRead).toHaveBeenCalledTimes(2));
  });

  test("Cancel and Escape stay refused on the failure panel while the payment is still being recorded", async () => {
    let finish: (value: unknown) => void = () => {};
    mocks.recordTransfer.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    renderWithClient(<PayCardAction card={card} />, client);
    fireEvent.click(await openTool());
    mocks.accountList.mockRejectedValue(new Error("boom"));
    await act(async () => {
      await client.refetchQueries({ queryKey: queryKeys.accounts });
    });
    expect(await screen.findByText("Could not load accounts.")).toBeInTheDocument();

    const cancel = screen.getByRole("button", { name: "Cancel" });
    expect(cancel).toBeDisabled();
    fireEvent.click(cancel);
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    fireEvent.keyDown(screen.getByText("Could not load accounts."), { key: "Escape" });
    expect(screen.getByText("Could not load accounts.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /pay card/i })).toBeNull();

    await act(async () => finish(ok({ op_seq: 1, replayed: false })));
    await waitFor(() => expect(screen.getByRole("button", { name: /pay card/i })).toBeInTheDocument());
  });
});
