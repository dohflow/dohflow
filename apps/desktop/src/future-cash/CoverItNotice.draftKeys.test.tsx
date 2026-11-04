/**
 * personal-cfo-56w.8: the per-draft idempotency key of Cover it (personal-cfo-5ie.17), pinned for
 * EVERY field of the draft's identity with the shared helper. Its own file keeps it clear of the
 * read-back suites in CoverItNotice.test.tsx.
 */

import { useState } from "react";
import { act, fireEvent, screen } from "@testing-library/react";

import { expectDraftKeyIdentity } from "@/test/idempotencyIdentity";
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
const BROKERAGE = "0190a000-0000-7000-8000-000000000003";

const account = (id: string, name: string, balance: number, role = "liquid_cash") =>
  ({ id, name, cashflow_role: role, subtype: null, active: true, balance: usd(balance) }) as AccountViewDto;

const series = (id: string, name: string, lowest: number) => ({
  account_id: id,
  name,
  subtype: "checking",
  tier: "spendable" as const,
  days: [{ date: "2026-07-20", closing: { p10: usd(lowest), p50: usd(lowest), p90: usd(lowest) }, events: [] }],
});

/// `chk` is short by 300; with `alsoShort` another account is short by more, so it becomes the one covered.
const projection = (alsoShort = false): MultiSeriesForecastDto => ({
  currency: "USD",
  start_date: "2026-07-01",
  horizon_days: 90,
  groups: [],
  accounts: [series("chk", "Checking", -30_000), ...(alsoShort ? [series("chk2", "Joint", -40_000)] : [])],
});

beforeEach(() => {
  vi.clearAllMocks();
  mocks.transactionList.mockResolvedValue(ok([]));
  mocks.accountList.mockResolvedValue(
    ok([
      account("chk", "Checking", 20_000),
      account("chk2", "Joint", 10_000),
      account(SAVINGS, "Savings", 500_000),
      account(BROKERAGE, "Brokerage", 900_000, "investment_asset"),
    ]),
  );
  // A lost connection: the call throws, the panel stays open and the key stays with the draft.
  mocks.recordTransfer.mockRejectedValue(new Error("connection lost"));
});

/// The notice under a forecast the test can change from outside, as the Future Cash view's own
/// reads change under it.
let setAlsoShort: (value: boolean) => void = () => {};
function Harness() {
  const [alsoShort, set] = useState(false);
  setAlsoShort = set;
  return <CoverItNotice projection={projection(alsoShort)} />;
}

const amount = (value: string) => {
  fireEvent.change(screen.getByLabelText("Amount to transfer"), { target: { value } });
};
const from = (id: string) => {
  fireEvent.change(screen.getByLabelText("From account"), { target: { value: id } });
};

describe("the Cover it key's identity (personal-cfo-56w.8)", () => {
  it("is the account covered, the account it comes from and the amount", async () => {
    renderWithClient(<Harness />);
    const openAndFill = async () => {
      fireEvent.click(await screen.findByRole("button", { name: /cover it/i }));
      await screen.findByRole("button", { name: /confirm transfer/i });
      amount("200");
      from(SAVINGS);
    };
    await expectDraftKeyIdentity({
      name: "Cover it",
      open: openAndFill,
      send: async () => {
        // Confirm is held until the accounts are re-read after a call that threw.
        await vi.waitFor(() => expect(screen.getByRole("button", { name: /confirm transfer/i })).toBeEnabled());
        fireEvent.click(screen.getByRole("button", { name: /confirm transfer/i }));
      },
      keys: () => mocks.recordTransfer.mock.calls.map((c) => c[0].idempotency_key),
      // No `refusal` step yet: the helper found that Cover it keeps its key after a typed refusal,
      // where docs/agent/FRONTEND.md section 8 says the next try is a new draft. That is a product
      // change, so it gets its own fix (personal-cfo-56w.8 AC4); this spec gains `refusal` with it.
      success: {
        arrange: () => mocks.recordTransfer.mockResolvedValueOnce(ok({ op_seq: 1, replayed: false })),
        // A recorded transfer closes the panel: the same transfer is entered into a freshly opened one.
        reopen: openAndFill,
      },
      fields: [
        { name: "amount", change: () => amount("201") },
        { name: "from account", change: () => from(BROKERAGE) },
        {
          // The account covered comes from the forecast the notice is given: another account's
          // larger shortfall makes it the one. (The currency is the forecast's too, and the sources
          // are filtered to match it, so it cannot change without the account also changing.)
          name: "account covered",
          change: () => void act(() => setAlsoShort(true)),
        },
      ],
    });
  });
});
