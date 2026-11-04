import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";

import type { AccountViewDto } from "@/bindings";

import { useBatchBalanceDraft } from "./useBatchBalanceDraft";

const mocks = vi.hoisted(() => ({ assertBalance: vi.fn() }));
vi.mock("@/bindings", () => ({ commands: { assertBalance: mocks.assertBalance } }));

const checking: AccountViewDto = {
  id: "0190a000-0000-7000-8000-000000000001",
  name: "Checking",
  cashflow_role: "liquid_cash",
  subtype: null,
  active: true,
  balance: { minor_units: 100_000, currency: "USD" },
  notes: null,
  linked_account_id: null,
  linked_account_name: null,
};

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return renderHook(() => useBatchBalanceDraft(), { wrapper });
}

describe("the batch draft while a save is pending (personal-cfo-g3m.37)", () => {
  it("reset and setDraft do nothing until the save settles, so a new draft cannot be deleted when it resolves", async () => {
    let finish: (value: unknown) => void = () => {};
    mocks.assertBalance.mockReset();
    mocks.assertBalance.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    const { result } = setup();

    act(() => result.current.setDraft(checking.id, "2500"));
    act(() => {
      void result.current.saveAll([checking]);
    });
    await waitFor(() => expect(result.current.saving).toBe(true));

    // Both are ignored from a stale closure too: the submitted draft and result stay.
    act(() => result.current.reset());
    act(() => result.current.setDraft(checking.id, "9999"));
    expect(result.current.valueOf(checking)).toBe("2500");
    expect(result.current.saving).toBe(true);
    // A second save does not start over the first.
    await act(async () => {
      void result.current.saveAll([checking]);
    });
    expect(mocks.assertBalance).toHaveBeenCalledTimes(1);

    await act(async () =>
      finish({ status: "ok", data: { balance: { minor_units: 250_000, currency: "USD" }, unexplained: null } }),
    );
    await waitFor(() => expect(result.current.saving).toBe(false));
    expect(result.current.resultOf(checking.id)).toEqual({ kind: "ok", confirmed: true });
    // Settled: reset works again.
    act(() => result.current.reset());
    expect(result.current.resultOf(checking.id)).toBeUndefined();
  });
});
