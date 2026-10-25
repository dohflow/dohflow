import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, within } from "@testing-library/react";

import { useNotices } from "@/components/ui/notice-context";
import { ThemeProvider } from "@/theme/ThemeProvider";
import { UnlockedHome } from "./UnlockedHome";

// The shell owns the one notifications region (docs/agent/FRONTEND.md section 6.2, personal-cfo-egn67):
// first inside `main`, and a pending Undo survives leaving the screen. Rendered WITHOUT the test
// helper's own provider, so it proves the shell supplies it.

vi.mock("@/accounts/useAccounts", () => ({
  useAccounts: () => ({ accounts: [{ id: "synthetic-account" }] }),
}));
vi.mock("@/money-inbox/useMoneyInbox", () => ({
  useMoneyInbox: () => ({ items: [] }),
}));
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({ lockVault: vi.fn() }),
}));
vi.mock("@/backup/BackupNudge", () => ({ BackupNudge: () => null }));
vi.mock("@/dashboard/DashboardView", () => ({ DashboardView: () => <h1>Dashboard</h1> }));
vi.mock("@/settings/BuildBadge", () => ({ BuildBadge: () => null }));
vi.mock("@/settings/UpdateAvailableNotice", () => ({ UpdateAvailableNotice: () => null }));
// The notices that sit beside the region render a marker, so the ORDER is under test.
vi.mock("@/forecast-activation/CapabilityUnlockNotice", () => ({
  CapabilityUnlockNotice: () => <div>capability notice</div>,
}));
vi.mock("@/bills/SuggestedRecurring", () => ({ SuggestedRecurring: () => null }));
vi.mock("@/accounts/LoanDoubleCountWarning", () => ({ LoanDoubleCountWarning: () => null }));
vi.mock("@/bills/BillsView", () => ({
  BillsView: function StubBills() {
    const { notify } = useNotices();
    return (
      <button onClick={() => notify({ message: "Removed Rent.", undo: async () => true })}>
        remove rent
      </button>
    );
  },
}));

function renderHome() {
  return render(
    <ThemeProvider>
      <QueryClientProvider client={new QueryClient()}>
        <UnlockedHome />
      </QueryClientProvider>
    </ThemeProvider>,
  );
}

describe("UnlockedHome notifications", () => {
  it("renders the Notifications region as the first thing inside main", async () => {
    renderHome();
    fireEvent.click(await screen.findByRole("button", { name: "Bills" }));
    fireEvent.click(await screen.findByRole("button", { name: "remove rent" }));

    const region = await screen.findByRole("region", { name: "Notifications" });
    expect(region.parentElement?.tagName).toBe("MAIN");
    expect(region.parentElement?.firstElementChild).toBe(region);
    // Before the other notices and the page content in the tab order.
    for (const later of [screen.getByText("capability notice"), screen.getByRole("button", { name: "remove rent" })]) {
      expect(region.compareDocumentPosition(later) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    }
    expect(within(region).getByText("Removed Rent.")).toBeInTheDocument();
  });

  it("keeps a pending Undo when the user moves to another tab", async () => {
    renderHome();
    fireEvent.click(await screen.findByRole("button", { name: "Bills" }));
    fireEvent.click(await screen.findByRole("button", { name: "remove rent" }));
    await screen.findByRole("region", { name: "Notifications" });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Dashboard" }));
    });
    await screen.findByRole("heading", { name: "Dashboard" });
    const region = screen.getByRole("region", { name: "Notifications" });
    expect(within(region).getByRole("button", { name: "Undo" })).toBeInTheDocument();
  });
});
