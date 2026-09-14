import { render, screen } from "@testing-library/react";

import type { DebtPayoffPlanDto } from "@/bindings";
import { DebtBurndownChart } from "./DebtBurndownChart";

function plan(
  strategy: string,
  debt_free_month: number | null,
  traj: number[],
): DebtPayoffPlanDto {
  return {
    strategy,
    debt_free_month,
    total_interest_minor: 0,
    currency: "USD",
    monthly_total_owed_minor: traj,
    per_debt: [],
  };
}

describe("DebtBurndownChart", () => {
  it("renders an accessible figure with a per-strategy legend", () => {
    const plans = [
      plan("minimum_only", null, [700_000, 690_000, 680_000, 670_000]),
      plan("snowball", 3, [700_000, 500_000, 200_000, 0]),
      plan("avalanche", 3, [700_000, 480_000, 180_000, 0]),
    ];
    render(<DebtBurndownChart plans={plans} currency="USD" />);

    const fig = screen.getByRole("figure", { name: /debt owed over time/i });
    expect(fig).toHaveAccessibleName(/minimum only/i);
    expect(fig).toHaveAccessibleName(/snowball/i);
    expect(fig).toHaveAccessibleName(/avalanche/i);
    // The visible legend lists each plotted strategy.
    expect(screen.getByText("Minimum only")).toBeInTheDocument();
    expect(screen.getByText("Snowball")).toBeInTheDocument();
    expect(screen.getByText("Avalanche")).toBeInTheDocument();
  });

  it("shows a hint when the trajectory is too short to chart", () => {
    render(<DebtBurndownChart plans={[plan("snowball", 0, [])]} currency="USD" />);
    expect(screen.getByText(/not enough data to chart/i)).toBeInTheDocument();
  });
});
