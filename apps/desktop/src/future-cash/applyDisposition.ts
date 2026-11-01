/// How applying a scenario treats each assumption kind (ADR 0055 section 2 as amended by
/// personal-cfo-5ie.13). One list, mirroring the ADR table row by row; `applyDisposition.test.ts`
/// pins it against that table, which `collision_kinds.rs` pins against the kernel's
/// `AssumptionKind::supersedes_on_promotion`, so the three cannot drift apart.
///
/// - `collides`: the forecast reads one event per kind and target, so an applied event REPLACES the
///   existing one.
/// - `adds`: the forecast reads every active instance, so an applied event is ADDED beside the
///   existing ones. For the two amount kinds the scenario's window wins only where the windows
///   overlap, and the existing amount stays in force outside that overlap.
export type ApplyDisposition = "collides" | "adds";

export const APPLY_DISPOSITION: Readonly<Record<string, ApplyDisposition>> = {
  bill_amount: "adds",
  income_amount: "adds",
  bill_date: "collides",
  income_date: "collides",
  exclusion: "collides",
  minimum_cash_floor: "collides",
  inflation_rate: "collides",
  card_payment_behavior: "collides",
  scenario_toggle: "collides",
  one_time_event: "adds",
  recurring_debt_payment: "adds",
  variable_spend_override: "adds",
};

/// Whether applying an event of `kind` replaces the existing event of that kind and target.
/// An unknown kind is treated as replacing, the cautious reading (it names a change).
export function replacesOnApply(kind: string): boolean {
  return APPLY_DISPOSITION[kind] !== "adds";
}
