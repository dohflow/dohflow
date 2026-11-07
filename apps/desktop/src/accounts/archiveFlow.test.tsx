import { act, renderHook } from "@testing-library/react";

import type { AccountViewDto } from "@/bindings";
import { ARCHIVE_OUTCOME_UNKNOWN, useArchiveFlow } from "./archiveFlow";

const ACCOUNT = {
  id: "acc-1",
  name: "Checking",
  cashflow_role: "liquid_cash",
  balance: { minor_units: 200_000, currency: "USD" },
} as AccountViewDto;

// The phone's confirmation keeps the question open while the archive runs and when it fails (personal-cfo-i93n3.28).
describe("useArchiveFlow.confirmInSheet", () => {
  it("closes the question only when the archive worked", async () => {
    const archive = vi.fn().mockResolvedValue(null);
    const { result } = renderHook(() => useArchiveFlow(ACCOUNT, archive));
    act(() => result.current.request());
    expect(result.current.confirming).toBe(true);
    await act(async () => result.current.confirmInSheet());
    expect(archive).toHaveBeenCalledWith("acc-1");
    expect(result.current.confirming).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it("stays open with the failure when the archive is refused, and clears it on the next attempt", async () => {
    const archive = vi.fn().mockResolvedValueOnce({ Validation: "Not now." }).mockResolvedValueOnce(null);
    const { result } = renderHook(() => useArchiveFlow(ACCOUNT, archive));
    act(() => result.current.request());
    await act(async () => result.current.confirmInSheet());
    expect(result.current.confirming).toBe(true);
    expect(result.current.error).toContain("Not now.");
    expect(result.current.busy).toBe(false);
    await act(async () => result.current.confirmInSheet());
    expect(result.current.confirming).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it("stays open with the unknown-outcome message when the call threw", async () => {
    const archive = vi.fn().mockRejectedValue(new Error("connection lost"));
    const { result } = renderHook(() => useArchiveFlow(ACCOUNT, archive));
    act(() => result.current.request());
    await act(async () => result.current.confirmInSheet());
    expect(result.current.confirming).toBe(true);
    expect(result.current.error).toBe(ARCHIVE_OUTCOME_UNKNOWN);
  });

  it("does nothing while a call is already running (no second archive)", async () => {
    let finish!: (value: null) => void;
    const archive = vi.fn().mockReturnValue(new Promise<null>((resolve) => (finish = resolve)));
    const { result } = renderHook(() => useArchiveFlow(ACCOUNT, archive));
    act(() => result.current.request());
    let first!: Promise<void>;
    act(() => {
      first = result.current.confirmInSheet();
    });
    await act(async () => result.current.confirmInSheet());
    expect(archive).toHaveBeenCalledTimes(1);
    await act(async () => {
      finish(null);
      await first;
    });
    expect(result.current.confirming).toBe(false);
  });

  it("dismissSheet closes it, but not while a call is running", async () => {
    let finish!: (value: null) => void;
    const archive = vi.fn().mockReturnValue(new Promise<null>((resolve) => (finish = resolve)));
    const { result } = renderHook(() => useArchiveFlow(ACCOUNT, archive));
    act(() => result.current.request());
    let running!: Promise<void>;
    act(() => {
      running = result.current.confirmInSheet();
    });
    act(() => result.current.dismissSheet());
    expect(result.current.confirming).toBe(true);
    await act(async () => {
      finish(null);
      await running;
    });
    act(() => result.current.request());
    act(() => result.current.dismissSheet());
    expect(result.current.confirming).toBe(false);
  });
});
