import { act, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import type { VaultStatusDto } from "@/bindings";
import { useVault, VaultProvider } from "./useVault";

// personal-cfo-i93n3.13: a host can lock the vault on its own (backgrounding, a device lock),
// so the provider re-reads the status when the page becomes visible again.

const mocks = vi.hoisted(() => ({
  vaultStatus: vi.fn(),
  listVaults: vi.fn(),
}));
vi.mock("@/bindings", () => ({
  commands: { vaultStatus: mocks.vaultStatus, listVaults: mocks.listVaults },
}));

const unlocked: VaultStatusDto = { state: "Unlocked", account_count: 3 };
const locked: VaultStatusDto = { state: "Locked", account_count: null };

function Probe() {
  const { status } = useVault();
  return <div data-testid="state">{status?.state ?? "loading"}</div>;
}

function setup() {
  const client = new QueryClient();
  client.setQueryData(["balances"], { secret: "cached financial data" });
  render(
    <QueryClientProvider client={client}>
      <VaultProvider>
        <Probe />
      </VaultProvider>
    </QueryClientProvider>,
  );
  return client;
}

function setVisibility(state: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", { configurable: true, value: state });
  act(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.listVaults.mockResolvedValue({
    status: "ok",
    data: { vaults: [], restore_recovery_status: "clear" },
  });
  setVisibility("visible");
});

test("becoming visible with the vault still unlocked changes nothing", async () => {
  mocks.vaultStatus.mockResolvedValue({ status: "ok", data: unlocked });
  const client = setup();
  await waitFor(() => expect(screen.getByTestId("state")).toHaveTextContent("Unlocked"));
  mocks.vaultStatus.mockClear();

  setVisibility("hidden");
  setVisibility("visible");
  await waitFor(() => expect(mocks.vaultStatus).toHaveBeenCalledTimes(1));
  expect(screen.getByTestId("state")).toHaveTextContent("Unlocked");
  expect(client.getQueryData(["balances"])).toBeDefined();
});

test("becoming visible after the host locked the vault shows it locked and drops the cache", async () => {
  mocks.vaultStatus.mockResolvedValue({ status: "ok", data: unlocked });
  const client = setup();
  await waitFor(() => expect(screen.getByTestId("state")).toHaveTextContent("Unlocked"));

  mocks.vaultStatus.mockResolvedValue({ status: "ok", data: locked });
  setVisibility("visible");
  await waitFor(() => expect(screen.getByTestId("state")).toHaveTextContent("Locked"));
  expect(client.getQueryData(["balances"])).toBeUndefined();
});

test("a hidden page re-reads nothing, and a vault that is already locked is not re-read", async () => {
  mocks.vaultStatus.mockResolvedValue({ status: "ok", data: locked });
  setup();
  await waitFor(() => expect(screen.getByTestId("state")).toHaveTextContent("Locked"));
  mocks.vaultStatus.mockClear();
  setVisibility("hidden");
  setVisibility("visible");
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(mocks.vaultStatus).not.toHaveBeenCalled();
});

test("a failed read leaves the screen as it is", async () => {
  mocks.vaultStatus.mockResolvedValue({ status: "ok", data: unlocked });
  setup();
  await waitFor(() => expect(screen.getByTestId("state")).toHaveTextContent("Unlocked"));
  mocks.vaultStatus.mockRejectedValue(new Error("service down"));
  setVisibility("visible");
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(screen.getByTestId("state")).toHaveTextContent("Unlocked");
});
