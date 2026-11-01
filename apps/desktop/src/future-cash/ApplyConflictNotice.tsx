import { AlertTriangle } from "lucide-react";

import { formatIsoDate, formatMoney } from "@/lib/format";
import { useBaseCurrency } from "@/settings/useBaseCurrency";

import { useApplyConflicts } from "./useApplyConflicts";
import type { AmountOverlap, ScenarioConflict } from "./scenarioConflicts";
import { KIND_LABEL } from "./describeConflict";

/// The labels of the amount kinds, which the notice words as an overlap rather than a replacement.
const AMOUNT_LABEL: Record<string, string> = {
  bill_amount: "Bill amount",
  income_amount: "Income amount",
};

/// The dates an amount change takes over, as the start of a sentence: "from Mar 1, 2027 to
/// Jun 30, 2027", "from Mar 1, 2027", "through Jun 30, 2027" or "for every date".
function overlapDates(overlap: AmountOverlap): string {
  const { from, to } = overlap;
  if (from !== null && to !== null) {
    return `from ${formatIsoDate(from)} to ${formatIsoDate(to)}`;
  }
  if (from !== null) return `from ${formatIsoDate(from)}`;
  if (to !== null) return `through ${formatIsoDate(to)}`;
  return "for every date";
}

function describe(conflict: ScenarioConflict, currency: string): string {
  const value = (minor: number | null) =>
    minor === null ? null : formatMoney({ minor_units: minor, currency });
  const from = value(conflict.loser.amountMinor);
  const to = value(conflict.winner.amountMinor);

  // An amount kind is ADDED beside the existing amount, which stays in force outside the dates
  // where the scenario's window overlaps it (ADR 0055 section 2): say the overlap, never "replaces".
  if (conflict.change === "overlaps" && conflict.overlap !== null) {
    const what = AMOUNT_LABEL[conflict.kind] ?? "Amount";
    const dates = overlapDates(conflict.overlap);
    if (from === null || to === null) {
      return `${what}: this overrides the amount you already have ${dates}.`;
    }
    return conflict.overlap.existingContinuesOutside
      ? `${what}: ${dates} this uses ${to} instead of ${from}; outside those dates your forecast keeps ${from}.`
      : `${what}: ${dates} this uses ${to} instead of ${from}.`;
  }

  const what = KIND_LABEL[conflict.kind] ?? "a change";
  const window =
    conflict.winner.from === null
      ? ""
      : ` from ${formatIsoDate(conflict.winner.from)}`;
  return from !== null && to !== null
    ? `${what}: your forecast already assumes ${from}${window}. Applying this changes it to ${to}.`
    : `${what} is already set here. Applying this replaces it${window}.`;
}

/// The heading: what applying does to the assumptions already there. A replacement and an
/// overlap are different things, so the count is split when both occur.
function heading(conflicts: ScenarioConflict[]): string {
  const replaced = conflicts.filter((c) => c.change === "replaces").length;
  const overlapped = conflicts.length - replaced;
  if (overlapped === 0) {
    return replaced === 1
      ? "This replaces one assumption you already have"
      : `This replaces ${replaced} assumptions you already have`;
  }
  if (replaced === 0) {
    return overlapped === 1
      ? "This overrides an amount you already have, for some dates"
      : `This overrides ${overlapped} amounts you already have, for some dates`;
  }
  return `This changes ${conflicts.length} assumptions you already have`;
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
        {heading(conflicts)}
      </p>
      <ul className="mt-2 flex list-disc flex-col gap-1 pl-5 text-muted-foreground">
        {conflicts.map((c) => (
          <li key={`${c.loser.eventId}:${c.winner.eventId}`}>
            {describe(c, baseCurrency)}
          </li>
        ))}
      </ul>
      <p className="mt-2 text-xs text-muted-foreground">
        {conflicts.some((c) => c.change === "replaces")
          ? "Applying supersedes the older assumption. Reverting this scenario restores it."
          : "Your existing amounts stay in force outside those dates. Undoing this scenario removes the new ones."}
      </p>
    </div>
  );
}
