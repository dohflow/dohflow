/// The debt-paydown strategy tokens the payoff engine emits (personal-cfo-od07), with their
/// shared display labels — kept in one place so the burndown chart legend and the compare table
/// never drift. Descriptive only (ADR 0018): no strategy is labelled best/recommended.
export type Strategy = "minimum_only" | "snowball" | "avalanche";

/// Canonical display order (baseline first, then the rolling strategies).
export const STRATEGY_ORDER: readonly Strategy[] = [
  "minimum_only",
  "snowball",
  "avalanche",
];

/// The label shown for each strategy across the chart legend and the compare table.
export const STRATEGY_LABEL: Record<Strategy, string> = {
  minimum_only: "Minimum only",
  snowball: "Snowball",
  avalanche: "Avalanche",
};
