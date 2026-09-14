import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { VaultProvider } from "@/vault/useVault";
import { RestoreFromBackup } from "./RestoreFromBackup";

const mocks = vi.hoisted(() => ({
  vaultStatus: vi.fn(),
  restoreBackup: vi.fn(),
  listVaults: vi.fn(),
  open: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    vaultStatus: mocks.vaultStatus,
    restoreBackup: mocks.restoreBackup,
    // VaultProvider loads the vault registry on mount (j0cg.6); an empty list
    // keeps this suite in single-vault mode without unhandled rejections.
    listVaults: mocks.listVaults,
  },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function renderRestore() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <VaultProvider>
        <RestoreFromBackup />
      </VaultProvider>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  mocks.vaultStatus.mockReset();
  mocks.restoreBackup.mockReset();
  mocks.open.mockReset();
  mocks.vaultStatus.mockResolvedValue(ok({ state: "NoVault", account_count: null }));
  mocks.listVaults.mockResolvedValue(ok({ vaults: [] }));
});

describe("RestoreFromBackup", () => {
  it("picks a backup file, takes its password, and restores", async () => {
    mocks.open.mockResolvedValue("/home/me/personal-cfo-backup.pcfobk");
    mocks.restoreBackup.mockResolvedValue(ok({ state: "Unlocked", account_count: 1 }));
    renderRestore();

    fireEvent.click(screen.getByRole("button", { name: /restore from backup/i }));
    expect(
      await screen.findByText("/home/me/personal-cfo-backup.pcfobk"),
    ).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText(/backup password/i), {
      target: { value: "pw" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^restore$/i }));

    await waitFor(() =>
      expect(mocks.restoreBackup).toHaveBeenCalledWith(
        "/home/me/personal-cfo-backup.pcfobk",
        "pw",
      ),
    );
  });

  it("surfaces a restore error (wrong password)", async () => {
    mocks.open.mockResolvedValue("/p.pcfobk");
    mocks.restoreBackup.mockResolvedValue({
      status: "error",
      error: "VaultUnlockFailed",
    });
    renderRestore();

    fireEvent.click(screen.getByRole("button", { name: /restore from backup/i }));
    await screen.findByLabelText(/backup password/i);
    fireEvent.change(screen.getByLabelText(/backup password/i), {
      target: { value: "wrong" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^restore$/i }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });
});
