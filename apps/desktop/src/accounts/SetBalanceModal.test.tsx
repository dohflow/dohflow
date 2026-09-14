import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto } from "@/bindings";
import { SetBalanceModal } from "./SetBalanceModal";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  assertBalance: vi.fn(),
  convertUnexplainedToTransaction: vi.fn(),
  accountUnexplained: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    assertBalance: mocks.assertBalance,
    convertUnexplainedToTransaction: mocks.convertUnexplainedToTransaction,
    accountUnexplained: mocks.accountUnexplained,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function account(over: Partial<AccountViewDto> = {}): AccountViewDto {
  return {
    id: "0190a000-0000-7000-8000-000000000001",
    name: "Checking",
    cashflow_role: "liquid_cash",
    subtype: null,
    active: true,
    balance: { minor_units: 0, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.accountList.mockResolvedValue(ok([account()]));
  // Asserting leaves a $500 unexplained plug.
  mocks.assertBalance.mockResolvedValue(
    ok({
      balance: { minor_units: 50_000, currency: "USD" },
      unexplained: { minor_units: 50_000, currency: "USD" },
    }),
  );
  mocks.convertUnexplainedToTransaction.mockResolvedValue(
    ok({ op_seq: 1, replayed: false }),
  );
  // The modal re-reads the plug after converting (yl53 review) — cleared here.
  mocks.accountUnexplained.mockResolvedValue(ok(null));
});

describe("SetBalanceModal", () => {
  it("converts the unexplained plug into a transaction (dyy4)", async () => {
    renderWithClient(
      <SetBalanceModal
        account={account()}
        onClose={() => {}}
        onExplainWithTransactions={() => {}}
      />,
    );

    fireEvent.change(screen.getByLabelText(/new balance/i), {
      target: { value: "500" },
    });
    fireEvent.click(screen.getByRole("button", { name: /set balance/i }));

    // The outcome surfaces the plug + the convert action.
    expect(
      await screen.findByText(/unexplained adjustment/i),
    ).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: /record as a transaction/i }),
    );

    await waitFor(() =>
      expect(mocks.convertUnexplainedToTransaction).toHaveBeenCalledWith(
        "0190a000-0000-7000-8000-000000000001",
        expect.stringMatching(/.+/),
      ),
    );
    // Once converted, the view reports it's fully explained.
    expect(
      await screen.findByText(/fully explained/i),
    ).toBeInTheDocument();
  });

  it("keeps showing a residual the conversion did not clear (yl53)", async () => {
    mocks.accountUnexplained.mockResolvedValue(
      ok({ minor_units: 26_544, currency: "USD" }),
    );
    renderWithClient(
      <SetBalanceModal
        account={account()}
        onClose={() => {}}
        onExplainWithTransactions={() => {}}
      />,
    );
    fireEvent.change(screen.getByLabelText(/new balance/i), {
      target: { value: "500" },
    });
    fireEvent.click(screen.getByRole("button", { name: /set balance/i }));
    expect(
      await screen.findByText(/unexplained adjustment/i),
    ).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: /record as a transaction/i }),
    );
    await waitFor(() =>
      expect(mocks.accountUnexplained).toHaveBeenCalledTimes(1),
    );
    // The backend still reports a plug — the view must NOT claim it cleared.
    expect(screen.queryByText(/fully explained/i)).not.toBeInTheDocument();
    expect(await screen.findByText(/unexplained adjustment/i)).toBeInTheDocument();
  });
});
