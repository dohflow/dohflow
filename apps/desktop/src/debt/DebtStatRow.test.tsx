import { render, screen } from "@testing-library/react";

import type { AccountViewDto, DebtTermsDto } from "@/bindings";

import { DebtStatRow } from "./DebtStatRow";

const card = {
  id: "card-1",
  name: "Visa",
  cashflow_role: "credit_facility",
  subtype: "credit_card",
  active: true,
  balance: { minor_units: -124_000, currency: "USD" },
} as AccountViewDto;
const terms = {
  account_id: "card-1",
  apr_bps: 2199,
  statement_close_day: 5,
  payment_due_day: 25,
  grace_period_days: 21,
  credit_limit_minor: 800_000,
  repayment_philosophy: "pay_minimum",
  fixed_amount_minor: null,
  min_payment_percent_bps: 200,
  min_payment_floor_minor: 3500,
  paying_source_account_id: null,
  original_principal_minor: null,
} as DebtTermsDto;

// personal-cfo-i93n3.28 slice 7: the stat tiles reflow at the largest text size on a phone (compact-only; desktop unchanged).
describe("DebtStatRow on a phone", () => {
  it("fits as many tiles as the text size allows, and keeps each figure on one line by capping its size to the window", () => {
    render(<DebtStatRow debts={[{ account: card, terms, owedMinor: 124_000 }]} currency="USD" />);
    const label = screen.getByText("Total owed");
    const tile = label.parentElement!;
    expect(tile).toHaveClass("min-w-0");
    expect(tile.parentElement!.className).toContain("compact:grid-cols-[repeat(auto-fit,minmax(min(100%,9rem),1fr))]");
    // The desktop grid is still its own columns.
    expect(tile.parentElement!.className).toContain("lg:grid-cols-5");
    const figure = screen.getByText("$1,240.00");
    expect(figure).toHaveClass("compact:text-[min(1.125rem,7.5vw)]", "compact:[overflow-wrap:anywhere]");
    expect(label).toHaveClass("compact:[overflow-wrap:anywhere]");
  });
});
