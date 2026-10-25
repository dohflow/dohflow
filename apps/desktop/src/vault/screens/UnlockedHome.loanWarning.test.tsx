import { fireEvent, screen } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";
import { UnlockedHome } from "./UnlockedHome";

// The loan double-count warning is shown on the Bills tab as well as Accounts and Cash Flow
// (personal-cfo-6wk.20, AC5). The tab's other content is stubbed; only the placement is under test.

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
vi.mock("@/forecast-activation/CapabilityUnlockNotice", () => ({
  CapabilityUnlockNotice: () => null,
}));
vi.mock("@/bills/BillsView", () => ({ BillsView: () => <div>Bills list</div> }));
vi.mock("@/bills/SuggestedRecurring", () => ({ SuggestedRecurring: () => null }));
vi.mock("@/bills/LinkedBillNotices", () => ({
  LinkedBillNotices: () => <div role="region" aria-label="Linked bills" />,
}));
vi.mock("@/accounts/LoanDoubleCountWarning", () => ({
  LoanDoubleCountWarning: () => <div role="region" aria-label="Loan payment counted twice" />,
}));

describe("UnlockedHome Bills tab", () => {
  it("shows the loan double-count warning above the suggestions and the bill list", async () => {
    renderWithClient(<UnlockedHome />);
    fireEvent.click(await screen.findByRole("button", { name: "Bills" }));

    expect(await screen.findByText("Bills list")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Loan payment counted twice" })).toBeInTheDocument();
  });

  it("shows the linked-bill notices on the Bills tab as well", async () => {
    renderWithClient(<UnlockedHome />);
    fireEvent.click(await screen.findByRole("button", { name: "Bills" }));
    expect(await screen.findByRole("region", { name: "Linked bills" })).toBeInTheDocument();
  });

  it("does not show it on the dashboard", async () => {
    renderWithClient(<UnlockedHome />);
    await screen.findByRole("heading", { name: "Dashboard" });
    expect(screen.queryByRole("region", { name: "Loan payment counted twice" })).toBeNull();
  });
});
