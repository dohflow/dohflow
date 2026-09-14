import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto } from "@/bindings";
import { ReconcileBalanceModal } from "./ReconcileBalanceModal";

const mocks = vi.hoisted(() => ({
  transactionList: vi.fn(),
  recordTransaction: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    transactionList: mocks.transactionList,
    recordTransaction: mocks.recordTransaction,
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
    balance: { minor_units: 100_000, currency: "USD" }, // $1,000.00
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  };
}

beforeEach(() => {
  mocks.transactionList.mockReset();
  mocks.recordTransaction.mockReset();
  mocks.transactionList.mockResolvedValue(ok([]));
  mocks.recordTransaction.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
});

describe("ReconcileBalanceModal", () => {
  it("computes the remaining delta from the target", () => {
    renderWithClient(
      <ReconcileBalanceModal account={account()} onClose={() => {}} />,
    );
    // The target prefills to the current balance, so the gap starts reconciled.
    expect(screen.getByText(/reconciled/i)).toBeInTheDocument();

    // Raising the target to $1,200 leaves +$200 remaining.
    fireEvent.change(screen.getByLabelText(/target balance/i), {
      target: { value: "1200" },
    });
    expect(screen.getByText(/\+\$200\.00/)).toBeInTheDocument();
  });

  it("posts a reconciling transaction and decrements the remaining", async () => {
    renderWithClient(
      <ReconcileBalanceModal account={account()} onClose={() => {}} />,
    );
    fireEvent.change(screen.getByLabelText(/target balance/i), {
      target: { value: "1200" },
    });
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "150" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add & new/i }));

    await waitFor(() =>
      expect(mocks.recordTransaction).toHaveBeenCalledTimes(1),
    );
    const input = mocks.recordTransaction.mock.calls[0]?.[0];
    expect(input.account_id).toBe(account().id);
    // +$150 money-in (the gap-closing direction by default).
    expect(input.amount.minor_units).toBe(15_000);
    // Remaining drops from +$200 to +$50.
    await waitFor(() =>
      expect(screen.getByText(/\+\$50\.00/)).toBeInTheDocument(),
    );
  });

  it("reconciles to zero across multiple transactions", async () => {
    renderWithClient(
      <ReconcileBalanceModal account={account()} onClose={() => {}} />,
    );
    fireEvent.change(screen.getByLabelText(/target balance/i), {
      target: { value: "1200" },
    });

    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "150" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add & new/i }));
    await waitFor(() =>
      expect(screen.getByText(/\+\$50\.00/)).toBeInTheDocument(),
    );

    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "50" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add & new/i }));

    await waitFor(() =>
      expect(screen.getByText(/reconciled/i)).toBeInTheDocument(),
    );
    expect(mocks.recordTransaction).toHaveBeenCalledTimes(2);
  });
});
