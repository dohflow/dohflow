import { AlertTriangle } from "lucide-react";

import { formatIsoDate, formatMoney } from "@/lib/format";
import { useBaseCurrency } from "@/settings/useBaseCurrency";

import { useApplyConflicts } from "./useApplyConflicts";
import type { ScenarioConflict } from "./scenarioConflicts";
import { KIND_LABEL } from "./describeConflict";

function describe(conflict: ScenarioConflict, currency: string): string {
  const what = KIND_LABEL[conflict.kind] ?? "a change";
  const value = (minor: number | null) =>
    minor === null ? null : formatMoney({ minor_units: minor, currency });
  const from = value(conflict.loser.amountMinor);
  const to = value(conflict.winner.amountMinor);
  const window =
    conflict.winner.from === null
      ? ""
      : ` from ${formatIsoDate(conflict.winner.from)}`;
  return from !== null && to !== null
    ? `${what}: your forecast already assumes ${from}${window}. Applying this changes it to ${to}.`
    : `${what} is already set here. Applying this replaces it${window}.`;
}

/// What applying a scenario would override in the forecast's existing assumptions
/// (personal-cfo-4d8.27.6.5, ADR 0059 §3).
///
/// Descriptive per ADR 0018: it states which value the forecast uses now, which it would
/// use after, and lets the user proceed or cancel. It does **not** block — ADR 0059 §3
/// decided conflicts compose rather than error, and the later change wins. What conflicts
/// deserve is visibility, so the user sees the override instead of discovering it in a
/// number later.
export function ApplyConflictNotice({ scenarioId }: { scenarioId: string }) {
  const { conflicts } = useApplyConflicts(scenarioId);
  const { baseCurrency } = useBaseCurrency();

  // `null` = still comparing. Rendering nothing beats rendering an all-clear the check has
  // not performed.
  if (conflicts === null || conflicts.length === 0) return null;

  return (
    <div
      role="status"
      className="mt-3 rounded-md border border-warning/40 bg-warning/10 p-3 text-sm"
    >
      <p className="flex items-center gap-1.5 font-medium">
        <AlertTriangle className="size-4 shrink-0" aria-hidden />
        {conflicts.length === 1
          ? "This replaces one assumption you already have"
          : `This replaces ${conflicts.length} assumptions you already have`}
      </p>
      <ul className="mt-2 flex list-disc flex-col gap-1 pl-5 text-muted-foreground">
        {conflicts.map((c) => (
          <li key={`${c.loser.eventId}:${c.winner.eventId}`}>
            {describe(c, baseCurrency)}
          </li>
        ))}
      </ul>
      <p className="mt-2 text-xs text-muted-foreground">
        Applying supersedes the older assumption. Reverting this scenario restores it.
      </p>
    </div>
  );
}
