import type { CreateRecurringBillInput, LoanOverlapMatchDto } from "@/bindings";
import { commands } from "@/bindings";
import { mintIdempotencyKey } from "@/lib/idempotency";

/// The bill fields a loan-overlap check reads. Both creation forms that run it build these.
export type OverlapCheckDraft = Omit<
  CreateRecurringBillInput,
  "source_merchant_key" | "idempotency_key"
>;

/// Before a bill is created, ask whether it looks like a loan's payment (personal-cfo-6wk.20).
/// Shared by every form that creates a bill (the bill form, a promoted transaction or suggestion,
/// and Add transaction's recurring mode: personal-cfo-6wk.23) so the check, its fail-open rule and
/// its answer handling exist once. Returns the match to show, or null to create the bill.
///
/// Fail open: the check is advisory, so if it cannot run (an error result, or a throw) the bill is
/// still created. `notPaymentOfLoanId`: a suggestion row already answered "is this the loan's
/// payment?" with "Add as a separate bill", so the question is not asked again.
export async function findLoanOverlap(
  draft: OverlapCheckDraft,
  options: { sourceMerchantKey: string | null; notPaymentOfLoanId?: string },
): Promise<LoanOverlapMatchDto | null> {
  try {
    const checked = await commands.checkLoanOverlap({
      ...draft,
      source_merchant_key: options.sourceMerchantKey,
      idempotency_key: mintIdempotencyKey(),
    });
    if (checked.status === "ok" && checked.data) {
      const asksAboutLoan =
        checked.data.strength === "possible" &&
        checked.data.matched_by === "declared_terms";
      if (!(asksAboutLoan && options.notPaymentOfLoanId)) return checked.data;
    }
  } catch {
    // fall through to creation
  }
  return null;
}
