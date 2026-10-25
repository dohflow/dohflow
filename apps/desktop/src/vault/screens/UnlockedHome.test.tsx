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

describe("UnlockedHome scroll container", () => {
  // personal-cfo-twpo.1. jsdom has no layout engine, so this guards the structural cause: the shell
  // is exactly viewport height with `main` as its only scroller, and `main` must be a POSITIONED
  // element so that `position: absolute` descendants (every `sr-only` label) are contained and
  // scrolled by it. Measured before the fix with linked accounts loaded: the document scrolled
  // 1817px past an 800px window (six sr-only labels); after: 0px, with `main` unchanged.
  it("keeps the main pane positioned so absolutely positioned labels cannot stretch the document", async () => {
    const { container } = renderWithClient(<UnlockedHome />);
    await screen.findByRole("button", { name: "Open Backups in Settings" });
    const main = container.querySelector("main");
    expect(main).not.toBeNull();
    expect(main!.classList.contains("relative")).toBe(true);
    expect(main!.classList.contains("overflow-y-auto")).toBe(true);
    // The shell itself must not scroll the document: exactly viewport height, overflow hidden.
    const shell = main!.parentElement!;
    expect(shell.classList.contains("h-screen")).toBe(true);
    expect(shell.classList.contains("overflow-hidden")).toBe(true);
  });
});

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
