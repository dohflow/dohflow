import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";
import { BackupScheduleCard } from "./BackupScheduleCard";

const mocks = vi.hoisted(() => ({
  backupScheduleSettings: vi.fn(),
  backupHistory: vi.fn(),
  configureBackup: vi.fn(),
  runBackupNow: vi.fn(),
  open: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    backupScheduleSettings: mocks.backupScheduleSettings,
    backupHistory: mocks.backupHistory,
    configureBackup: mocks.configureBackup,
    runBackupNow: mocks.runBackupNow,
  },
}));
vi.mock("@tauri-apps/api/path", () => ({
  homeDir: vi.fn().mockResolvedValue("/Users/test"),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));
vi.mock("@/vault/useVault", () => ({
  describeIpcError: (error: unknown) => String(error),
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const initialSettings = {
  cadence: "weekly",
  destination: null,
  next_due_at: null,
  last_run_at: null,
  last_error: null,
} as const;

beforeEach(() => {
  mocks.backupScheduleSettings.mockReset();
  mocks.backupHistory.mockReset();
  mocks.configureBackup.mockReset();
  mocks.runBackupNow.mockReset();
  mocks.open.mockReset();
  mocks.backupScheduleSettings.mockResolvedValue(ok(initialSettings));
  mocks.backupHistory.mockResolvedValue(ok([]));
  mocks.configureBackup.mockResolvedValue(ok(initialSettings));
  mocks.runBackupNow.mockResolvedValue(
    ok({ destination: "/Users/test/Backups/new.pcfobk" }),
  );
  mocks.open.mockResolvedValue(null);
});

describe("BackupScheduleCard", () => {
  it("saves the selected local folder and cadence with keep-all behavior", async () => {
    const folder = "/Users/test/Library/Mobile Documents/com~apple~CloudDocs/Backups";
    const savedSettings = {
      ...initialSettings,
      cadence: "monthly",
      destination: folder,
    };
    mocks.open.mockResolvedValue(folder);
    mocks.backupScheduleSettings
      .mockResolvedValueOnce(ok(initialSettings))
      .mockResolvedValue(ok(savedSettings));
    mocks.configureBackup.mockResolvedValue(ok(savedSettings));
    mocks.runBackupNow.mockResolvedValue(ok({ destination: `${folder}/new.pcfobk` }));
    renderWithClient(<BackupScheduleCard />);

    fireEvent.click(await screen.findByRole("button", { name: /choose folder/i }));
    expect(
      await screen.findByText(
        "This folder is synced by iCloud Drive; your encrypted backups will follow it.",
      ),
    ).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Schedule"), {
      target: { value: "monthly" },
    });
    fireEvent.click(screen.getByRole("button", { name: /save backup settings/i }));

    await waitFor(() =>
      expect(mocks.configureBackup).toHaveBeenCalledWith("monthly", folder),
    );
    expect(await screen.findByText("Backup settings saved.")).toBeInTheDocument();
    expect(screen.queryByLabelText("Retention")).not.toBeInTheDocument();
    expect(
      screen.getByText(/dohflow keeps all backups and does not delete older copies automatically/i),
    ).toBeInTheDocument();

    const backupNowButton = screen.getByRole("button", { name: /back up now/i });
    await waitFor(() => expect(backupNowButton).toBeEnabled());
    fireEvent.click(backupNowButton);
    await waitFor(() => expect(mocks.runBackupNow).toHaveBeenCalledTimes(1));
    expect(
      await screen.findByText(`Backup created and verified at ${folder}/new.pcfobk.`),
    ).toBeInTheDocument();
  });

  it("does not run now while the first selected folder is unsaved", async () => {
    mocks.open.mockResolvedValue("/Users/test/Backups/B");
    renderWithClient(<BackupScheduleCard />);

    fireEvent.click(await screen.findByRole("button", { name: /choose folder/i }));
    const backupNowButton = screen.getByRole("button", { name: /back up now/i });

    expect(backupNowButton).toBeDisabled();
    expect(
      await screen.findByText("Save backup settings before using Back up now."),
    ).toBeInTheDocument();
    fireEvent.click(backupNowButton);
    expect(mocks.runBackupNow).not.toHaveBeenCalled();
  });

  it("does not run now against saved folder A while unsaved folder B is displayed", async () => {
    const folderA = "/Users/test/Backups/A";
    const folderB = "/Users/test/Backups/B";
    mocks.backupScheduleSettings.mockResolvedValue(
      ok({ ...initialSettings, destination: folderA }),
    );
    mocks.open.mockResolvedValue(folderB);
    renderWithClient(<BackupScheduleCard />);

    fireEvent.click(await screen.findByRole("button", { name: /choose folder/i }));

    expect(await screen.findByText(folderB)).toBeInTheDocument();
    const backupNowButton = screen.getByRole("button", { name: /back up now/i });
    expect(backupNowButton).toBeDisabled();
    expect(
      await screen.findByText("Save backup settings before using Back up now."),
    ).toBeInTheDocument();
    fireEvent.click(backupNowButton);
    expect(mocks.runBackupNow).not.toHaveBeenCalled();
  });

  it("runs a verified backup now in the saved folder", async () => {
    mocks.backupScheduleSettings.mockResolvedValue(
      ok({
        ...initialSettings,
        destination: "/Users/test/Backups",
      }),
    );
    renderWithClient(<BackupScheduleCard />);
    fireEvent.click(await screen.findByRole("button", { name: /back up now/i }));

    await waitFor(() => expect(mocks.runBackupNow).toHaveBeenCalledTimes(1));
    expect(
      await screen.findByText(/backup created and verified at \/users\/test\/backups/i),
    ).toBeInTheDocument();
  });

  it("shows the latest verified receipt and terminal scheduled failure", async () => {
    mocks.backupScheduleSettings.mockResolvedValue(
      ok({ ...initialSettings, last_error: "Could not create the backup file." }),
    );
    mocks.backupHistory.mockResolvedValue(
      ok([
        {
          backup_id: "backup-1",
          created_at: "2026-09-22T12:00:00Z",
          destination: "/Users/test/Backups/recent.pcfobk",
          verified: true,
        },
      ]),
    );
    renderWithClient(<BackupScheduleCard />);

    expect(
      await screen.findByText(/last backup: .* to \/users\/test\/backups\/recent\.pcfobk \(verified\)/i),
    ).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent(/scheduled backup failed/i);
  });
});
