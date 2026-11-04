/**
 * personal-cfo-56w.8: the per-draft idempotency key of Set balance (personal-cfo-g3m.37), pinned for
 * EVERY field of the draft's identity with the shared helper. Its own file keeps it clear of the
 * outcome-view suite in SetBalanceModal.test.tsx.
 */

import { fireEvent, screen } from "@testing-library/react";

import { expectDraftKeyIdentity } from "@/test/idempotencyIdentity";
import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto } from "@/bindings";
import { SetBalanceModal } from "./SetBalanceModal";

const mocks = vi.hoisted(() => ({
  assertBalance: vi.fn(),
  accountUnexplained: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    assertBalance: mocks.assertBalance,
    accountUnexplained: mocks.accountUnexplained,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const SAVED = ok({ balance: { minor_units: 50_000, currency: "USD" }, unexplained: null });
const REFUSED = { status: "error", error: { Validation: "That amount is too large." } } as const;

const CHECKING: AccountViewDto = {
  id: "0190a000-0000-7000-8000-000000000001",
  name: "Checking",
  cashflow_role: "liquid_cash",
  subtype: null,
  active: true,
  balance: { minor_units: 0, currency: "USD" },
  notes: null,
  linked_account_id: null,
  linked_account_name: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  // A lost connection: the call throws, the modal stays on the form and the key stays with the draft.
  mocks.assertBalance.mockRejectedValue(new Error("connection lost"));
  mocks.accountUnexplained.mockResolvedValue(ok(null));
});

const change = (label: RegExp, value: string) => {
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
};
const fill = () => {
  change(/new balance/i, "500");
  change(/as of/i, "2026-06-05");
};
const view = () =>
  renderWithClient(
    <SetBalanceModal account={CHECKING} onClose={() => {}} onExplainWithTransactions={() => {}} />,
  );

describe("the Set balance key's identity (personal-cfo-56w.8)", () => {
  it("is the balance and the date (the account and currency are the modal's own)", async () => {
    let rendered = view();
    await expectDraftKeyIdentity({
      name: "Set balance",
      open: fill,
      send: async () => {
        // The button is disabled until the previous call settles.
        await vi.waitFor(() => expect(screen.getByRole("button", { name: /^set balance$/i })).toBeEnabled());
        fireEvent.click(screen.getByRole("button", { name: /^set balance$/i }));
      },
      keys: () => mocks.assertBalance.mock.calls.map((c) => c[0].idempotency_key),
      refusal: { arrange: () => mocks.assertBalance.mockResolvedValueOnce(REFUSED) },
      success: {
        arrange: () => mocks.assertBalance.mockResolvedValueOnce(SAVED),
        // A saved balance swaps the form for its outcome: the same draft is typed into a fresh modal.
        reopen: () => {
          rendered.unmount();
          rendered = view();
          fill();
        },
      },
      fields: [
        { name: "new balance", change: () => change(/new balance/i, "501") },
        { name: "as of date", change: () => change(/as of/i, "2026-06-06") },
      ],
    });
  });
});
