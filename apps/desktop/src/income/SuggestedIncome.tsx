// "Detected income" (personal-cfo-gmnk, onboarding epic 5fp6): recurring
// deposits in the realized history that are not yet modeled as income sources —
// each an approve / edit / deny prefill, never auto-created (ADR 0018). Mirrors
// SuggestedRecurring for bills; the form it opens is the real IncomeForm.

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Loader2, Sparkles } from "lucide-react";

import type { RecurringCandidateDto } from "@/bindings";
import { useAccounts } from "@/accounts/useAccounts";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { formatIsoDate, formatMoney } from "@/lib/format";
import { frequencyLabel } from "@/lib/frequency";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { describeIpcError } from "@/vault/useVault";

import { IncomeForm, minorUnitsToInput } from "./IncomeView";
import { useIncome } from "./useIncome";
import { INCOME_CANDIDATES_KEY, useDismissIncomeCandidate, useIncomeCandidates } from "./useIncomeCandidates";

const CADENCE_LABEL: Record<string, string> = {
  weekly: "weekly",
  biweekly: "every 2 weeks",
  monthly: "monthly",
  quarterly: "quarterly",
  annual: "yearly",
};

/// The provider's shouting ("ACME PAYROLL") as a readable default name.
function titleCase(raw: string): string {
  return raw
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

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

export function SuggestedIncome() {
  const { candidates } = useIncomeCandidates();
  const { dismiss } = useDismissIncomeCandidate();
  const { accounts } = useAccounts();
  const { addIncomeSource } = useIncome();
  const queryClient = useQueryClient();
  const [adding, setAdding] = useState<string | null>(null);
  const [dismissing, setDismissing] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  if (!candidates || candidates.length === 0) return null;

  async function deny(candidate: RecurringCandidateDto) {
    const id = `${candidate.merchant_key}:${candidate.currency}`;
    setDismissing(id);
    setError(null);
    const failure = await dismiss({
      merchant_key: candidate.merchant_key,
      currency: candidate.currency,
      amount_minor: candidate.amount_minor,
      frequency: candidate.frequency,
      reason: null,
      idempotency_key: mintIdempotencyKey(),
    });
    setDismissing(null);
    if (failure) setError(describeIpcError(failure));
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <Sparkles className="size-4 text-primary" aria-hidden />
          Detected income
        </CardTitle>
        <p className="text-sm text-muted-foreground">
          Deposits that arrive on a schedule in your recorded history and are not
          yet listed as income. Add the ones that are yours; dismiss the rest.
          Transfers from your own accounts can show up here too — dismiss those.
          Paid on the 15th and the last day of the month? Pick Semimonthly when
          adding; every-two-weeks is the closest the detector can tell apart.
        </p>
      </CardHeader>
      <CardContent>
        <ul className="flex flex-col gap-2">
          {candidates.map((candidate) => {
            const id = `${candidate.merchant_key}:${candidate.currency}`;
            const cadence =
              CADENCE_LABEL[candidate.frequency] ?? frequencyLabel(candidate.frequency);
            const account =
              candidate.source_account_names.length === 1
                ? `into ${candidate.source_account_names[0]}`
                : candidate.source_account_names.length > 1
                  ? `across ${candidate.source_account_names.join(", ")}`
                  : null;
            return (
              <li key={id} className="rounded-md border px-3 py-2">
                {adding === id ? (
                  <IncomeForm
                    accounts={accounts ?? []}
                    submitLabel="Add income"
                    fallbackCurrency={candidate.currency}
                    defaultValues={{
                      name: titleCase(candidate.display),
                      amount: minorUnitsToInput(candidate.amount_minor),
                      frequency: candidate.frequency,
                      anchor: candidate.last_seen,
                      deposit_account_id: candidate.source_account_id ?? "",
                    }}
                    onCancel={() => setAdding(null)}
                    onSubmit={async (draft) => {
                      const failure = await addIncomeSource({
                        ...draft,
                        idempotency_key: mintIdempotencyKey(),
                      });
                      if (failure) return describeIpcError(failure);
                      setAdding(null);
                      // The new source consumes its candidate; refresh both.
                      void queryClient.invalidateQueries({ queryKey: INCOME_CANDIDATES_KEY });
                      return null;
                    }}
                  />
                ) : (
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">
                        {titleCase(candidate.display)}
                      </p>
                      <p className="text-xs text-muted-foreground">
                        {amountLabel(candidate)} · {cadence}
                        {account ? ` · ${account}` : ""} · last seen{" "}
                        {formatIsoDate(candidate.last_seen)}
                      </p>
                    </div>
                    <div className="flex shrink-0 items-center gap-2">
                      <Button size="sm" onClick={() => setAdding(id)}>
                        Add as income
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={dismissing === id}
                        onClick={() => void deny(candidate)}
                      >
                        {dismissing === id ? (
                          <Loader2 className="animate-spin" aria-hidden />
                        ) : null}
                        Dismiss
                      </Button>
                    </div>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
        {error ? (
          <p role="alert" className="mt-2 text-sm text-loss">
            {error}
          </p>
        ) : null}
      </CardContent>
    </Card>
  );
}
