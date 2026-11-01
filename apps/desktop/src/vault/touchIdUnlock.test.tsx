import { act, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import type { VaultStatusDto } from "@/bindings";
import { useVault, VaultProvider, type TouchIdUnlockResult } from "./useVault";

// personal-cfo-xuu (4 of 4): the provider's Touch ID unlock moves the vault status exactly as a
// password unlock does, and only when the host says the vault is open.

const mocks = vi.hoisted(() => ({
  vaultStatus: vi.fn(),
  listVaults: vi.fn(),
  touchIdUnlock: vi.fn(),
}));
vi.mock("@/bindings", () => ({
  commands: {
    vaultStatus: mocks.vaultStatus,
    listVaults: mocks.listVaults,
    touchIdUnlock: mocks.touchIdUnlock,
  },
}));

const locked: VaultStatusDto = { state: "Locked", account_count: null };
const unlocked: VaultStatusDto = { state: "Unlocked", account_count: 3 };

let attempt: () => Promise<TouchIdUnlockResult>;

function Probe() {
  const { status, unlockWithTouchId } = useVault();
  attempt = unlockWithTouchId;
  return <div data-testid="state">{status?.state ?? "loading"}</div>;
}

async function setup() {
  render(
    <QueryClientProvider client={new QueryClient()}>
      <VaultProvider>
        <Probe />
      </VaultProvider>
    </QueryClientProvider>,
  );
  await waitFor(() => expect(screen.getByTestId("state")).toHaveTextContent("Locked"));
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.vaultStatus.mockResolvedValue({ status: "ok", data: locked });
  mocks.listVaults.mockResolvedValue({
    status: "ok",
    data: { vaults: [], restore_recovery_status: "clear" },
  });
});

test("an unlocked outcome flips the status to Unlocked", async () => {
  mocks.touchIdUnlock.mockResolvedValue({
    status: "ok",
    data: { unlocked: true, vault: unlocked, reason: null },
  });
  await setup();
  let result: TouchIdUnlockResult | undefined;
  await act(async () => {
    result = await attempt();
  });
  expect(result).toEqual({ kind: "unlocked" });
  expect(screen.getByTestId("state")).toHaveTextContent("Unlocked");
});

test("a password-required outcome keeps the vault locked and passes the reason on", async () => {
  mocks.touchIdUnlock.mockResolvedValue({
    status: "ok",
    data: { unlocked: false, vault: null, reason: "after_restart" },
  });
  await setup();
  let result: TouchIdUnlockResult | undefined;
  await act(async () => {
    result = await attempt();
  });
  expect(result).toEqual({ kind: "password", reason: "after_restart" });
  expect(screen.getByTestId("state")).toHaveTextContent("Locked");
});

test("an error outcome keeps the vault locked and returns the error", async () => {
  mocks.touchIdUnlock.mockResolvedValue({ status: "error", error: "VaultIntegrityCheckFailed" });
  await setup();
  let result: TouchIdUnlockResult | undefined;
  await act(async () => {
    result = await attempt();
  });
  expect(result).toEqual({ kind: "error", error: "VaultIntegrityCheckFailed" });
  expect(screen.getByTestId("state")).toHaveTextContent("Locked");
});

test("an 'unlocked' answer with no vault status is not treated as unlocked", async () => {
  mocks.touchIdUnlock.mockResolvedValue({
    status: "ok",
    data: { unlocked: true, vault: null, reason: null },
  });
  await setup();
  let result: TouchIdUnlockResult | undefined;
  await act(async () => {
    result = await attempt();
  });
  expect(result?.kind).toBe("password");
  expect(screen.getByTestId("state")).toHaveTextContent("Locked");
});
