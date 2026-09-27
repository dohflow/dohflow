import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";
import { ManualBackupExport } from "./BackupView";

const mocks = vi.hoisted(() => ({
  exportBackup: vi.fn(),
  save: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: { exportBackup: mocks.exportBackup },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: mocks.save }));
vi.mock("@/vault/useVault", () => ({
  describeIpcError: (e: unknown) =>
    typeof e === "string" ? e : "Something went wrong.",
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

beforeEach(() => {
  mocks.exportBackup.mockReset();
  mocks.save.mockReset();
  localStorage.clear();
});

describe("ManualBackupExport", () => {
  it("exports to the chosen path without asking for a password", async () => {
    mocks.save.mockResolvedValue("/home/me/personal-cfo-backup.pcfobk");
    mocks.exportBackup.mockResolvedValue(ok(null));
    renderWithClient(<ManualBackupExport />);

    expect(screen.queryByLabelText(/vault password/i)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /export backup/i }));

    await waitFor(() =>
      expect(mocks.exportBackup).toHaveBeenCalledWith(
        "/home/me/personal-cfo-backup.pcfobk",
      ),
    );
    expect(await screen.findByText(/backup saved to/i)).toBeInTheDocument();
    expect(localStorage.getItem("backup-exported:default")).toBeNull();
  });

  it("does nothing when the save dialog is cancelled", async () => {
    mocks.save.mockResolvedValue(null);
    renderWithClient(<ManualBackupExport />);

    fireEvent.click(screen.getByRole("button", { name: /export backup/i }));

    await waitFor(() => expect(mocks.save).toHaveBeenCalled());
    expect(mocks.exportBackup).not.toHaveBeenCalled();
  });

  it("surfaces an export error", async () => {
    mocks.save.mockResolvedValue("/p.pcfobk");
    mocks.exportBackup.mockResolvedValue({ status: "error", error: "VaultLocked" });
    renderWithClient(<ManualBackupExport />);

    fireEvent.click(screen.getByRole("button", { name: /export backup/i }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });

  it("disables duplicate submissions while the native Save dialog is open", async () => {
    let finishPick!: (path: string | null) => void;
    mocks.save.mockReturnValue(new Promise((resolve) => { finishPick = resolve; }));
    const onBusyChange = vi.fn();
    renderWithClient(<ManualBackupExport onBusyChange={onBusyChange} />);

    const exportButton = screen.getByRole("button", { name: /export backup/i });
    fireEvent.click(exportButton);
    expect(exportButton).toBeDisabled();
    fireEvent.click(exportButton);
    expect(mocks.save).toHaveBeenCalledTimes(1);
    expect(mocks.exportBackup).not.toHaveBeenCalled();
    finishPick(null);
    await waitFor(() => expect(exportButton).toBeEnabled());
    expect(onBusyChange).toHaveBeenNthCalledWith(1, true);
    expect(onBusyChange).toHaveBeenLastCalledWith(false);
  });
});
