import { formatIsoDate, formatMoney } from "@/lib/format";

import type { ScenarioConflict } from "./scenarioConflicts";

/// Human wording for the kinds that can conflict. Keyed to the same closed list the
/// detector uses, so a new conflicting kind cannot slip through unlabelled.
///
/// Shared rather than duplicated: the apply notice and the stack's collide panel describe
/// the SAME conflicts, and two copies of this table would drift into naming one thing two
/// ways on one screen.
export const KIND_LABEL: Record<string, string> = {
  bill_amount: "a bill amount",
  income_amount: "an income amount",
  bill_date: "a bill's date",
  income_date: "an income date",
  exclusion: "leaving something out",
};

/// How a collision reads in the STACK (personal-cfo-88o4).
///
/// Deliberately not the apply notice's sentence. That one is about a decision the user is
/// weighing ("applying this changes it to X"); this one is about a state that already
/// holds — which value the forecast is using right now, and what it beat. Reusing the
/// apply wording here would describe a pending action that is not pending.
export function describeConflict(
  conflict: ScenarioConflict,
  currency: string,
): string {
  const what = KIND_LABEL[conflict.kind] ?? "a change";
  const value = (minor: number | null) =>
    minor === null ? null : formatMoney({ minor_units: minor, currency });
  const used = value(conflict.winner.amountMinor);
  const beaten = value(conflict.loser.amountMinor);
  const window =
    conflict.winner.from === null ? "" : ` from ${formatIsoDate(conflict.winner.from)}`;
  return used !== null && beaten !== null
    ? `${what}: the forecast uses ${used}${window}, over ${beaten}.`
    : `${what}: the higher sheet's version is in effect${window}.`;
}
