import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";
import { BackupView } from "./BackupView";

const mocks = vi.hoisted(() => ({
  exportBackup: vi.fn(),
  save: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: { exportBackup: mocks.exportBackup },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: mocks.save }));
// The view reads the vault registry only to key the backup-exported marker
// (personal-cfo-vdmb); single-vault mode (empty registry) is fine here.
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({ vaults: [] }),
  describeIpcError: (e: unknown) =>
    typeof e === "string" ? e : "Something went wrong.",
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

beforeEach(() => {
  mocks.exportBackup.mockReset();
  mocks.save.mockReset();
  localStorage.clear();
});

describe("BackupView", () => {
  it("exports to the chosen path without asking for a password", async () => {
    mocks.save.mockResolvedValue("/home/me/personal-cfo-backup.pcfobk");
    mocks.exportBackup.mockResolvedValue(ok(null));
    renderWithClient(<BackupView />);

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
    renderWithClient(<BackupView />);

    fireEvent.click(screen.getByRole("button", { name: /export backup/i }));

    await waitFor(() => expect(mocks.save).toHaveBeenCalled());
    expect(mocks.exportBackup).not.toHaveBeenCalled();
  });

  it("surfaces an export error", async () => {
    mocks.save.mockResolvedValue("/p.pcfobk");
    mocks.exportBackup.mockResolvedValue({ status: "error", error: "VaultLocked" });
    renderWithClient(<BackupView />);

    fireEvent.click(screen.getByRole("button", { name: /export backup/i }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });
});
