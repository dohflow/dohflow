import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { COMPACT_QUERY } from "@/lib/compact";
import { renderWithClient } from "@/test/renderWithClient";
import { UnlockedHome } from "./UnlockedHome";

vi.mock("@/accounts/useAccounts", () => ({
  useAccounts: () => ({ accounts: [{ id: "synthetic-account" }] }),
}));
vi.mock("@/money-inbox/useMoneyInbox", () => ({
  useMoneyInbox: () => ({ items: [{}, {}, {}] }),
}));
const lockVault = vi.fn();
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({ lockVault }),
}));
vi.mock("@/backup/BackupNudge", () => ({ BackupNudge: () => null }));
vi.mock("@/dashboard/DashboardView", () => ({
  DashboardView: () => <h2>Dashboard</h2>,
}));
vi.mock("@/accounts/AccountsView", () => ({
  AccountsView: () => <h2>Accounts</h2>,
}));
vi.mock("@/settings/SettingsView", () => ({ SettingsView: () => <h2>Settings</h2> }));
vi.mock("@/settings/BuildBadge", () => ({ BuildBadge: () => null }));
vi.mock("@/settings/UpdateAvailableNotice", () => ({ UpdateAvailableNotice: () => null }));
vi.mock("@/forecast-activation/CapabilityUnlockNotice", () => ({ CapabilityUnlockNotice: () => null }));
vi.mock("@/search/GlobalSearch", () => ({
  GlobalSearch: ({ onClose }: { onClose: () => void }) => (
    <div role="dialog" aria-label="Search palette">
      <button type="button" onClick={onClose}>Close search</button>
    </div>
  ),
}));

function stubMatchMedia(compact: boolean) {
  vi.stubGlobal("matchMedia", ((query: string) => ({
    matches: compact && query === COMPACT_QUERY,
    media: query,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  })) as unknown as typeof window.matchMedia);
}

afterEach(() => {
  vi.unstubAllGlobals();
  lockVault.mockReset();
});

describe("the unlocked shell on a compact screen (personal-cfo-i93n3.25 slice C)", () => {
  it("replaces the sidebar with the Sections, Search and Lock bar above the page", async () => {
    stubMatchMedia(true);
    const { container } = renderWithClient(<UnlockedHome />);
    await screen.findByRole("heading", { name: "Dashboard" });
    // No persistent sidebar, no section list in the page: the navigation is the bar and its dialog.
    expect(container.querySelector("aside")).toBeNull();
    expect(screen.queryByRole("navigation", { name: "Sections" })).toBeNull();
    const bar = screen.getByRole("group", { name: "Page actions" });
    expect(within(bar).getAllByRole("button").map((b) => b.textContent)).toEqual(["Sections", "Search", "Lock"]);
    // The shell stacks the bar over the page and the page keeps its own scroll pane.
    const shell = container.querySelector("main")!.parentElement!;
    expect(shell).toHaveClass("flex-col");
    expect(container.querySelector("main")).toHaveClass("px-4");
  });

  it("navigates through the Sections dialog and focuses the new destination's heading", async () => {
    stubMatchMedia(true);
    renderWithClient(<UnlockedHome />);
    await screen.findByRole("heading", { name: "Dashboard" });
    fireEvent.click(screen.getByRole("button", { name: "Sections" }));
    const dialog = screen.getByRole("dialog", { name: "Sections" });
    expect(within(dialog).getByRole("button", { name: "Dashboard" })).toHaveAttribute("aria-current", "page");
    // The Money Inbox row carries the shell's pending count.
    expect(within(dialog).getByRole("button", { name: "Money Inbox, 3 waiting" })).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "Accounts" }));
    const heading = await screen.findByRole("heading", { name: "Accounts" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Sections" })).toBeNull());
    await waitFor(() => expect(heading).toHaveFocus());
  });

  it("opens Search from the bar and locks from the bar", async () => {
    stubMatchMedia(true);
    renderWithClient(<UnlockedHome />);
    await screen.findByRole("heading", { name: "Dashboard" });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    expect(screen.getByRole("dialog", { name: "Search palette" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Close search" }));
    fireEvent.click(screen.getByRole("button", { name: "Lock" }));
    await waitFor(() => expect(lockVault).toHaveBeenCalledTimes(1));
  });
});

describe("the unlocked shell on a desktop window", () => {
  it("keeps the sidebar and shows no compact bar", async () => {
    stubMatchMedia(false);
    const { container } = renderWithClient(<UnlockedHome />);
    await screen.findByRole("heading", { name: "Dashboard" });
    expect(container.querySelector("aside")).not.toBeNull();
    expect(screen.getByRole("navigation", { name: "Sections" })).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "Page actions" })).toBeNull();
    const shell = container.querySelector("main")!.parentElement!;
    expect(shell).not.toHaveClass("flex-col");
    expect(container.querySelector("main")).toHaveClass("px-6");
    expect(container.querySelector("main")).not.toHaveClass("px-4");
  });
});
