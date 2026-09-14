import { formatIsoDate, formatMoney } from "@/lib/format";

import type { AppliedOverride } from "./useAppliedOverrides";

/// Says that an applied scenario, not this record, is what the forecast uses
/// (personal-cfo-abhr, ADR 0055).
///
/// Applying a scenario promotes its assumption events into base rather than editing the
/// bill or income source (ADR 0055 §1 — the only option that covers all twelve assumption
/// kinds). So the stored amount and the forecast's amount legitimately differ, and a user
/// who sees them disagree with no explanation will reasonably conclude one is broken.
///
/// Descriptive only (ADR 0018): it states which figure the forecast uses and which
/// scenario put it there. It does not suggest reverting, and it does not judge the change.
export function AppliedOverrideNote({
  override,
  currency,
  onOpenScenario,
}: {
  override: AppliedOverride;
  /// The entity's own currency — the override carries minor units, not a currency.
  currency: string;
  /// Take the user to the scenario responsible, so the note is a route and not a
  /// dead end.
  onOpenScenario: (scenarioId: string) => void;
}) {
  const value =
    override.amountMinor !== null
      ? formatMoney({ minor_units: override.amountMinor, currency })
      : override.date !== null
        ? formatIsoDate(override.date)
        : null;

  return (
    <p className="mt-0.5 text-xs text-muted-foreground">
      {value === null ? (
        <>Your forecast uses a value from </>
      ) : (
        <>
          Your forecast uses{" "}
          <span className="font-medium text-foreground">{value}</span> from{" "}
        </>
      )}
      <button
        type="button"
        onClick={() => onOpenScenario(override.scenarioId)}
        className="font-medium text-primary underline-offset-2 hover:underline"
      >
        {override.scenarioName}
      </button>
      , an applied scenario.
    </p>
  );
}
