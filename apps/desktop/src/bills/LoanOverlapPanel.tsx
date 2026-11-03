import type { AccountViewDto, LoanOverlapMatchDto } from "@/bindings";
import { Button } from "@/components/ui/button";

import { useBills, type BillFollowUp } from "./useBills";

/// "Add and link to <loan>" (personal-cfo-6wk.22): creates the bill, then links it to the loan it
/// is the payment of. Only offered while the loan has no linked bill (one per loan), which the bill
/// list already says; the server still refuses a second link as a backstop.
function AddAndLinkButton({
  loanId,
  loanName,
  saving,
  onClick,
}: {
  loanId: string;
  loanName: string;
  saving: boolean;
  onClick: () => void;
}) {
  const { bills } = useBills();
  const taken = (bills ?? []).some(
    (b) => b.loan_link?.loan_account_id === loanId,
  );
  if (taken) return null;
  return (
    <Button type="button" disabled={saving} onClick={onClick}>
      {saving ? "Saving…" : `Add and link to ${loanName}`}
    </Button>
  );
}

/// How a creation-time match is presented (personal-cfo-6wk.20): `strong` is the loan's payment,
/// `ask` is a question (the user's declared loan terms), `soft` is a note.
function overlapKind(match: LoanOverlapMatchDto): "strong" | "ask" | "soft" {
  if (match.strength === "strong") return "strong";
  return match.matched_by === "declared_terms" ? "ask" : "soft";
}

/// The warning a match produces, with the choices it offers: the confirm panel that replaces a
/// form's buttons until the user decides. Descriptive and advisory only (ADR 0018). `onCreate` is
/// "create the bill anyway", with what the user chose about the loan (link it, or remember it is
/// not that loan's payment); `onCancel` is "don't add".
export function LoanOverlapPanel({
  match,
  autopayAccountId,
  accounts,
  saving,
  submitLabel,
  onCancel,
  onCreate,
}: {
  match: LoanOverlapMatchDto;
  autopayAccountId: string | null;
  accounts: AccountViewDto[];
  saving: boolean;
  submitLabel: string;
  onCancel: () => void;
  onCreate: (follow?: BillFollowUp) => void;
}) {
  return (
    <div
      role="alert"
      className="flex flex-col gap-3 rounded-md border border-warning/40 bg-warning/5 p-3 text-sm"
    >
      {overlapKind(match) === "strong" && (
        <>
          <p>{`This is ${match.loan_name}'s payment — don't add a second one.`}</p>
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="ghost"
              disabled={saving}
              onClick={onCancel}
            >
              Don&apos;t add
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={saving}
              onClick={() => onCreate()}
            >
              {saving ? "Saving…" : "Add anyway"}
            </Button>
            <AddAndLinkButton
              loanId={match.loan_account_id}
              loanName={match.loan_name}
              saving={saving}
              onClick={() =>
                onCreate({
                  linkLoanAccountId: match.loan_account_id,
                  linkLoanName: match.loan_name,
                })
              }
            />
          </div>
        </>
      )}
      {overlapKind(match) === "ask" && (
        <>
          {/* The user's own loan terms are enough to ASK, never to assert it is counted twice. */}
          <p className="font-medium">{`Is this ${match.loan_name}'s payment?`}</p>
          <p className="text-muted-foreground">
            {`It's paid from ${
              accounts.find((account) => account.id === autopayAccountId)
                ?.name ?? "the loan's paying account"
            } around ${match.loan_name}'s due date, as set in its terms.`}
          </p>
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="ghost"
              disabled={saving}
              onClick={onCancel}
            >
              Yes — don&apos;t add a second one
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={saving}
              onClick={() =>
                onCreate({
                  notPaymentOfLoanAccountId: match.loan_account_id,
                })
              }
            >
              {saving ? "Saving…" : "No, it's a different bill"}
            </Button>
          </div>
        </>
      )}
      {overlapKind(match) === "soft" && (
        <>
          <p>{`This bill might be the same as ${match.loan_name}'s payment.`}</p>
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="ghost"
              disabled={saving}
              onClick={onCancel}
            >
              Cancel
            </Button>
            <Button type="button" disabled={saving} onClick={() => onCreate()}>
              {saving ? "Saving…" : submitLabel}
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
