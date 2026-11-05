/**
 * personal-cfo-56w.8: the per-draft idempotency key of the batch balance save (personal-cfo-g3m.37),
 * pinned for EVERY field of the row's identity with the shared helper. One key belongs to each row's
 * draft; its own file keeps it clear of the large AccountsView suite.
 */

import { fireEvent, screen } from "@testing-library/react";

import { expectDraftKeyIdentity } from "@/test/idempotencyIdentity";
import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto } from "@/bindings";
import { AccountsView } from "./AccountsView";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  assertBalance: vi.fn(),
  cashTiers: vi.fn(),
  baseCurrency: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    assertBalance: mocks.assertBalance,
    cashTiers: mocks.cashTiers,
    baseCurrency: mocks.baseCurrency,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const money = (minor: number) => ({ minor_units: minor, currency: "USD" });
const CHECKING: AccountViewDto = {
  id: "0190a000-0000-7000-8000-000000000001",
  name: "Checking",
  cashflow_role: "liquid_cash",
  subtype: null,
  active: true,
  balance: money(200_000),
  notes: null,
  linked_account_id: null,
  linked_account_name: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.accountList.mockResolvedValue(ok([CHECKING]));
  mocks.cashTiers.mockResolvedValue(ok({ spendable: money(200_000), reserve: money(0), net: money(200_000) }));
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  // A lost connection: the call throws, the row keeps its draft and its key.
  mocks.assertBalance.mockRejectedValue(new Error("connection lost"));
});

const balance = (value: string) =>
  fireEvent.change(screen.getByLabelText("New balance for Checking"), { target: { value } });
const asOf = (value: string) =>
  fireEvent.change(document.getElementById("batch-asof") as HTMLElement, { target: { value } });
const saveButton = () => screen.getByRole("button", { name: /^save/i });
/// The rows are locked while a save is pending, so a change waits for the last send to settle.
const settled = () => vi.waitFor(() => expect(saveButton()).toBeEnabled());

describe("the batch balance key's identity (personal-cfo-56w.8)", () => {
  it("is the row's balance and the as-of date (the account is the row, the currency its own)", async () => {
    renderWithClient(<AccountsView />);
    await screen.findByText("Checking");
    fireEvent.click(screen.getByRole("button", { name: /update balances/i }));
    await screen.findByLabelText("New balance for Checking");
    const fill = () => {
      balance("2500");
      asOf("2026-06-05");
    };
    await expectDraftKeyIdentity({
      name: "batch balance (one row)",
      open: fill,
      send: async () => {
        // Save is locked until the previous call settles.
        await vi.waitFor(() => expect(saveButton()).toBeEnabled());
        fireEvent.click(saveButton());
      },
      keys: () => mocks.assertBalance.mock.calls.map((c) => c[0].idempotency_key),
      refusal: {
        arrange: () =>
          mocks.assertBalance.mockResolvedValueOnce({ status: "error", error: { Validation: "That amount is too large." } }),
      },
      success: {
        arrange: () =>
          mocks.assertBalance.mockResolvedValueOnce(ok({ balance: money(250_000), unexplained: null })),
        // A saved row drops its draft: the same figure is typed again in the batch that is still open.
        reopen: async () => {
          await vi.waitFor(() => expect(screen.getByLabelText("New balance for Checking")).toHaveValue("2000"));
          fill();
        },
      },
      fields: [
        { name: "balance", change: async () => { await settled(); balance("2600"); } },
        { name: "as-of date", change: async () => { await settled(); asOf("2026-06-06"); } },
      ],
    });
  });
});
