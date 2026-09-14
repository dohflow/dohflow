import { useQuery } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// What an applied scenario changed about one entity (personal-cfo-abhr, ADR 0055).
export type AppliedOverride = {
  /// The assumption kind — `bill_amount`, `bill_date`, `income_amount`, `income_date`.
  kind: string;
  /// The scenario that was applied, so the surface can name it.
  scenarioId: string;
  scenarioName: string;
  /// The overriding amount in minor units, when the kind carries one.
  amountMinor: number | null;
  /// The overriding date (`YYYY-MM-DD`), when the kind carries one.
  date: string | null;
};

/// Pull the overriding value out of an event's `params_json`.
///
/// Lenient by design, matching how the rest of the app reads these payloads: a shape we
/// do not recognise yields nulls and the caller simply says less, rather than throwing and
/// taking a bill row down with it.
function readParams(json: string): { amountMinor: number | null; date: string | null } {
  try {
    const raw = JSON.parse(json) as Record<string, unknown>;
    const num = (v: unknown): number | null => (typeof v === "number" ? v : null);
    const str = (v: unknown): string | null => (typeof v === "string" ? v : null);
    return {
      amountMinor: num(raw.new_amount_minor) ?? num(raw.amount_minor),
      date: str(raw.new_date) ?? str(raw.date),
    };
  } catch {
    return { amountMinor: null, date: null };
  }
}

/// Base assumption overrides that exist because a scenario was APPLIED, indexed by the
/// entity they target (personal-cfo-abhr).
///
/// ADR 0055 chose to apply a scenario by promoting its events into base rather than
/// rewriting bills and income sources — the only option that covers all twelve assumption
/// kinds. The honest cost is that the forecast then uses a different figure than the
/// entity stores, and both are correct. This is what lets those surfaces say so.
///
/// Only PROMOTED events are returned. An override the user typed directly is also a base
/// event, but it needs a different explanation than "a scenario did this", and conflating
/// them would attribute the user's own edit to a scenario.
export function useAppliedOverrides() {
  const events = useQuery({
    queryKey: queryKeys.scenarioEvents(null),
    queryFn: () =>
      ipcQuery(
        commands.forecastAssumptionList(null),
        "Could not load your forecast assumptions.",
      ),
  });
  const scenarios = useQuery({
    queryKey: queryKeys.scenarios,
    queryFn: () =>
      ipcQuery(commands.scenarioList(), "Could not load your scenarios."),
  });

  const byEntity = new Map<string, AppliedOverride>();
  const names = new Map(
    (scenarios.data ?? []).map((scenario) => [scenario.id, scenario.name]),
  );
  for (const event of events.data ?? []) {
    const from = event.promoted_from_scenario_id;
    if (from === null || event.target_entity_id === null) continue;
    const { amountMinor, date } = readParams(event.params_json);
    byEntity.set(event.target_entity_id, {
      kind: event.kind,
      scenarioId: from,
      // A scenario deleted after being applied leaves its promoted events in place
      // (ADR 0055 §1 — they are base events now and stand on their own). Say something
      // true rather than rendering "undefined".
      scenarioName: names.get(from) ?? "a deleted scenario",
      amountMinor,
      date,
    });
  }

  return {
    /// What an applied scenario changed about `entityId`, or `undefined`.
    overrideFor: (entityId: string): AppliedOverride | undefined =>
      byEntity.get(entityId),
  };
}
