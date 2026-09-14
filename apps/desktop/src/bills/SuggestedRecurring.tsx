import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, Loader2, Sparkles } from "lucide-react";

import type { RecurringCandidateDto } from "@/bindings";
import { commands } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { formatIsoDate, formatMoney } from "@/lib/format";
import { frequencyLabel } from "@/lib/frequency";
import { useCategories } from "@/categories/useCategories";
import { categoryLabels } from "@/categories/labels";
import { queryKeys } from "@/lib/query";

import { mintIdempotencyKey } from "@/lib/idempotency";

import { RecurringCandidateForm } from "./BillsView";
import {
  useDismissRecurringSuggestion,
  useRecurringCandidates,
} from "./useRecurringCandidates";

/// The retro-attach result shown right after a candidate is promoted (ADR 0047 §1,
/// personal-cfo-4d8.25.8): how many historical transactions matched the new bill's
/// schedule, with an EXPLICIT mark-reviewed affordance (ADR 0032 — review state never
/// changes silently).
type AttachedSummary = {
  billName: string;
  linkedTransactionIds: string[];
  marked: boolean;
};

/// Plain-language cadence labels (the backend token → display).
const CADENCE_LABEL: Record<string, string> = {
  weekly: "weekly",
  biweekly: "every 2 weeks",
  monthly: "monthly",
  quarterly: "quarterly",
  annual: "yearly",
};

/// The cadence with the typical day when one is known ("monthly · roughly the 19th",
/// personal-cfo-4d8.25.11).
function cadenceWithDay(candidate: RecurringCandidateDto): string {
  const cadence = CADENCE_LABEL[candidate.frequency] ?? frequencyLabel(candidate.frequency);
  const day = candidate.typical_day_of_month;
  return day ? `${cadence} · roughly the ${ordinal(day)}` : cadence;
}

function ordinal(n: number): string {
  const rem10 = n % 10;
  const rem100 = n % 100;
  if (rem10 === 1 && rem100 !== 11) return `${n}st`;
  if (rem10 === 2 && rem100 !== 12) return `${n}nd`;
  if (rem10 === 3 && rem100 !== 13) return `${n}rd`;
  return `${n}th`;
}

/// The pay-from provenance: "always on Venture X" when every observation posted to one
/// account; the account list otherwise (ADR 0047 §4, personal-cfo-4d8.25.11).
function accountProvenance(candidate: RecurringCandidateDto): string | null {
  const names = candidate.source_account_names;
  if (names.length === 0) return null;
  if (names.length === 1) return `always on ${names[0]}`;
  return `across ${names.join(", ")}`;
}

/// A qualitative confidence band from the detector's basis points — descriptive, not a score
/// the user is meant to act on numerically.
function confidenceLabel(bps: number): string {
  if (bps >= 8000) return "high confidence";
  if (bps >= 5000) return "medium confidence";
  return "low confidence";
}

/// The observed charge amount: a single value for a fixed bill, or a min–max range for a
/// variable one (personal-cfo-4d8.24.5).
function amountLabel(candidate: RecurringCandidateDto): string {
  const { currency, amount_min_minor, amount_max_minor } = candidate;
  if (amount_min_minor === amount_max_minor) {
    return formatMoney({ minor_units: amount_min_minor, currency });
  }
  return `${formatMoney({ minor_units: amount_min_minor, currency })}–${formatMoney({
    minor_units: amount_max_minor,
    currency,
  })}`;
}

/// The "Suggested recurring" review surface (personal-cfo-98ql): merchants detected as recurring
/// in the realized history that are not yet tracked as bills. Each is a suggestion the user
/// confirms — clicking "Add as recurring" opens a pre-filled bill form (ADR 0018; nothing is
/// auto-created). Renders nothing when there is nothing to suggest.
export function SuggestedRecurring() {
  const { candidates } = useRecurringCandidates();
  const { dismiss } = useDismissRecurringSuggestion();
  const { categories } = useCategories();
  const queryClient = useQueryClient();
  // Leaf-only name for the dominant-category hint (the row gives merchant context).
  const { leafLabel } = categoryLabels(categories ?? []);
  const [promoting, setPromoting] = useState<string | null>(null);
  const [dismissing, setDismissing] = useState<string | null>(null);
  const [attached, setAttached] = useState<AttachedSummary | null>(null);
  const [marking, setMarking] = useState(false);

  /// After a promotion, fetch the new bill's retro-attached history (ADR 0047 §1) and
  /// keep the summary panel up even though the candidate row just disappeared.
  async function showAttached(billName: string, eventId: string | null) {
    setPromoting(null);
    if (!eventId) return;
    const result = await commands.recurringBillHistory(eventId);
    if (result.status !== "ok") return;
    const linked = result.data
      .map((occurrence) => occurrence.linked_transaction_id)
      .filter((id): id is string => id !== null);
    if (linked.length > 0) {
      setAttached({ billName, linkedTransactionIds: linked, marked: false });
    }
  }

  async function markAttachedReviewed(summary: AttachedSummary) {
    setMarking(true);
    const results = await Promise.all(
      summary.linkedTransactionIds.map((id) =>
        commands.markReviewed(id, true, mintIdempotencyKey()),
      ),
    );
    setMarking(false);
    if (results.every((r) => r.status === "ok")) {
      // Guarded functional update: if the user dismissed the panel (or a newer
      // promotion replaced it) while the fan-out was in flight, stay dismissed —
      // a completed mark-reviewed must not resurrect a closed panel.
      setAttached((prev) =>
        prev && prev.linkedTransactionIds === summary.linkedTransactionIds
          ? { ...summary, marked: true }
          : prev,
      );
      void queryClient.invalidateQueries({ queryKey: queryKeys.moneyInbox });
      void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
    }
  }

  if ((!candidates || candidates.length === 0) && !attached) return null;

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="flex items-center gap-2 text-base">
          <Sparkles className="size-4 text-primary" aria-hidden />
          Suggested recurring
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        {attached && (
          <div
            role="status"
            className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-gain/30 bg-gain/10 px-3 py-2 text-sm"
          >
            <span className="flex items-center gap-2">
              <CheckCircle2 className="size-4 text-gain" aria-hidden />
              {attached.marked
                ? `Marked ${attached.linkedTransactionIds.length} past transactions reviewed for ${attached.billName}.`
                : `Matched ${attached.linkedTransactionIds.length} past transactions for ${attached.billName}.`}
            </span>
            <span className="flex items-center gap-2">
              {!attached.marked && (
                <Button
                  size="sm"
                  variant="outline"
                  disabled={marking}
                  onClick={() => void markAttachedReviewed(attached)}
                >
                  {marking && <Loader2 className="size-3.5 animate-spin" aria-hidden />}
                  Mark {attached.linkedTransactionIds.length} reviewed
                </Button>
              )}
              <Button size="sm" variant="ghost" onClick={() => setAttached(null)}>
                Done
              </Button>
            </span>
          </div>
        )}
        {candidates && candidates.length > 0 && (
          <p className="text-sm text-muted-foreground">
            Merchants that show up regularly in your history. Add one as a recurring bill to
            include it in your forecast.
          </p>
        )}
        <ul className="flex flex-col gap-2">
          {(candidates ?? []).map((candidate) => {
            const id = `${candidate.merchant_key}:${candidate.currency}`;
            return (
              <li key={id} className="rounded-md border px-3 py-2">
                {promoting === id ? (
                  <RecurringCandidateForm
                    candidate={candidate}
                    onCancel={() => setPromoting(null)}
                    onCreated={(eventId) => void showAttached(candidate.display, eventId)}
                  />
                ) : (
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div className="min-w-0">
                      <div className="truncate font-medium">{candidate.display}</div>
                      <div className="text-xs text-muted-foreground">
                        {amountLabel(candidate)} · {cadenceWithDay(candidate)} · seen{" "}
                        {candidate.occurrence_count} times ·{" "}
                        {confidenceLabel(candidate.confidence_bps)}
                        {accountProvenance(candidate) && (
                          <> · {accountProvenance(candidate)}</>
                        )}
                      </div>
                      {/* Source metadata surfaced before promotion (personal-cfo-4d8.24.5). */}
                      <div className="text-xs text-muted-foreground">
                        Last seen {formatIsoDate(candidate.last_seen)}
                        {candidate.dominant_category_id &&
                          leafLabel(candidate.dominant_category_id) && (
                            <> · {leafLabel(candidate.dominant_category_id)}</>
                          )}
                      </div>
                      {/* The detection proof: the observed charges behind the suggestion
                          (personal-cfo-4d8.25.11) — expandable, dep-free details/summary. */}
                      {candidate.observations.length > 0 && (
                        <details className="text-xs text-muted-foreground">
                          <summary className="cursor-pointer select-none">
                            Why? {candidate.observations.length} observed charges
                          </summary>
                          <ul className="mt-1 flex flex-col gap-0.5 pl-4">
                            {candidate.observations.map((observation, index) => (
                              <li
                                // Two identical charges (same day/amount/account) are
                                // legitimate rows; the index keeps keys unique in this
                                // stable, sorted, capped list.
                                key={`${observation.date}:${index}`}
                                className="tabular-nums"
                              >
                                {formatIsoDate(observation.date)} ·{" "}
                                {formatMoney({
                                  minor_units: observation.amount_minor,
                                  currency: candidate.currency,
                                })}{" "}
                                · {observation.account_name}
                              </li>
                            ))}
                          </ul>
                        </details>
                      )}
                    </div>
                    <div className="flex items-center gap-2">
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={dismissing === id}
                        aria-label={`Dismiss ${candidate.display}`}
                        onClick={async () => {
                          setDismissing(id);
                          // Not a bill — record the dismissal so it stops being suggested
                          // until the amount/cadence materially changes (ADR 0046).
                          await dismiss({
                            merchant_key: candidate.merchant_key,
                            currency: candidate.currency,
                            amount_minor: candidate.amount_minor,
                            frequency: candidate.frequency,
                            reason: null,
                            idempotency_key: mintIdempotencyKey(),
                          });
                          setDismissing(null);
                        }}
                      >
                        Dismiss
                      </Button>
                      <Button variant="outline" size="sm" onClick={() => setPromoting(id)}>
                        Add as recurring
                      </Button>
                    </div>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      </CardContent>
    </Card>
  );
}
