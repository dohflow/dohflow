import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { BackupNudge } from "./BackupNudge";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  backupHistory: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    backupHistory: mocks.backupHistory,
  },
}));
// Single-vault mode: an empty registry keys the markers under "default".
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({ vaults: [] }),
  describeIpcError: () => "Something went wrong.",
}));

const oneAccount = {
  id: "acc-1",
  name: "Checking",
  cashflow_role: "liquid_cash",
  subtype: null,
  active: true,
  balance: { minor_units: 50_000, currency: "USD" },
};

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

const NUDGE_COPY = /dohflow keeps no cloud copy/i;

beforeEach(() => {
  mocks.accountList.mockReset();
  mocks.backupHistory.mockReset();
  mocks.accountList.mockResolvedValue(ok([oneAccount]));
  mocks.backupHistory.mockResolvedValue(ok([]));
  localStorage.clear();
});

describe("BackupNudge", () => {
  it("shows for a vault with data until history proves a verified backup exists", async () => {
    renderWithClient(<BackupNudge onOpenBackup={vi.fn()} />);
    expect(await screen.findByText(NUDGE_COPY)).toBeInTheDocument();
  });

  it("stays hidden on an empty vault", async () => {
    mocks.accountList.mockResolvedValue(ok([]));
    renderWithClient(<BackupNudge onOpenBackup={vi.fn()} />);
    await waitFor(() => expect(mocks.accountList).toHaveBeenCalled());
    expect(screen.queryByText(NUDGE_COPY)).not.toBeInTheDocument();
  });

  it("stays hidden once the vault history contains a verified backup", async () => {
    mocks.backupHistory.mockResolvedValue(
      ok([{ verified: true, backup_id: "backup-1" }]),
    );
    renderWithClient(<BackupNudge onOpenBackup={vi.fn()} />);
    await waitFor(() => expect(mocks.accountList).toHaveBeenCalled());
    expect(screen.queryByText(NUDGE_COPY)).not.toBeInTheDocument();
  });

  it("does not trust a legacy localStorage exported marker", async () => {
    localStorage.setItem("backup-exported:default", new Date().toISOString());
    renderWithClient(<BackupNudge onOpenBackup={vi.fn()} />);
    expect(await screen.findByText(NUDGE_COPY)).toBeInTheDocument();
    expect(mocks.backupHistory).toHaveBeenCalledTimes(1);
  });

  it("routes to the Backup tab", async () => {
    const onOpenBackup = vi.fn();
    renderWithClient(<BackupNudge onOpenBackup={onOpenBackup} />);
    fireEvent.click(
      await screen.findByRole("button", { name: /open backup/i }),
    );
    expect(onOpenBackup).toHaveBeenCalledTimes(1);
  });

  it("dismisses immediately and persists across a remount", async () => {
    const first = renderWithClient(<BackupNudge onOpenBackup={vi.fn()} />);
    fireEvent.click(
      await screen.findByRole("button", { name: /dismiss backup reminder/i }),
    );
    expect(screen.queryByText(NUDGE_COPY)).not.toBeInTheDocument();
    first.unmount();

    // A fresh mount (new session) reads the persisted dismissal.
    renderWithClient(<BackupNudge onOpenBackup={vi.fn()} />);
    await waitFor(() => expect(mocks.accountList).toHaveBeenCalledTimes(2));
    expect(screen.queryByText(NUDGE_COPY)).not.toBeInTheDocument();
  });
});
