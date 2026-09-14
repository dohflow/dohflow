import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { Sidebar, type Tab } from "./Sidebar";

// The Support heart leaves the app through the opener plugin; mock it at the
// boundary so the test proves the exact URL that reaches it (n76x.18).
const mocks = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));

beforeEach(() => {
  mocks.openUrl.mockReset();
  mocks.openUrl.mockResolvedValue(undefined);
});

function setup(
  over: {
    tab?: Tab;
    collapsed?: boolean;
    locking?: boolean;
    inboxCount?: number;
  } = {},
) {
  const props = {
    tab: (over.tab ?? "dashboard") as Tab,
    onSelect: vi.fn(),
    collapsed: over.collapsed ?? false,
    onToggleCollapse: vi.fn(),
    onOpenSearch: vi.fn(),
    onLock: vi.fn(),
    locking: over.locking ?? false,
    inboxCount: over.inboxCount ?? 0,
  };
  renderWithClient(<Sidebar {...props} />);
  return props;
}

describe("Sidebar", () => {
  it("renders the sections and marks the active one", () => {
    setup({ tab: "transactions" });
    expect(screen.getByRole("button", { name: "Dashboard" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Income" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Transactions" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("selects a section on click", () => {
    const { onSelect } = setup();
    fireEvent.click(screen.getByRole("button", { name: "Cash Flow" }));
    expect(onSelect).toHaveBeenCalledWith("cash-flow");
  });

  it("groups the sections into Overview / Planning / Review (ADR 0049)", () => {
    setup();
    // Each group is an announced landmark holding its own destinations.
    const overview = screen.getByRole("group", { name: "Overview" });
    const planning = screen.getByRole("group", { name: "Planning" });
    const review = screen.getByRole("group", { name: "Review" });
    expect(within(overview).getByRole("button", { name: "Dashboard" })).toBeInTheDocument();
    expect(within(overview).getByRole("button", { name: "Transactions" })).toBeInTheDocument();
    expect(within(planning).getByRole("button", { name: "Cash Flow" })).toBeInTheDocument();
    expect(within(planning).getByRole("button", { name: "Bills" })).toBeInTheDocument();
    // The Money Inbox is its own destination now, out of the Transactions hub.
    expect(within(review).getByRole("button", { name: "Money Inbox" })).toBeInTheDocument();
    expect(within(review).getByRole("button", { name: "Categories" })).toBeInTheDocument();
  });

  it("badges the Money Inbox with its pending count, in the accessible name", () => {
    setup({ inboxCount: 12 });
    expect(
      screen.getByRole("button", { name: "Money Inbox, 12 waiting" }),
    ).toBeInTheDocument();
    expect(screen.getByText("12")).toBeInTheDocument();
  });

  it("shows no badge when the inbox is empty", () => {
    setup({ inboxCount: 0 });
    expect(screen.getByRole("button", { name: "Money Inbox" })).toBeInTheDocument();
  });

  it("keeps every section keyboard/SR-reachable when collapsed", () => {
    setup({ collapsed: true });
    // Labels are hidden visually, but the accessible name (aria-label) remains.
    expect(screen.getByRole("button", { name: "Accounts" })).toBeInTheDocument();
    // Debt is its own destination (ADR 0049 §5), sitting between Accounts and
    // Transactions in Overview (§1). It was a RESERVED slot until its surface shipped
    // (§6) — a nav entry routing nowhere is a dead link.
    expect(screen.getByRole("button", { name: "Debt" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Settings" })).toBeInTheDocument();
  });

  it("shows the brand lockup when expanded (personal-cfo-4d8.28.3)", () => {
    setup({ collapsed: false });
    // The mark + wordmark lockup is an SVG image named DohFlow — not text, not a raster.
    const brand = screen.getByRole("img", { name: "DohFlow" });
    expect(brand.tagName).toBe("svg");
    expect(screen.queryByText("DohFlow")).not.toBeInTheDocument();
  });

  it("hides the brand when collapsed, keeping the (centered) toggle", () => {
    // The cramped brand+toggle pair in a 56px column is what read as broken
    // (personal-cfo-4d8.9); collapsed shows just the toggle.
    setup({ collapsed: true });
    expect(screen.queryByRole("img", { name: "DohFlow" })).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /expand sidebar/i }),
    ).toBeInTheDocument();
  });

  it("opens the search palette, with a ⌘K hint when expanded", () => {
    const { onOpenSearch } = setup();
    // The shortcut hint is visible chrome, not part of the accessible name.
    expect(screen.getByText("⌘K")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /search/i }));
    expect(onOpenSearch).toHaveBeenCalled();
  });

  it("keeps Search reachable when collapsed (icon-only)", () => {
    const { onOpenSearch } = setup({ collapsed: true });
    expect(screen.queryByText("⌘K")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /search/i }));
    expect(onOpenSearch).toHaveBeenCalled();
  });

  it("toggles collapse and locks", () => {
    const { onToggleCollapse, onLock } = setup();
    fireEvent.click(screen.getByRole("button", { name: /collapse sidebar/i }));
    expect(onToggleCollapse).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /lock vault/i }));
    expect(onLock).toHaveBeenCalled();
  });

  it("offers a quiet Support DohFlow heart that opens the sponsor page (n76x.18)", async () => {
    setup();
    const heart = screen.getByRole("button", { name: "Support DohFlow" });
    // Muted, no badge, no count — it must never read as a notification.
    expect(heart).toHaveClass("text-muted-foreground/70");
    expect(heart).not.toHaveClass("animate-pulse");
    expect(heart.textContent).toBe("");
    fireEvent.click(heart);
    await waitFor(() =>
      expect(mocks.openUrl).toHaveBeenCalledWith("https://dohflow.app/sponsor"),
    );
    expect(mocks.openUrl).toHaveBeenCalledTimes(1);
  });

  it("keeps the Support heart reachable when collapsed", async () => {
    setup({ collapsed: true });
    fireEvent.click(screen.getByRole("button", { name: "Support DohFlow" }));
    await waitFor(() =>
      expect(mocks.openUrl).toHaveBeenCalledWith("https://dohflow.app/sponsor"),
    );
  });
});
