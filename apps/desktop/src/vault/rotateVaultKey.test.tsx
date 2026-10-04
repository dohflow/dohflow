import { act, fireEvent, render, screen } from "@testing-library/react";
import { onlineManager, QueryClientProvider } from "@tanstack/react-query";

import { useBackupHistory } from "@/backup/useBackup";
import { createQueryClient } from "@/lib/queryClient";

import { VaultProvider, useVault } from "./useVault";

const mocks = vi.hoisted(() => ({
  vaultStatus: vi.fn(),
  listVaults: vi.fn(),
  rotateVaultKey: vi.fn(),
  backupHistory: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    vaultStatus: mocks.vaultStatus,
    listVaults: mocks.listVaults,
    rotateVaultKey: mocks.rotateVaultKey,
    backupHistory: mocks.backupHistory,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

/// A polled query (backup history polls every 10 s) beside the rotate action.
function Probe() {
  useBackupHistory();
  const { rotateVaultKey } = useVault();
  return (
    <button type="button" onClick={() => void rotateVaultKey("pw")}>
      rotate
    </button>
  );
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  vi.clearAllMocks();
  mocks.vaultStatus.mockResolvedValue(ok({ state: "Unlocked", account_count: 1 }));
  mocks.listVaults.mockResolvedValue(
    ok({ vaults: [], active_id: null, restore_recovery: "clear" }),
  );
  mocks.backupHistory.mockResolvedValue(ok([]));
});

afterEach(() => {
  vi.useRealTimers();
  onlineManager.setOnline(true);
});

test("no polled command fires while a key rotation is pending, and polling resumes after", async () => {
  let finish: (value: unknown) => void = () => {};
  mocks.rotateVaultKey.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  render(
    <QueryClientProvider client={createQueryClient()}>
      <VaultProvider>
        <Probe />
      </VaultProvider>
    </QueryClientProvider>,
  );
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  const initialPolls = mocks.backupHistory.mock.calls.length;
  expect(initialPolls).toBeGreaterThan(0);

  fireEvent.click(screen.getByRole("button", { name: "rotate" }));
  expect(mocks.rotateVaultKey).toHaveBeenCalledWith({ password: "pw" });
  expect(onlineManager.isOnline()).toBe(false);

  // Three poll intervals pass while the rotation holds the vault lock.
  await act(async () => {
    await vi.advanceTimersByTimeAsync(35_000);
  });
  expect(mocks.backupHistory.mock.calls.length).toBe(initialPolls);
  expect(mocks.vaultStatus).toHaveBeenCalledTimes(1); // only the mount load

  await act(async () => {
    finish(ok({ state: "Unlocked", account_count: 1 }));
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(onlineManager.isOnline()).toBe(true);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(11_000);
  });
  expect(mocks.backupHistory.mock.calls.length).toBeGreaterThan(initialPolls);
});

test("polling resumes even when the rotation fails", async () => {
  mocks.rotateVaultKey.mockResolvedValue({
    status: "error",
    error: "VaultUnlockFailed",
  });
  render(
    <QueryClientProvider client={createQueryClient()}>
      <VaultProvider>
        <Probe />
      </VaultProvider>
    </QueryClientProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "rotate" }));
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(onlineManager.isOnline()).toBe(true);
});
