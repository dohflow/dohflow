import { fireEvent, render, screen, within } from "@testing-library/react";
import { Search } from "lucide-react";

import { RankedBars, type RankedBar } from "./ranked-bars";

const BARS: RankedBar[] = [
  { key: "food", label: "Food", value: 84_200, drillable: true },
  { key: "transport", label: "Transport", value: 21_050 },
  { key: "misc", label: "Misc", value: 0 },
];

describe("RankedBars (personal-cfo-4d8.27.8.4)", () => {
  it("renders a labelled figure with a value beside every bar", () => {
    render(<RankedBars bars={BARS} currency="USD" label="Spend by category" />);
    expect(
      screen.getByRole("figure", { name: "Spend by category" }),
    ).toBeInTheDocument();
    // Every value is direct-labelled — no value is reachable only by hovering, which is
    // the rule a tooltip-only chart breaks (docs/design-system/dataviz.md §5).
    expect(screen.getByText("$842.00")).toBeInTheDocument();
    expect(screen.getByText("$210.50")).toBeInTheDocument();
    expect(screen.getByText("$0.00")).toBeInTheDocument();
  });

  it("paints every bar the same colour", () => {
    // Shading bars by value would re-encode length as hue and fail the palette gates by
    // construction (ADR 0054). One series, one colour — and therefore no legend.
    const { container } = render(
      <RankedBars bars={BARS} currency="USD" label="Spend" />,
    );
    const fills = Array.from(
      container.querySelectorAll<HTMLElement>("span[style*='width']"),
    ).map((el) => el.className);
    expect(fills.length).toBe(BARS.length);
    expect(new Set(fills).size).toBe(1);
  });

  it("scales bar length against the largest bar, and survives an all-zero range", () => {
    const { container } = render(
      <RankedBars bars={BARS} currency="USD" label="Spend" />,
    );
    const widths = Array.from(
      container.querySelectorAll<HTMLElement>("span[style*='width']"),
    ).map((el) => el.style.width);
    expect(widths[0]).toBe("100%");
    expect(widths[1]).toBe("25%"); // 21,050 / 84,200
    expect(widths[2]).toBe("0%");

    // An all-zero range must not divide by zero and blank the chart.
    const zeros = render(
      <RankedBars
        bars={[{ key: "a", label: "A", value: 0 }]}
        currency="USD"
        label="Spend"
      />,
    );
    expect(
      zeros.container.querySelector<HTMLElement>("span[style*='width']")?.style.width,
    ).toBe("0%");
  });

  it("makes bars real buttons whose name carries the value", () => {
    const onSelect = vi.fn();
    render(
      <RankedBars bars={BARS} currency="USD" label="Spend" onSelect={onSelect} />,
    );
    // Keyboard-reachable, and the accessible name does not depend on comparing lengths.
    const food = screen.getByRole("button", { name: /Food, \$842\.00/ });
    expect(food).toHaveAccessibleName(/open its subcategories/);
    fireEvent.click(food);
    expect(onSelect).toHaveBeenCalledWith(BARS[0]);

    // A bar with nothing below it says so by omission, not by a dead affordance.
    expect(
      screen.getByRole("button", { name: /Transport/ }),
    ).not.toHaveAccessibleName(/subcategories/);
  });

  it("renders no buttons when the caller passes no handler", () => {
    render(<RankedBars bars={BARS} currency="USD" label="Spend" />);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("shows skeletons while loading, and never the empty state", () => {
    const { container } = render(
      <RankedBars bars={[]} currency="USD" label="Spend" status="loading" empty={{ icon: Search, title: "No spending" }} />,
    );
    expect(container.querySelectorAll(".animate-pulse").length).toBeGreaterThan(0);
    // Saying "no spending" before the data lands would be a lie.
    expect(screen.queryByText("No spending")).not.toBeInTheDocument();
  });

  it("shows the empty state only when ready with no bars", () => {
    render(
      <RankedBars
        bars={[]}
        currency="USD"
        label="Spend"
        empty={{ icon: Search, title: "No spending in this range" }}
      />,
    );
    expect(screen.getByText("No spending in this range")).toBeInTheDocument();
  });

  it("replaces the bars with an error rather than an empty chart", () => {
    render(
      <RankedBars
        bars={BARS}
        currency="USD"
        label="Spend"
        status="error"
        error="Could not load spending."
        empty={{ icon: Search, title: "No spending" }}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Could not load spending.");
    expect(screen.queryByText("Food")).not.toBeInTheDocument();
    // An error is not emptiness — never claim there was no spending.
    expect(screen.queryByText("No spending")).not.toBeInTheDocument();
  });

  it("renders a footer slot", () => {
    render(
      <RankedBars
        bars={BARS}
        currency="USD"
        label="Spend"
        footer={<p>Showing June 2026</p>}
      />,
    );
    const figure = screen.getByRole("figure");
    expect(within(figure).getByText("Showing June 2026")).toBeInTheDocument();
  });
});
