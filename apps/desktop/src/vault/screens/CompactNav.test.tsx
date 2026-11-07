import { useState } from "react";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import { COMPACT_QUERY } from "@/lib/compact";
import { LARGE_TEXT_ROOT_PX } from "@/lib/largeText";
import { CompactNavBar, focusDestinationHeading } from "./CompactNav";
import { GROUPS, NAV, type Tab } from "./Sidebar";

vi.mock("@/settings/BuildBadge", () => ({ BuildBadge: () => <p>build badge</p> }));

/// The compact navigation inside a page that has a heading in `main`, the way the shell hosts it.
function Harness({
  onSearch = () => undefined,
  onLock = () => undefined,
  inbox = 0,
}: {
  onSearch?: () => void;
  onLock?: () => void;
  inbox?: number;
}) {
  const [tab, setTab] = useState<Tab>("cash-flow");
  return (
    <div>
      <CompactNavBar
        tab={tab}
        onSelect={setTab}
        onOpenSearch={onSearch}
        onLock={onLock}
        locking={false}
        inboxCount={inbox}
      />
      <main>
        <h2 data-testid="heading">{`Destination: ${tab}`}</h2>
        <button type="button">A control in the page</button>
      </main>
    </div>
  );
}

function setRootFontSize(px: string | null) {
  if (px === null) document.documentElement.style.removeProperty("font-size");
  else document.documentElement.style.fontSize = px;
}

afterEach(() => setRootFontSize(null));

function openSections() {
  const opener = screen.getByRole("button", { name: "Sections" });
  fireEvent.click(opener);
  return opener;
}

describe("the compact navigation (personal-cfo-i93n3.25 slice C)", () => {
  it("shows Sections, Search and Lock as word buttons above the page at an ordinary text size", () => {
    render(<Harness />);
    const bar = screen.getByRole("group", { name: "Page actions" });
    expect(within(bar).getAllByRole("button").map((b) => b.textContent)).toEqual([
      "Sections",
      "Search",
      "Lock",
    ]);
    // The bar comes before the page in the document, so it is first in the reading order.
    expect(bar.compareDocumentPosition(screen.getByTestId("heading"))).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    );
  });

  it("uses icon buttons named Sections, Search and Lock at an accessibility text size", () => {
    setRootFontSize(`${LARGE_TEXT_ROOT_PX}px`);
    render(<Harness />);
    const bar = screen.getByRole("group", { name: "Page actions" });
    const buttons = within(bar).getAllByRole("button");
    expect(buttons.map((b) => b.getAttribute("aria-label"))).toEqual(["Sections", "Search", "Lock"]);
    // No visible word: the icon carries it, and the accessible name is the same word.
    for (const button of buttons) {
      expect(button.textContent).toBe("");
      expect(button.querySelector("svg")).not.toBeNull();
      // At least 48 points on both sides (a fixed floor, not spacing).
      expect(button.className).toContain("min-h-12");
      expect(button.className).toContain("min-w-12");
    }
  });

  it("switches at the first accessibility size: 28px is the root size of Accessibility Medium", () => {
    // iOS root sizes: 14 to 23px are the ordinary sizes (Extra Small to Extra Extra Extra Large), 28px is
    // the first accessibility size. The icon bar is for those, not for ordinary large text.
    expect(LARGE_TEXT_ROOT_PX).toBe(28);
  });

  it("stays on word buttons just below the accessibility threshold", () => {
    setRootFontSize(`${LARGE_TEXT_ROOT_PX - 1}px`);
    render(<Harness />);
    expect(screen.getByRole("button", { name: "Sections" }).textContent).toBe("Sections");
  });

  it("is a plain button that reports whether its dialog is open, not a pop-up (menu) button", () => {
    render(<Harness />);
    const sections = screen.getByRole("button", { name: "Sections" });
    // `aria-haspopup` would make iOS expose a pop-up button, not a button.
    expect(sections).not.toHaveAttribute("aria-haspopup");
    expect(sections).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(sections);
    expect(sections).toHaveAttribute("aria-expanded", "true");
  });

  it("calls Search and Lock", () => {
    const onSearch = vi.fn();
    const onLock = vi.fn();
    render(<Harness onSearch={onSearch} onLock={onLock} />);
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    fireEvent.click(screen.getByRole("button", { name: "Lock" }));
    expect(onSearch).toHaveBeenCalledTimes(1);
    expect(onLock).toHaveBeenCalledTimes(1);
  });

  it("opens a full-screen Sections dialog that lists the sidebar's groups and destinations in order", () => {
    render(<Harness />);
    openSections();
    const dialog = screen.getByRole("dialog", { name: "Sections" });
    const groups = within(dialog).getAllByRole("region");
    expect(groups.map((g) => within(g).getByText(/^(Overview|Planning|Review)$/).textContent)).toEqual(
      GROUPS.map((g) => g.label),
    );
    // Every destination, in the sidebar's order (Settings last, after the groups).
    const labels = within(dialog)
      .getAllByRole("button")
      .map((b) => b.getAttribute("aria-label") ?? b.textContent);
    expect(labels.filter((l) => l !== "Close")).toEqual([
      ...GROUPS.flatMap((g) => NAV.filter((n) => n.group === g.key).map((n) => n.label)),
      ...NAV.filter((n) => n.group === "utility").map((n) => n.label),
    ]);
    // It is presented full screen on a compact screen (the sheet's bottom anchoring is overridden).
    for (const name of ["compact:top-0", "compact:h-dvh", "compact:max-h-dvh", "compact:rounded-none", "compact:pt-(--safe-area-top)"]) {
      expect(dialog).toHaveClass(name);
    }
  });

  it("marks exactly one destination as the current page, with a check mark", () => {
    render(<Harness />);
    openSections();
    const dialog = screen.getByRole("dialog", { name: "Sections" });
    const current = within(dialog)
      .getAllByRole("button")
      .filter((b) => b.getAttribute("aria-current") === "page");
    expect(current).toHaveLength(1);
    expect(current[0]!).toHaveAccessibleName("Cash Flow");
    expect(current[0]!.querySelectorAll("svg").length).toBe(2); // its icon and the check mark
    const another = within(dialog).getByRole("button", { name: "Bills" });
    expect(another.querySelectorAll("svg").length).toBe(1);
  });

  it("speaks the Money Inbox count with its name", () => {
    render(<Harness inbox={12} />);
    openSections();
    expect(screen.getByRole("button", { name: "Money Inbox, 12 waiting" })).toBeInTheDocument();
  });

  it("starts focus at Close and makes the page behind it inert to assistive technology", () => {
    render(<Harness />);
    openSections();
    expect(screen.getByRole("button", { name: "Close" })).toHaveFocus();
    // Everything outside the dialog is hidden from assistive technology (Radix `aria-hidden`s it) and
    // out of the role tree.
    expect(screen.queryByRole("button", { name: "A control in the page" })).toBeNull();
    expect(screen.getByTestId("heading").closest('[aria-hidden="true"]')).not.toBeNull();
  });

  it("dismisses on Close without navigating and restores focus to the Sections button", async () => {
    render(<Harness />);
    const opener = openSections();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByTestId("heading")).toHaveTextContent("Destination: cash-flow");
    expect(opener).toHaveFocus();
  });

  it("dismisses on Escape without navigating and restores focus to the Sections button", async () => {
    render(<Harness />);
    const opener = openSections();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByTestId("heading")).toHaveTextContent("Destination: cash-flow");
    expect(opener).toHaveFocus();
  });

  it("choosing a destination dismisses the dialog, changes the destination and focuses its heading", async () => {
    render(<Harness />);
    openSections();
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Bills" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const heading = screen.getByTestId("heading");
    expect(heading).toHaveTextContent("Destination: bills");
    await waitFor(() => expect(heading).toHaveFocus());
  });

  it("keeps Tab inside the dialog while it is open", () => {
    render(<Harness />);
    openSections();
    const dialog = screen.getByRole("dialog", { name: "Sections" });
    const last = within(dialog).getAllByRole("button").at(-1)!;
    last.focus();
    fireEvent.keyDown(last, { key: "Tab" });
    // Radix keeps the focus within the dialog; the page's own control is never focused.
    expect(document.activeElement && dialog.contains(document.activeElement)).toBe(true);
  });
});

describe("focusDestinationHeading", () => {
  it("makes the first heading in main focusable and focuses it", () => {
    render(
      <main>
        <h2>Accounts</h2>
      </main>,
    );
    focusDestinationHeading();
    const heading = screen.getByRole("heading", { name: "Accounts" });
    expect(heading).toHaveFocus();
    expect(heading).toHaveAttribute("tabindex", "-1");
  });

  it("waits a few frames for a heading that mounts late", async () => {
    vi.useFakeTimers({ toFake: ["requestAnimationFrame"] });
    render(<main />);
    focusDestinationHeading();
    const heading = document.createElement("h1");
    heading.textContent = "Late";
    document.querySelector("main")!.append(heading);
    await act(async () => {
      vi.advanceTimersByTime(50);
    });
    expect(heading).toHaveFocus();
    vi.useRealTimers();
  });

  it("the compact query used by the shell is the one signal", () => {
    expect(COMPACT_QUERY).toContain("pointer: coarse");
  });
});
