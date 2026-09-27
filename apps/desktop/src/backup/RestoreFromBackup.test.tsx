import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { VaultProvider, useVault } from "@/vault/useVault";
import { RestoreFromBackup } from "./RestoreFromBackup";

const mocks = vi.hoisted(() => ({
  vaultStatus: vi.fn(),
  restoreBackup: vi.fn(),
  restoreBackupAsNewVault: vi.fn(),
  listVaults: vi.fn(),
  open: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    vaultStatus: mocks.vaultStatus,
    restoreBackup: mocks.restoreBackup,
    restoreBackupAsNewVault: mocks.restoreBackupAsNewVault,
    // VaultProvider loads the vault registry on mount (j0cg.6); an empty list
    // keeps this suite in single-vault mode without unhandled rejections.
    listVaults: mocks.listVaults,
  },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function StatusProbe() {
  const { status } = useVault();
  return <span data-testid="vault-state">{status?.state}</span>;
}

function renderRestore(mode: "fresh" | "newNamed" = "fresh") {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <VaultProvider>
        <RestoreFromBackup mode={mode} />
        <StatusProbe />
      </VaultProvider>
    </QueryClientProvider>,
  );
  return { ...view, queryClient };
}

beforeEach(() => {
  mocks.vaultStatus.mockReset();
  mocks.restoreBackup.mockReset();
  mocks.restoreBackupAsNewVault.mockReset();
  mocks.open.mockReset();
  mocks.vaultStatus.mockResolvedValue(ok({ state: "NoVault", account_count: null }));
  mocks.listVaults.mockResolvedValue(ok({ vaults: [], restore_recovery_status: "clear" }));
});

describe("Restore as a separate named vault", () => {
  it("keeps the original and submits a trimmed name with the selected backup", async () => {
    mocks.open.mockResolvedValue("/backup.pcfobk");
    mocks.restoreBackupAsNewVault.mockResolvedValue(ok({ state: "Unlocked", account_count: 1 }));
    const { queryClient } = renderRestore("newNamed");
    queryClient.setQueryData(["financial", "original"], { balance: 100 });

    expect(screen.getByText(/original vault and its files stay/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /restore as a new vault/i }));
    await screen.findByLabelText(/new vault name/i);
    fireEvent.change(screen.getByLabelText(/new vault name/i), { target: { value: "  Recovered  " } });
    fireEvent.change(screen.getByLabelText(/backup password/i), { target: { value: "secret" } });
    fireEvent.submit(screen.getByLabelText(/backup password/i).closest("form")!);

    await waitFor(() => expect(mocks.restoreBackupAsNewVault).toHaveBeenCalledWith(
      "/backup.pcfobk", "secret", "Recovered",
    ));
    expect(queryClient.getQueryData(["financial", "original"])).toBeUndefined();
    await waitFor(() => expect(screen.getByTestId("vault-state")).toHaveTextContent("Unlocked"));
    expect(mocks.restoreBackup).not.toHaveBeenCalled();
  });

  it("cancels without a restore and rejects whitespace-only names", async () => {
    mocks.open.mockResolvedValue("/backup.pcfobk");
    renderRestore("newNamed");
    fireEvent.click(screen.getByRole("button", { name: /restore as a new vault/i }));
    await screen.findByLabelText(/new vault name/i);
    fireEvent.change(screen.getByLabelText(/new vault name/i), { target: { value: "   " } });
    fireEvent.change(screen.getByLabelText(/backup password/i), { target: { value: "secret" } });
    expect(screen.getByRole("button", { name: /^restore$/i })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(screen.getByRole("button", { name: /restore as a new vault/i })).toHaveFocus();
    expect(mocks.restoreBackupAsNewVault).not.toHaveBeenCalled();
  });

  it("leaves the form unopened when the native file picker is canceled", async () => {
    mocks.open.mockResolvedValue(null);
    renderRestore("newNamed");
    fireEvent.click(screen.getByRole("button", { name: /restore as a new vault/i }));
    await waitFor(() => expect(mocks.open).toHaveBeenCalledTimes(1));
    expect(screen.queryByLabelText(/new vault name/i)).not.toBeInTheDocument();
    expect(mocks.restoreBackupAsNewVault).not.toHaveBeenCalled();
  });

  it("blocks duplicate submits and cancellation while busy, then allows an error retry", async () => {
    mocks.open.mockResolvedValue("/backup.pcfobk");
    let finish!: (value: { status: "error"; error: "VaultUnlockFailed" }) => void;
    mocks.restoreBackupAsNewVault.mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    mocks.restoreBackupAsNewVault.mockResolvedValueOnce(ok({ state: "Unlocked", account_count: 1 }));
    renderRestore("newNamed");
    fireEvent.click(screen.getByRole("button", { name: /restore as a new vault/i }));
    await screen.findByLabelText(/new vault name/i);
    fireEvent.change(screen.getByLabelText(/new vault name/i), { target: { value: "Recovered" } });
    fireEvent.change(screen.getByLabelText(/backup password/i), { target: { value: "wrong" } });
    const restore = screen.getByRole("button", { name: /^restore$/i });
    fireEvent.click(restore);
    fireEvent.click(restore);
    expect(mocks.restoreBackupAsNewVault).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: /cancel/i })).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent(/restoring backup/i);

    finish({ status: "error", error: "VaultUnlockFailed" });
    expect(await screen.findByRole("alert")).toHaveTextContent(/incorrect password/i);
    fireEvent.change(screen.getByLabelText(/backup password/i), { target: { value: "correct" } });
    fireEvent.click(screen.getByRole("button", { name: /^restore$/i }));
    await waitFor(() => expect(mocks.restoreBackupAsNewVault).toHaveBeenCalledTimes(2));
  });
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
