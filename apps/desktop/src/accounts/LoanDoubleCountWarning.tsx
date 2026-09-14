import { AlertTriangle } from "lucide-react";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

import { useLoanDoubleCountWarnings } from "./useLoanDoubleCountWarnings";

/// How a suspected pair was matched — plain description, not advice (ADR 0018).
function matchReason(nameMatch: boolean, amountMatch: boolean): string {
  if (nameMatch && amountMatch) return "Matched by name and payment amount.";
  if (amountMatch) return "Matched by payment amount.";
  return "Matched by name.";
}

/// A descriptive warning (personal-cfo-6wk.11, ADR 0018) when a loan is tracked BOTH as a loan
/// account with payment terms AND an active recurring `loan_payment` bill — which counts its
/// payment twice in the Future Cash forecast. Names the suspected pairs; the app never removes
/// either side for you. Renders nothing when no overlap is detected.
export function LoanDoubleCountWarning() {
  const { warnings } = useLoanDoubleCountWarnings();
  if (!warnings || warnings.length === 0) return null;

  return (
    <Card
      role="region"
      aria-label="Possible duplicated loans"
      className="border-warning/40 bg-warning/5"
    >
      <CardHeader className="pb-2">
        <CardTitle className="flex items-center gap-2 text-base text-warning">
          <AlertTriangle className="size-4 shrink-0" aria-hidden />
          Possible duplicated {warnings.length > 1 ? "loans" : "loan"}
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-3 text-sm">
        <p className="text-muted-foreground">
          A loan tracked as both a loan account and a recurring loan-payment bill has its payment
          counted twice in your Cash Flow forecast. These look like the same loan tracked both
          ways.
        </p>
        <ul className="flex flex-col gap-2">
          {warnings.map((w) => (
            <li
              key={`${w.loan_account_id}:${w.bill_event_id}`}
              className="rounded-md bg-background/60 px-3 py-2"
            >
              <div className="font-medium">
                {w.loan_name} <span className="text-muted-foreground">(loan account)</span> &amp;{" "}
                {w.bill_name} <span className="text-muted-foreground">(recurring bill)</span>
              </div>
              <div className="text-xs text-muted-foreground">
                {matchReason(w.name_match, w.amount_match)}
              </div>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  );
}
