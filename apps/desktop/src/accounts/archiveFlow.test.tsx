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
    expect(archive).toHaveBeenCalledWith("acc-1", expect.any(String));
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

  // A retry after an outcome nobody could confirm must be safe (planning's condition on slice 3): the first attempt may
  // have committed, so the retry carries the SAME key and the host replays it.
  describe("a retry after an unknown outcome", () => {
    it("reuses the key of the attempt whose outcome is unknown, so a first attempt that committed is replayed", async () => {
      const keys: string[] = [];
      const archive = vi
        .fn()
        .mockImplementationOnce(async (_id: string, key: string) => {
          keys.push(key);
          throw new Error("connection lost after the host committed");
        })
        .mockImplementationOnce(async (_id: string, key: string) => {
          keys.push(key);
          return null; // the host replays the committed attempt: ok
        });
      const { result } = renderHook(() => useArchiveFlow(ACCOUNT, archive));
      act(() => result.current.request());
      await act(async () => result.current.confirmInSheet());
      expect(result.current.error).toBe(ARCHIVE_OUTCOME_UNKNOWN);
      expect(result.current.confirming).toBe(true);
      await act(async () => result.current.confirmInSheet());
      expect(keys).toHaveLength(2);
      expect(keys[0]).toBeTruthy();
      expect(keys[1]).toBe(keys[0]);
      expect(result.current.error).toBeNull();
      expect(result.current.confirming).toBe(false);
    });

    it("reuses it on the desktop path too (confirm, then request and confirm again)", async () => {
      const keys: string[] = [];
      const archive = vi
        .fn()
        .mockImplementationOnce(async (_id: string, key: string) => {
          keys.push(key);
          throw new Error("lost");
        })
        .mockImplementationOnce(async (_id: string, key: string) => {
          keys.push(key);
          return null;
        });
      const { result } = renderHook(() => useArchiveFlow(ACCOUNT, archive));
      act(() => result.current.request());
      await act(async () => result.current.confirm());
      expect(result.current.error).toBe(ARCHIVE_OUTCOME_UNKNOWN);
      act(() => result.current.request());
      await act(async () => result.current.confirm());
      expect(keys[1]).toBe(keys[0]);
      expect(result.current.error).toBeNull();
    });

    it("a confirmed refusal drops the key: the next attempt is a new one with a new key", async () => {
      const keys: string[] = [];
      const archive = vi
        .fn()
        .mockImplementationOnce(async (_id: string, key: string) => {
          keys.push(key);
          return { Validation: "Not now." };
        })
        .mockImplementationOnce(async (_id: string, key: string) => {
          keys.push(key);
          return null;
        });
      const { result } = renderHook(() => useArchiveFlow(ACCOUNT, archive));
      act(() => result.current.request());
      await act(async () => result.current.confirmInSheet());
      await act(async () => result.current.confirmInSheet());
      expect(keys[1]).not.toBe(keys[0]);
    });

    it("never reuses a pending key for a different account (an unknown outcome on one account is not another's retry)", async () => {
      const keys: string[] = [];
      const archive = vi.fn().mockImplementation(async (_id: string, key: string) => {
        keys.push(key);
        if (keys.length === 1) throw new Error("lost");
        return null;
      });
      const other = { ...ACCOUNT, id: "acc-2", name: "Savings" } as AccountViewDto;
      const { result, rerender } = renderHook(({ account }) => useArchiveFlow(account, archive), {
        initialProps: { account: ACCOUNT },
      });
      act(() => result.current.request());
      await act(async () => result.current.confirmInSheet());
      rerender({ account: other });
      act(() => result.current.request());
      await act(async () => result.current.confirmInSheet());
      expect(keys[1]).not.toBe(keys[0]);
    });

    it("a confirmed success drops the key too (archiving again later is a new archive)", async () => {
      const keys: string[] = [];
      const archive = vi.fn().mockImplementation(async (_id: string, key: string) => {
        keys.push(key);
        return null;
      });
      const { result } = renderHook(() => useArchiveFlow(ACCOUNT, archive));
      act(() => result.current.request());
      await act(async () => result.current.confirmInSheet());
      act(() => result.current.request());
      await act(async () => result.current.confirmInSheet());
      expect(keys[1]).not.toBe(keys[0]);
    });
  });
});
