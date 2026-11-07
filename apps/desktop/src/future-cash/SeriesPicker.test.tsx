import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import type { AccountSeriesDto } from "@/bindings";
import { COMPACT_QUERY } from "@/lib/compact";
import { SeriesPicker } from "./SeriesPicker";

const ACCOUNTS: AccountSeriesDto[] = [
  { account_id: "chk-1", name: "Everyday Checking", subtype: "checking", tier: "spendable", days: [] },
  { account_id: "sav-1", name: "Rainy Day", subtype: "savings", tier: "reserve", days: [] },
];


function stubMatchMedia(compact: boolean) {
  vi.stubGlobal("matchMedia", ((query: string) => ({
    matches: compact && query === COMPACT_QUERY,
    media: query,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  })) as unknown as typeof window.matchMedia);
}

function open(selection: string[], onChange = vi.fn()) {
  render(<SeriesPicker accounts={ACCOUNTS} selection={selection} onChange={onChange} />);
  fireEvent.click(screen.getByRole("button", { name: "Series" }));
  return onChange;
}

describe("SeriesPicker (personal-cfo-4d8.25.26)", () => {
  it("renders the tier hierarchy with each tier's accounts nested", () => {
    open(["net", "spendable", "reserve"]);
    expect(screen.getByRole("checkbox", { name: "Net cash" })).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Spendable" })).toBeChecked();
    expect(
      screen.getByRole("checkbox", { name: "Everyday Checking" }),
    ).not.toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Rainy Day" })).toBeInTheDocument();
  });

  it("selecting an account drops its parent tier (coherence rule — no double count)", () => {
    const onChange = open(["net", "spendable", "reserve"]);
    fireEvent.click(screen.getByRole("checkbox", { name: "Everyday Checking" }));
    const next = onChange.mock.calls[0]?.[0] as string[];
    expect(next).toContain("acct:chk-1");
    expect(next).not.toContain("spendable"); // parent tier dropped
    expect(next).toContain("reserve"); // the other tier is untouched
    expect(next).toContain("net");
  });

  it("disambiguates duplicate account names (adversarial review of 4d8.25.26)", () => {
    render(
      <SeriesPicker
        accounts={[
          { account_id: "s1", name: "Savings", subtype: "savings", tier: "reserve", days: [] },
          { account_id: "s2", name: "Savings", subtype: "money_market", tier: "reserve", days: [] },
        ]}
        selection={["reserve"]}
        onChange={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Series" }));
    // Distinct subtypes disambiguate the two same-named accounts.
    expect(
      screen.getByRole("checkbox", { name: "Savings · Savings" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("checkbox", { name: "Savings · Money market" }),
    ).toBeInTheDocument();
  });

  it("selecting a tier drops its individual accounts", () => {
    const onChange = open(["net", "reserve", "acct:chk-1"]);
    fireEvent.click(screen.getByRole("checkbox", { name: "Spendable" }));
    const next = onChange.mock.calls[0]?.[0] as string[];
    expect(next).toContain("spendable");
    expect(next).not.toContain("acct:chk-1"); // child dropped
  });
});

describe("SeriesPicker on a phone (personal-cfo-i93n3.27 slice B)", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("opens in the shared dialog named by the source copy, with the same rows and no Apply step", () => {
    stubMatchMedia(true);
    const onChange = open(["net", "spendable", "reserve"]);
    const dialog = screen.getByRole("dialog", { name: "Choose which series to plot" });
    expect(within(dialog).getByText("Plot a tier or its individual accounts.")).toBeInTheDocument();
    expect(within(dialog).getByRole("checkbox", { name: "Spendable" })).toBeChecked();
    expect(within(dialog).queryByRole("button", { name: /apply/i })).toBeNull();
    // A change applies immediately, with the same coherence rule.
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "Everyday Checking" }));
    const next = onChange.mock.calls[0]?.[0] as string[];
    expect(next).toContain("acct:chk-1");
    expect(next).not.toContain("spendable");
  });

  it("is a plain button (no aria-haspopup, which iOS exposes as a pop-up button) that reports expanded", () => {
    stubMatchMedia(true);
    render(<SeriesPicker accounts={ACCOUNTS} selection={["net"]} onChange={vi.fn()} />);
    const trigger = screen.getByRole("button", { name: "Series" });
    expect(trigger).not.toHaveAttribute("aria-haspopup");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
  });

  it("Close and Escape keep the choice and return focus to the Series button", async () => {
    stubMatchMedia(true);
    const onChange = vi.fn();
    render(<SeriesPicker accounts={ACCOUNTS} selection={["net"]} onChange={onChange} />);
    // Taken before it opens: while the dialog is open the page behind it is hidden from the role tree.
    const trigger = screen.getByRole("button", { name: "Series" });
    fireEvent.click(trigger);
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(trigger).toHaveFocus());
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(trigger);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(trigger).toHaveFocus());
    expect(onChange).not.toHaveBeenCalled();
  });

  it("keeps the desktop popover on a desktop window", () => {
    stubMatchMedia(false);
    open(["net"]);
    const popover = screen.getByRole("dialog", { name: "Choose which series to plot" });
    expect(popover.className).toContain("absolute");
    expect(screen.queryByRole("button", { name: "Close" })).toBeNull();
    expect(screen.getByRole("button", { name: "Series" })).toHaveAttribute("aria-haspopup", "true");
  });

  it("wraps a long account name on a phone instead of truncating it", () => {
    stubMatchMedia(true);
    render(
      <SeriesPicker
        accounts={[{ account_id: "x", name: "A very long joint household savings account name", subtype: "savings", tier: "reserve", days: [] }]}
        selection={["net"]}
        onChange={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Series" }));
    const label = screen.getByText("A very long joint household savings account name");
    expect(label).toHaveClass("break-words");
    expect(label).not.toHaveClass("truncate");
  });
});
