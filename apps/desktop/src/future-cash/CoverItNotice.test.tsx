import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto, MultiSeriesForecastDto } from "@/bindings";
import { CoverItNotice } from "./CoverItNotice";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  transactionList: vi.fn(),
  recordTransfer: vi.fn(),
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

  expect(await screen.findByText(/projected to reach about/i)).toBeInTheDocument();

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
