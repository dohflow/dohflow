import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";
import { UnlockedHome } from "./UnlockedHome";

vi.mock("@/accounts/useAccounts", () => ({
  useAccounts: () => ({ accounts: [{ id: "synthetic-account" }] }),
}));
vi.mock("@/money-inbox/useMoneyInbox", () => ({
  useMoneyInbox: () => ({ items: [] }),
}));
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({ lockVault: vi.fn() }),
}));
vi.mock("@/backup/BackupNudge", () => ({
  BackupNudge: ({ onOpenBackups }: { onOpenBackups: () => void }) => (
    <button type="button" onClick={onOpenBackups}>Open Backups in Settings</button>
  ),
}));
vi.mock("@/dashboard/DashboardView", () => ({
  DashboardView: () => <h1>Dashboard</h1>,
}));
vi.mock("@/settings/SettingsView", () => ({
  SettingsView: () => <div id="settings-backups" tabIndex={-1}>Backups card</div>,
}));
vi.mock("@/settings/BuildBadge", () => ({ BuildBadge: () => null }));
vi.mock("@/settings/UpdateAvailableNotice", () => ({ UpdateAvailableNotice: () => null }));
vi.mock("@/forecast-activation/CapabilityUnlockNotice", () => ({ CapabilityUnlockNotice: () => null }));

describe("UnlockedHome backup navigation", () => {
  it("opens Settings and focuses Backups from the reminder without a Backup tab", async () => {
    renderWithClient(<UnlockedHome />);
    const reminder = await screen.findByRole("button", { name: "Open Backups in Settings" });
    reminder.focus();
    fireEvent.click(reminder);

    const backups = await screen.findByText("Backups card");
    await waitFor(() => expect(backups).toHaveFocus());
    expect(screen.getByRole("button", { name: "Settings" })).toHaveAttribute("aria-current", "page");
    expect(screen.queryByRole("button", { name: /^Backup$/ })).not.toBeInTheDocument();
  });
});
