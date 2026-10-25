import { AlertTriangle } from "lucide-react";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

import type { LoanDoubleCountWarningDto } from "@/bindings";

import { useLoanDoubleCountWarnings } from "./useLoanDoubleCountWarnings";

/// The type labels a loan-typed strong match can carry (planning-approved wording, 6wk.20).
const LOAN_TYPED_LABELS: Record<string, string> = {
  loan_payment: "Loan payment",
  rent_mortgage: "Rent or mortgage",
};

/// Why a STRONG match is one: the evidence route, in plain description, not advice (ADR 0018).
/// Planning-approved sentences (personal-cfo-6wk.20). Null when the evidence cannot be named.
function strongReason(w: LoanDoubleCountWarningDto): string | null {
  if (w.matched_by === "payee") {
    return `Same payee as the payments already made to ${w.loan_name}, around its due date.`;
  }
  if (w.matched_by === "loan_typed") {
    const label = w.bill_type ? LOAN_TYPED_LABELS[w.bill_type] : undefined;
    if (!label || !w.paid_from_account_name) return null;
    return `A ${label} bill paid from ${w.paid_from_account_name} around ${w.loan_name}'s due date.`;
  }
  return null;
}

/// A descriptive warning (personal-cfo-6wk.11, ADR 0018) when a loan is tracked BOTH as a loan
/// account with payment terms AND an active recurring bill that is its payment, which counts that
/// payment twice in the Future Cash forecast. Names the suspected pairs; the app never removes either
/// side for you. Renders nothing when no overlap is detected.
///
/// Only a STRONG match (personal-cfo-6wk.20) says the payment is "counted twice"; a POSSIBLE match is
/// a soft note ("might be the same as <loan>'s payment") that makes no such claim.
export function LoanDoubleCountWarning() {
  const { warnings } = useLoanDoubleCountWarnings();
  if (!warnings || warnings.length === 0) return null;
  const strong = warnings.filter((w) => w.strength === "strong");
  const possible = warnings.filter((w) => w.strength === "possible");

  return (
    <>
      {strong.length > 0 && (
        <Card
          role="region"
          aria-label="Possible duplicated loans"
          className="border-warning/40 bg-warning/5"
        >
          <CardHeader className="pb-2">
            <CardTitle className="flex items-center gap-2 text-base text-warning">
              <AlertTriangle className="size-4 shrink-0" aria-hidden />
              Possible duplicated {strong.length > 1 ? "loans" : "loan"}
            </CardTitle>
          </CardHeader>
          <CardContent className="flex flex-col gap-3 text-sm">
            <p className="text-muted-foreground">
              This loan&apos;s payment is also set up as a recurring bill, so it&apos;s counted twice
              in Cash Flow and your forecast.
            </p>
            <ul className="flex flex-col gap-2">
              {strong.map((w) => (
                <li
                  key={`${w.loan_account_id}:${w.bill_event_id}`}
                  className="rounded-md bg-background/60 px-3 py-2"
                >
                  <div className="font-medium">
                    {w.loan_name} <span className="text-muted-foreground">(loan account)</span>{" "}
                    &amp; {w.bill_name}{" "}
                    <span className="text-muted-foreground">(recurring bill)</span>
                  </div>
                  {strongReason(w) && (
                    <div className="text-xs text-muted-foreground">{strongReason(w)}</div>
                  )}
                </li>
              ))}
            </ul>
          </CardContent>
        </Card>
      )}
      {possible.length > 0 && (
        <Card role="region" aria-label="Possible matches" className="border-border bg-muted/30">
          <CardContent className="flex flex-col gap-2 pt-4 text-sm text-muted-foreground">
            <ul className="flex flex-col gap-1">
              {possible.map((w) => (
                <li key={`${w.loan_account_id}:${w.bill_event_id}`}>
                  <span className="font-medium text-foreground">{w.bill_name}</span>
                  <div>This bill might be the same as {w.loan_name}&apos;s payment.</div>
                </li>
              ))}
            </ul>
          </CardContent>
        </Card>
      )}
    </>
  );
}
