import { fireEvent, render, screen } from "@testing-library/react";

import type { AccountSeriesDto } from "@/bindings";
import { SeriesPicker } from "./SeriesPicker";

const ACCOUNTS: AccountSeriesDto[] = [
  { account_id: "chk-1", name: "Everyday Checking", subtype: "checking", tier: "spendable", days: [] },
  { account_id: "sav-1", name: "Rainy Day", subtype: "savings", tier: "reserve", days: [] },
];

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
