import { QueryClient, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { act, fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto, MultiSeriesForecastDto } from "@/bindings";
import { CoverItNotice } from "./CoverItNotice";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  transactionList: vi.fn(),
  recordTransfer: vi.fn(),
  forecast: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const usd = (m: number) => ({ minor_units: m, currency: "USD" });
const SAVINGS = "0190a000-0000-7000-8000-000000000002";

function day(date: string, net: number) {
  const m = usd(net);
  return { date, closing: { p10: m, p50: m, p90: m }, events: [] };
}

function projection(checkingDays: ReturnType<typeof day>[]): MultiSeriesForecastDto {
  return {
    currency: "USD",
    start_date: "2026-07-01",
    horizon_days: 90,
    groups: [],
    accounts: [
      {
        account_id: "chk",
        name: "Checking",
        subtype: "checking",
        tier: "spendable",
        days: checkingDays,
      },
    ],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.forecast.mockResolvedValue("forecast");
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
        balance: usd(20_000),
      },
      {
        id: SAVINGS,
        name: "Savings",
        cashflow_role: "liquid_cash",
        subtype: "savings",
        active: true,
        balance: usd(500_000),
      },
    ] as AccountViewDto[]),
  );
});

test("states the shortfall and covers it with a transfer from a liquid source", async () => {
  renderWithClient(
    <CoverItNotice
      projection={projection([day("2026-07-05", 20_000), day("2026-07-20", -30_000)])}
    />,
  );

  expect(await screen.findByText(/is projected to be below/i)).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: /cover it/i }));
  fireEvent.click(await screen.findByRole("button", { name: /confirm transfer/i }));

  await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(1));
  const input = mocks.recordTransfer.mock.calls[0]?.[0];
  expect(input.dest_account_id).toBe("chk"); // the shorting account
  expect(input.source_account_id).toBe(SAVINGS); // a liquid source, not the shorting account
  expect(input.amount.minor_units).toBe(30_000); // the proposed shortfall amount
});

test("excludes a source account in a different currency", async () => {
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: "chk",
        name: "Checking",
        cashflow_role: "liquid_cash",
        subtype: "checking",
        active: true,
        balance: usd(20_000),
      },
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
  renderWithClient(
    <CoverItNotice projection={projection([day("2026-07-20", -30_000)])} />,
  );
  fireEvent.click(await screen.findByRole("button", { name: /cover it/i }));
  // The EUR account can't cover a USD shortfall, so no source is offered.
  expect(
    await screen.findByText(/no other account has funds/i),
  ).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /confirm transfer/i })).toBeNull();
});

test("offers an investment account as a source (j0cg.2 withdrawal)", async () => {
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: "chk",
        name: "Checking",
        cashflow_role: "liquid_cash",
        subtype: "checking",
        active: true,
        balance: usd(5_000),
      },
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
  renderWithClient(
    <CoverItNotice projection={projection([day("2026-07-20", -30_000)])} />,
  );

  fireEvent.click(await screen.findByRole("button", { name: /cover it/i }));
  fireEvent.click(await screen.findByRole("button", { name: /confirm transfer/i }));

  await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(1));
  expect(mocks.recordTransfer.mock.calls[0]?.[0].source_account_id).toBe("brk");
});

test("renders nothing when no account dips below $0", async () => {
  const { container } = renderWithClient(
    <CoverItNotice projection={projection([day("2026-07-20", 50_000)])} />,
  );
  await waitFor(() => expect(mocks.accountList).toHaveBeenCalled());
  expect(container).toBeEmptyDOMElement();
});

const SHORT = [day("2026-07-05", 20_000), day("2026-07-20", -30_000)];

async function openTool() {
  fireEvent.click(await screen.findByRole("button", { name: /cover it/i }));
}

describe("what the tool says while its accounts are not ready (5ie.17)", () => {
  test("a read still loading is not 'no funding account'", async () => {
    mocks.accountList.mockReturnValue(new Promise(() => {}));
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await openTool();
    expect(await screen.findByText("Loading your accounts…")).toBeInTheDocument();
    expect(screen.queryByText(/no other account has funds/i)).toBeNull();
    expect(screen.queryByRole("button", { name: /confirm transfer/i })).toBeNull();
  });

  test("a failed read says so and is not 'no funding account'", async () => {
    mocks.accountList.mockRejectedValue(new Error("boom"));
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await openTool();
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load accounts.");
    expect(screen.queryByText(/no other account has funds/i)).toBeNull();
  });
});

describe("naming accounts (5ie.17)", () => {
  test("two accounts with one name are told apart, in the headline, the sentence and the picker", async () => {
    mocks.accountList.mockResolvedValue(
      ok([
        {
          id: "chk",
          name: "Savings",
          cashflow_role: "liquid_cash",
          subtype: "checking",
          active: true,
          balance: usd(20_000),
        },
        {
          id: SAVINGS,
          name: "Savings",
          cashflow_role: "liquid_cash",
          subtype: "savings",
          active: true,
          balance: usd(500_000),
        },
      ] as AccountViewDto[]),
    );
    const sameName = projection(SHORT);
    sameName.accounts[0]!.name = "Savings";
    renderWithClient(<CoverItNotice projection={sameName} />);
    expect(await screen.findByText("Savings · Checking")).toBeInTheDocument();
    await openTool();
    expect(
      await screen.findByRole("option", { name: "Savings · Savings ($5,000.00)" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: /^Savings \(/ })).toBeNull();
    expect(screen.getAllByText("Savings · Savings").length).toBeGreaterThan(0);
  });
});

describe("one idempotency key per draft (5ie.17)", () => {
  async function confirm() {
    fireEvent.click(await screen.findByRole("button", { name: /confirm transfer/i }));
  }
  const keyOf = (call: number) => mocks.recordTransfer.mock.calls[call]?.[0].idempotency_key;

  test("a retry after a thrown call sends the same key; a changed amount sends a new one", async () => {
    mocks.recordTransfer.mockRejectedValueOnce(new Error("connection lost"));
    mocks.recordTransfer.mockResolvedValue({ status: "error", error: { Validation: "nope" } });
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await openTool();
    await confirm();
    await screen.findByText(/couldn’t confirm whether the transfer was recorded/i);
    await waitFor(() => expect(screen.getByRole("button", { name: /confirm transfer/i })).toBeEnabled());
    await confirm();
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(2));
    expect(keyOf(1)).toBe(keyOf(0));

    fireEvent.change(screen.getByLabelText("Amount to transfer"), { target: { value: "250" } });
    await confirm();
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(3));
    expect(keyOf(2)).not.toBe(keyOf(0));
    expect(mocks.recordTransfer.mock.calls[2]?.[0].amount.minor_units).toBe(25_000);
  });

  test("a changed source account sends a new key", async () => {
    mocks.accountList.mockResolvedValue(
      ok([
        { id: "chk", name: "Checking", cashflow_role: "liquid_cash", subtype: "checking", active: true, balance: usd(20_000) },
        { id: SAVINGS, name: "Savings", cashflow_role: "liquid_cash", subtype: "savings", active: true, balance: usd(500_000) },
        { id: "mm", name: "Reserve", cashflow_role: "liquid_cash", subtype: "money_market", active: true, balance: usd(500_000) },
      ] as AccountViewDto[]),
    );
    mocks.recordTransfer.mockResolvedValue({ status: "error", error: { Validation: "nope" } });
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await openTool();
    await confirm();
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(1));
    fireEvent.change(screen.getByLabelText("From account"), { target: { value: "mm" } });
    await confirm();
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(2));
    expect(mocks.recordTransfer.mock.calls[1]?.[0].source_account_id).toBe("mm");
    expect(keyOf(1)).not.toBe(keyOf(0));
  });

  test("a recorded transfer ends the draft: the next one, even for the same amount, is a new key", async () => {
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await openTool();
    await confirm();
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(1));
    await openTool();
    await confirm();
    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(2));
    expect(keyOf(1)).not.toBe(keyOf(0));
  });
});

describe("while the transfer is pending (5ie.17)", () => {
  test("Cancel is disabled and Escape does not close the tool; both work once it is done", async () => {
    let finish: (value: unknown) => void = () => {};
    mocks.recordTransfer.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await openTool();
    fireEvent.click(await screen.findByRole("button", { name: /confirm transfer/i }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled());
    fireEvent.keyDown(screen.getByLabelText("Amount to transfer"), { key: "Escape" });
    expect(screen.getByLabelText("Amount to transfer")).toBeInTheDocument();
    await act(async () => finish(ok({ op_seq: 1, replayed: false })));
    await waitFor(() => expect(screen.queryByLabelText("Amount to transfer")).toBeNull());
  });

  test("Escape closes the tool when nothing is pending", async () => {
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await openTool();
    fireEvent.keyDown(await screen.findByLabelText("Amount to transfer"), { key: "Escape" });
    expect(screen.queryByLabelText("Amount to transfer")).toBeNull();
  });
});

describe("what happens when the transfer does not come back clean (5ie.17)", () => {
  test("a typed refusal shows the app's reason and does not re-read", async () => {
    mocks.recordTransfer.mockResolvedValue({
      status: "error",
      error: { Validation: "That account does not have enough to cover this." },
    });
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await openTool();
    const reads = mocks.accountList.mock.calls.length;
    fireEvent.click(await screen.findByRole("button", { name: /confirm transfer/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "That account does not have enough to cover this.",
    );
    expect(screen.queryByText(/couldn’t confirm/i)).toBeNull();
    expect(mocks.accountList.mock.calls.length).toBe(reads);
    expect(screen.getByRole("button", { name: /confirm transfer/i })).toBeEnabled();
  });

  test("a thrown call says the outcome is unknown, re-reads accounts, and holds Confirm until that is done", async () => {
    mocks.recordTransfer.mockRejectedValueOnce(new Error("connection lost"));
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await openTool();
    await screen.findByRole("button", { name: /confirm transfer/i });
    const reads = mocks.accountList.mock.calls.length;
    const stored = await mocks.accountList();
    let release: (value: unknown) => void = () => {};
    mocks.accountList.mockReturnValue(new Promise((resolve) => (release = resolve)));
    fireEvent.click(screen.getByRole("button", { name: /confirm transfer/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn’t confirm whether the transfer was recorded. Check your accounts and the forecast before trying again.",
    );
    await waitFor(() => expect(mocks.accountList.mock.calls.length).toBeGreaterThan(reads));
    expect(screen.getByRole("button", { name: /confirm transfer/i })).toBeDisabled();
    await act(async () => release(stored));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /confirm transfer/i })).toBeEnabled(),
    );
  });
});

/// A forecast read beside the notice, the way the Future Cash view holds one: the notice re-reads
/// ["forecast"] after an unknown transfer outcome.
function WithForecast({ projection: p }: { projection: MultiSeriesForecastDto }) {
  useQuery({
    queryKey: ["forecast", "multi"],
    queryFn: async () => {
      const value = await mocks.forecast();
      if (value instanceof Error) throw value;
      return value;
    },
  });
  return <CoverItNotice projection={p} />;
}

const newClient = () =>
  new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });

describe("Cancel and Escape in every opened state (5ie.17 repair)", () => {
  async function expectEscapeAndCancelClose(ready: () => Promise<unknown>) {
    for (const how of ["Escape", "Cancel"] as const) {
      await openTool();
      await ready();
      expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
      if (how === "Escape") {
        // The panel holds the focus, so the key reaches it from where the person is.
        fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
      } else {
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
      }
      expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
      // Closing returns focus to the button that opened the tool.
      const cover = screen.getByRole("button", { name: /cover it/i });
      await waitFor(() => expect(cover).toHaveFocus());
    }
  }

  test("loading", async () => {
    mocks.accountList.mockReturnValue(new Promise(() => {}));
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await expectEscapeAndCancelClose(() => screen.findByText("Loading your accounts…"));
  });

  test("a failed read", async () => {
    mocks.accountList.mockRejectedValue(new Error("boom"));
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await expectEscapeAndCancelClose(() => screen.findByText("Could not load accounts."));
  });

  test("no funding account", async () => {
    mocks.accountList.mockResolvedValue(
      ok([
        { id: "chk", name: "Checking", cashflow_role: "liquid_cash", subtype: "checking", active: true, balance: usd(20_000) },
      ] as AccountViewDto[]),
    );
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await expectEscapeAndCancelClose(() => screen.findByText(/no other account has funds/i));
  });

  test("ready", async () => {
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await expectEscapeAndCancelClose(() => screen.findByLabelText("Amount to transfer"));
  });
});

describe("focus is kept where the person put it (5ie.17 repair)", () => {
  function Beside() {
    const [renders, setRenders] = useState(0);
    return (
      <>
        <button onClick={() => setRenders((n) => n + 1)}>Outside {renders}</button>
        <CoverItNotice projection={projection(SHORT)} />
      </>
    );
  }

  test("a re-render does not pull focus from a control outside the open panel", async () => {
    renderWithClient(<Beside />);
    await openTool();
    await screen.findByRole("button", { name: /confirm transfer/i });
    const outside = screen.getByRole("button", { name: /^Outside/ });
    outside.focus();
    fireEvent.click(outside);
    expect(await screen.findByRole("button", { name: "Outside 1" })).toHaveFocus();
  });

  test("a focused control inside the panel going away returns focus to the panel", async () => {
    mocks.accountList.mockRejectedValue(new Error("boom"));
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await openTool();
    const retry = await screen.findByRole("button", { name: "Retry" });
    retry.focus();
    mocks.accountList.mockResolvedValue(
      ok([
        { id: "chk", name: "Checking", cashflow_role: "liquid_cash", subtype: "checking", active: true, balance: usd(20_000) },
        { id: SAVINGS, name: "Savings", cashflow_role: "liquid_cash", subtype: "savings", active: true, balance: usd(500_000) },
      ] as AccountViewDto[]),
    );
    fireEvent.click(retry);
    const confirm = await screen.findByRole("button", { name: /confirm transfer/i });
    await waitFor(() => expect(document.activeElement).not.toBe(document.body));
    const held = document.activeElement as HTMLElement;
    expect(held).toHaveAttribute("tabindex", "-1");
    expect(held).toContainElement(confirm);
  });
});

describe("a failed accounts read has a Retry (5ie.17 repair)", () => {
  test("Retry is disabled as Retrying… while it runs, keeps the alert if it fails again, and opens the tool when it works", async () => {
    mocks.accountList.mockRejectedValue(new Error("boom"));
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />);
    await openTool();
    await screen.findByText("Could not load accounts.");

    let release: (value: unknown) => void = () => {};
    mocks.accountList.mockReturnValueOnce(new Promise((_, reject) => (release = reject)));
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    const pending = await screen.findByRole("button", { name: "Retrying…" });
    expect(pending).toBeDisabled();
    await act(async () => release(new Error("still down")));
    // A second failure keeps the alert, and Retry is back.
    expect(await screen.findByRole("button", { name: "Retry" })).toBeEnabled();
    expect(screen.getByRole("alert")).toHaveTextContent("Could not load accounts.");
    expect(screen.queryByRole("button", { name: /confirm transfer/i })).toBeNull();

    mocks.accountList.mockResolvedValue(
      ok([
        { id: "chk", name: "Checking", cashflow_role: "liquid_cash", subtype: "checking", active: true, balance: usd(20_000) },
        { id: SAVINGS, name: "Savings", cashflow_role: "liquid_cash", subtype: "savings", active: true, balance: usd(500_000) },
      ] as AccountViewDto[]),
    );
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("button", { name: /confirm transfer/i })).toBeInTheDocument();
  });

  test("cached accounts with a failed refetch show the failed read, not Confirm", async () => {
    const client = newClient();
    renderWithClient(<CoverItNotice projection={projection(SHORT)} />, client);
    await openTool();
    await screen.findByRole("button", { name: /confirm transfer/i });

    mocks.accountList.mockRejectedValue(new Error("boom"));
    await act(async () => {
      await client.refetchQueries({ queryKey: ["accounts"] });
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load accounts.");
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: /confirm transfer/i })).toBeNull();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
  });
});

describe("after an unknown transfer outcome Confirm returns only when both re-reads worked (5ie.17 repair)", () => {
  const UNKNOWN =
    "Couldn’t confirm whether the transfer was recorded. Check your accounts and the forecast before trying again.";

  async function throwTransfer() {
    mocks.recordTransfer.mockRejectedValueOnce(new Error("connection lost"));
    renderWithClient(<WithForecast projection={projection(SHORT)} />, newClient());
    await openTool();
    await screen.findByRole("button", { name: /confirm transfer/i });
    await waitFor(() => expect(mocks.forecast).toHaveBeenCalledTimes(1));
  }

  test("the forecast is re-read too, and when both worked Confirm is back with the warning", async () => {
    await throwTransfer();
    fireEvent.click(screen.getByRole("button", { name: /confirm transfer/i }));
    await waitFor(() => expect(mocks.forecast).toHaveBeenCalledTimes(2));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /confirm transfer/i })).toBeEnabled(),
    );
    expect(screen.getByRole("alert")).toHaveTextContent(UNKNOWN);
  });

  test("a failed forecast re-read shows that failure with Retry and no Confirm", async () => {
    await throwTransfer();
    mocks.forecast.mockResolvedValue(new Error("Could not load your forecast."));
    fireEvent.click(screen.getByRole("button", { name: /confirm transfer/i }));
    expect(await screen.findByText("Could not load your forecast.")).toBeInTheDocument();
    expect(screen.getByText(UNKNOWN)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /confirm transfer/i })).toBeNull();
    expect(screen.queryByLabelText("Amount to transfer")).toBeNull();

    // Retry re-reads the forecast; once it works Confirm returns.
    mocks.forecast.mockResolvedValue("forecast");
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("button", { name: /confirm transfer/i })).toBeEnabled();
  });

  test("a failed accounts re-read shows that failure with Retry and no Confirm, and keeps the warning", async () => {
    await throwTransfer();
    mocks.accountList.mockRejectedValue(new Error("boom"));
    fireEvent.click(screen.getByRole("button", { name: /confirm transfer/i }));
    expect(await screen.findByText("Could not load accounts.")).toBeInTheDocument();
    expect(screen.getByText(UNKNOWN)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /confirm transfer/i })).toBeNull();
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
  });
});

describe("the headline and the tool say what the numbers are (5ie.17 AC9)", () => {
  // Sep 2 is the first day below zero (-$10.00); Sep 3 is the lowest (-$200.00).
  const SEP = projection([
    day("2026-09-01", 5_000),
    day("2026-09-02", -1_000),
    day("2026-09-03", -20_000),
  ]);
  SEP.start_date = "2026-09-01";

  test("the header pairs the first date below zero with the near-term low, not the worst with the first", async () => {
    renderWithClient(<CoverItNotice projection={SEP} />);
    expect(
      await screen.findByText(
        (_, el) =>
          el?.tagName === "SPAN" &&
          el.textContent ===
            "Checking is projected to be below $0.00 on Sep 2, 2026. Its near-term projected low is about -$200.00.",
      ),
    ).toBeInTheDocument();
  });

  test("the tool promises the near-term low reaches zero, with the helper amount, not 'above $0'", async () => {
    renderWithClient(<CoverItNotice projection={SEP} />);
    await openTool();
    expect(
      await screen.findByText(
        (_, el) =>
          el?.tagName === "P" &&
          el.textContent ===
            "A transfer of about $200.00 from Savings would bring Checking’s near-term projected low to $0.00.",
      ),
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/above \$0/);
    // Editing the amount does not relabel the recommendation.
    fireEvent.change(screen.getByLabelText("Amount to transfer"), { target: { value: "5" } });
    expect(
      screen.getByText((_, el) => el?.tagName === "P" && (el.textContent ?? "").startsWith("A transfer of about $200.00 from Savings")),
    ).toBeInTheDocument();
  });

  test("an account already negative on the first day reads the same way", async () => {
    const already = projection([day("2026-07-01", -5_000), day("2026-07-02", -8_000)]);
    renderWithClient(<CoverItNotice projection={already} />);
    expect(
      await screen.findByText(
        (_, el) =>
          el?.tagName === "SPAN" &&
          el.textContent ===
            "Checking is projected to be below $0.00 on Jul 1, 2026. Its near-term projected low is about -$80.00.",
      ),
    ).toBeInTheDocument();
  });

  test("a euro projection formats zero and the low in euros", async () => {
    mocks.accountList.mockResolvedValue(
      ok([
        { id: "chk", name: "Checking", cashflow_role: "liquid_cash", subtype: "checking", active: true, balance: { minor_units: 20_000, currency: "EUR" } },
        { id: SAVINGS, name: "Savings", cashflow_role: "liquid_cash", subtype: "savings", active: true, balance: { minor_units: 500_000, currency: "EUR" } },
      ] as AccountViewDto[]),
    );
    const eur = projection([day("2026-07-20", -30_000)]);
    eur.currency = "EUR";
    for (const d of eur.accounts[0]!.days) {
      for (const k of ["p10", "p50", "p90"] as const) d.closing[k] = { minor_units: -30_000, currency: "EUR" };
    }
    renderWithClient(<CoverItNotice projection={eur} />);
    expect(
      await screen.findByText(
        (_, el) =>
          el?.tagName === "SPAN" &&
          (el.textContent ?? "").includes("below €0.00") &&
          (el.textContent ?? "").includes("about -€300.00"),
      ),
    ).toBeInTheDocument();
    await openTool();
    expect(
      await screen.findByText(
        (_, el) =>
          el?.tagName === "P" &&
          (el.textContent ?? "").endsWith("would bring Checking’s near-term projected low to €0.00."),
      ),
    ).toBeInTheDocument();
  });
});
