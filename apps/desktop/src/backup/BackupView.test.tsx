import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { BackupView } from "./BackupView";
import { hasExportedBackup } from "./backupNudgeStorage";

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
  it("exports to the chosen path with the vault password", async () => {
    mocks.save.mockResolvedValue("/home/me/personal-cfo-backup.pcfobk");
    mocks.exportBackup.mockResolvedValue(ok(null));
    render(<BackupView />);

    fireEvent.change(screen.getByLabelText(/vault password/i), {
      target: { value: "s3cret" },
    });
    fireEvent.click(screen.getByRole("button", { name: /export backup/i }));

    await waitFor(() =>
      expect(mocks.exportBackup).toHaveBeenCalledWith(
        "s3cret",
        "/home/me/personal-cfo-backup.pcfobk",
      ),
    );
    expect(await screen.findByText(/backup saved to/i)).toBeInTheDocument();
    // A successful export records the marker that retires the Dashboard's
    // backup nudge (personal-cfo-vdmb).
    expect(hasExportedBackup("default")).toBe(true);
  });

  it("does nothing when the save dialog is cancelled", async () => {
    mocks.save.mockResolvedValue(null);
    render(<BackupView />);

    fireEvent.change(screen.getByLabelText(/vault password/i), {
      target: { value: "x" },
    });
    fireEvent.click(screen.getByRole("button", { name: /export backup/i }));

    await waitFor(() => expect(mocks.save).toHaveBeenCalled());
    expect(mocks.exportBackup).not.toHaveBeenCalled();
  });

  it("surfaces an export error", async () => {
    mocks.save.mockResolvedValue("/p.pcfobk");
    mocks.exportBackup.mockResolvedValue({ status: "error", error: "VaultLocked" });
    render(<BackupView />);

    fireEvent.change(screen.getByLabelText(/vault password/i), {
      target: { value: "x" },
    });
    fireEvent.click(screen.getByRole("button", { name: /export backup/i }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });
});
