import { render, screen } from "@testing-library/react";

import type { DebtPayoffPlanDto } from "@/bindings";
import { DebtPerDebtChart } from "./DebtPerDebtChart";

function plan(over: Partial<DebtPayoffPlanDto> = {}): DebtPayoffPlanDto {
  return {
    strategy: "snowball",
    debt_free_month: 3,
    total_interest_minor: 0,
    currency: "USD",
    monthly_total_owed_minor: [700_000, 500_000, 200_000, 0],
    per_debt: [
      { label: "Visa", monthly_owed_minor: [200_000, 100_000, 0, 0] },
      { label: "Auto loan", monthly_owed_minor: [500_000, 400_000, 200_000, 0] },
    ],
    ...over,
  };
}

describe("DebtPerDebtChart", () => {
  it("renders an accessible figure with a per-debt legend", () => {
    render(<DebtPerDebtChart plan={plan()} currency="USD" />);
    const fig = screen.getByRole("figure", { name: /owed balance per debt/i });
    expect(fig).toHaveAccessibleName(/Visa/i);
    expect(fig).toHaveAccessibleName(/Auto loan/i);
    // Each debt appears in the visible legend.
    expect(screen.getByText("Visa")).toBeInTheDocument();
    expect(screen.getByText("Auto loan")).toBeInTheDocument();
  });

  it("folds past the four colour slots instead of reusing a colour (hnba / ADR 0054)", () => {
    // Six debts, four slots. This used to index DEBT_COLORS with `i % length`, so the
    // 5th and 6th bands were painted identically to the 1st and 2nd — two different
    // debts rendered as one band, with the legend showing the same swatch twice.
    const six = Array.from({ length: 6 }, (_, i) => ({
      label: `Debt ${i + 1}`,
      // Descending starting balances, so the fold keeps the three largest.
      monthly_owed_minor: [600_000 - i * 100_000, 300_000 - i * 50_000, 0, 0],
    }));
    render(
      <DebtPerDebtChart
        plan={plan({ per_debt: six, monthly_total_owed_minor: [2_100_000, 1_050_000, 0, 0] })}
        currency="USD"
      />,
    );

    // The three largest keep their own band; the remaining three fold into one.
    expect(screen.getByText("Debt 1")).toBeInTheDocument();
    expect(screen.getByText("Debt 2")).toBeInTheDocument();
    expect(screen.getByText("Debt 3")).toBeInTheDocument();
    expect(screen.getByText("3 smaller debts")).toBeInTheDocument();
    expect(screen.queryByText("Debt 6")).not.toBeInTheDocument();

    // No two legend swatches share a colour — the actual invariant.
    const swatches = Array.from(
      document.querySelectorAll<HTMLElement>("figcaption span[style*='background-color']"),
    ).map((el) => el.style.backgroundColor);
    expect(swatches).toHaveLength(4);
    expect(new Set(swatches).size).toBe(swatches.length);
  });

  it("shows a hint when there is no per-debt data", () => {
    render(
      <DebtPerDebtChart
        plan={plan({ per_debt: [], monthly_total_owed_minor: [] })}
        currency="USD"
      />,
    );
    expect(screen.getByText(/break the paydown down per debt/i)).toBeInTheDocument();
  });
});
