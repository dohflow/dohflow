import { useState } from "react";

import type { RecurringBillDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { formatMoney } from "@/lib/format";
import { describeIpcError } from "@/vault/useVault";

import { belowScheduledMessage, cadenceChangedMessage } from "./linkMessages";
import { useBillLoanActions } from "./useBills";

/// What a bill's detail says about the loan it is linked to, or the answer that it is NOT a loan's
/// payment (personal-cfo-6wk.22; ADR 0035 addendum 2026-10-25). While the link is in force the bill
/// carries the cash once; Unlink restores the loan's own payment. Renders nothing for a bill with
/// neither a link nor an answer.
export function BillLoanLinkSection({ bill }: { bill: RecurringBillDto }) {
  const { unlinkBill, forgetNotLoanPayment } = useBillLoanActions();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const link = bill.loan_link;
  const answer = bill.not_loan_payment;
  if (!link && !answer) return null;

  async function run(action: () => Promise<unknown>) {
    setPending(true);
    setError(null);
    const failure = (await action()) as Parameters<typeof describeIpcError>[0] | null;
    setPending(false);
    if (failure) setError(describeIpcError(failure));
  }

  return (
    <section className="flex flex-col gap-2" aria-label="Loan link">
      {link && (
        <>
          <div className="flex items-center justify-between gap-3 text-sm">
            <span>{`Linked to ${link.loan_name}'s payment.`}</span>
            <Button
              size="sm"
              variant="outline"
              disabled={pending}
              onClick={() => void run(() => unlinkBill(bill.id))}
            >
              Unlink
            </Button>
          </div>
          {link.in_force && link.scheduled_payment && link.other && (
            <p className="text-xs text-muted-foreground">
              {`Includes ${formatMoney(link.scheduled_payment)} for ${link.loan_name} (principal and interest) and ${formatMoney(link.other)} other, such as escrow.`}
            </p>
          )}
          {link.cadence_changed && (
            <p className="text-xs text-warning">{cadenceChangedMessage(link.loan_name)}</p>
          )}
          {link.below_scheduled && (
            <p className="text-xs text-warning">{belowScheduledMessage(link.loan_name)}</p>
          )}
        </>
      )}
      {answer && (
        <div className="flex items-center justify-between gap-3 text-sm">
          <span>{`You said this isn't ${answer.loan_name}'s payment.`}</span>
          <Button
            size="sm"
            variant="outline"
            disabled={pending}
            onClick={() => void run(() => forgetNotLoanPayment(bill.id))}
          >
            Forget this answer
          </Button>
        </div>
      )}
      {error && (
        <p role="alert" className="text-xs text-loss">
          {error}
        </p>
      )}
    </section>
  );
}
