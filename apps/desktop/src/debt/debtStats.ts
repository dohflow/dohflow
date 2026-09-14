import type { AccountViewDto, DebtTermsDto } from "@/bindings";

/// ADR 0035 §5 defaults, used when a debt records neither a percent nor a floor. Mirrors
/// `DEFAULT_MIN_PERCENT_BPS` / `DEFAULT_MIN_FLOOR_MINOR` in db-worker's card_cycles.
const DEFAULT_MIN_PERCENT_BPS = 100; // 1%
const DEFAULT_MIN_FLOOR_MINOR = 2_500; // $25

/// A debt paired with its terms, if any were recorded.
export type ScopedDebt = {
  account: AccountViewDto;
  /// `null` when no terms are on record — which is NOT the same as terms of zero.
  terms: DebtTermsDto | null;
  /// Positive amount owed, in minor units.
  owedMinor: number;
};

/// The minimum payment: greater-of percent-of-balance / floor, never more than the balance.
///
/// A direct mirror of `forecast_engine::revolving::minimum_due`. Kept in step deliberately:
/// a household reading "$75 minimum" here and seeing the forecast pay something else would
/// have no way to tell which one is lying.
export function minimumDue(
  owedMinor: number,
  percentBps: number,
  floorMinor: number,
): number {
  if (owedMinor <= 0) return 0;
  const pct = Math.trunc((owedMinor * percentBps) / 10_000);
  return Math.min(Math.max(pct, floorMinor), owedMinor);
}

/// The percent/floor a debt actually pays by, applying the ADR 0035 §5 defaults only when
/// BOTH are unset — a debt that records a 0% floor of $10 means $10, not the default.
export function effectiveMinTerms(terms: DebtTermsDto): {
  percentBps: number;
  floorMinor: number;
} {
  const pct = terms.min_payment_percent_bps ?? 0;
  const floor = terms.min_payment_floor_minor ?? 0;
  const fixed = terms.fixed_amount_minor ?? 0;
  if (fixed === 0 && pct === 0 && floor === 0) {
    return {
      percentBps: DEFAULT_MIN_PERCENT_BPS,
      floorMinor: DEFAULT_MIN_FLOOR_MINOR,
    };
  }
  return { percentBps: pct, floorMinor: floor };
}

/// What this debt takes out of cash each month, by its repayment philosophy.
///
/// `null` when no terms are recorded: the payment is genuinely unknown, and guessing a
/// default here would put a number in a total that the household never agreed to.
export function monthlyPaymentMinor(debt: ScopedDebt): number | null {
  if (debt.terms === null) return null;
  if (debt.owedMinor <= 0) return 0;
  const philosophy = debt.terms.repayment_philosophy as string;
  if (philosophy === "pay_fixed_amount") {
    const fixed = debt.terms.fixed_amount_minor ?? 0;
    return Math.min(fixed, debt.owedMinor);
  }
  // Full payers clear the balance each cycle; there is no amortizing minimum to quote.
  if (
    philosophy === "pay_in_full" ||
    philosophy === "pay_statement_balance" ||
    philosophy === "pay_current_balance"
  ) {
    return debt.owedMinor;
  }
  const { percentBps, floorMinor } = effectiveMinTerms(debt.terms);
  return minimumDue(debt.owedMinor, percentBps, floorMinor);
}

/// Interest this debt accrues over one month at its current rate and balance.
///
/// `null` when no APR is on record. **Not zero** — a debt with no rate recorded is not a
/// debt at 0%, and folding it in as zero would understate what the debt costs.
export function monthlyInterestMinor(debt: ScopedDebt): number | null {
  const apr = debt.terms?.apr_bps ?? null;
  if (apr === null) return null;
  if (debt.owedMinor <= 0) return 0;
  return Math.round((debt.owedMinor * apr) / 10_000 / 12);
}

/// The balance-weighted average APR, in basis points.
///
/// Debts with no APR on record are **excluded from the weighting and counted**, so the
/// surface can say what the average leaves out rather than quietly averaging a zero in.
export function weightedAverageAprBps(debts: ScopedDebt[]): {
  aprBps: number | null;
  excluded: number;
} {
  let weighted = 0;
  let total = 0;
  let excluded = 0;
  for (const debt of debts) {
    if (debt.owedMinor <= 0) continue;
    const apr = debt.terms?.apr_bps ?? null;
    if (apr === null) {
      excluded += 1;
      continue;
    }
    weighted += debt.owedMinor * apr;
    total += debt.owedMinor;
  }
  return {
    aprBps: total === 0 ? null : Math.round(weighted / total),
    excluded,
  };
}

/// Months until this debt reaches zero at its current payment.
///
/// `null` means "not at this payment" — either there is no payment to model, or the payment
/// does not exceed the interest. That case is the one worth getting right: the amortization
/// formula returns a negative or non-finite result there, and rendering that as a number
/// would tell a household their debt clears in −4 months.
export function termRemainingMonths(debt: ScopedDebt): number | null {
  const payment = monthlyPaymentMinor(debt);
  if (payment === null || payment <= 0 || debt.owedMinor <= 0) return null;

  const apr = debt.terms?.apr_bps ?? null;
  // No rate recorded → treat as non-amortizing arithmetic rather than assuming 0%: we do
  // not know the rate, so we do not claim a payoff date.
  if (apr === null) return null;
  if (apr === 0) return Math.ceil(debt.owedMinor / payment);

  const monthlyRate = apr / 10_000 / 12;
  const interestFirstMonth = debt.owedMinor * monthlyRate;
  if (payment <= interestFirstMonth) return null;

  const months =
    -Math.log(1 - (monthlyRate * debt.owedMinor) / payment) /
    Math.log(1 + monthlyRate);
  return Number.isFinite(months) && months > 0 ? Math.ceil(months) : null;
}

/// The next day-of-month any debt in scope has a payment due, and how many share it.
///
/// Day-of-month only: `debt_terms` records a due DAY, not a date, so this deliberately does
/// not manufacture a calendar date it cannot substantiate.
export function nextPaymentDueDay(
  debts: ScopedDebt[],
  todayDay: number,
): { day: number; count: number } | null {
  const days = debts
    .map((d) => d.terms?.payment_due_day ?? null)
    .filter((d): d is number => d !== null && d >= 1 && d <= 31);
  if (days.length === 0) return null;
  // The next occurrence is the smallest day still ahead this month; if none are, the month
  // rolls over and the earliest day overall is next.
  const ahead = days.filter((d) => d >= todayDay);
  const day = ahead.length > 0 ? Math.min(...ahead) : Math.min(...days);
  return { day, count: days.filter((d) => d === day).length };
}

/// Everything the stat row states, derived once so the surface does no arithmetic.
export type DebtStats = {
  totalOwedMinor: number;
  aprBps: number | null;
  /// Debts left out of the APR average because no rate is on record.
  aprExcluded: number;
  /// `null` when at least one debt has no terms — a total that silently omits a debt is
  /// worse than one that admits it is incomplete.
  minimumsMinor: number | null;
  interestMinor: number | null;
  nextDue: { day: number; count: number } | null;
  /// Debts in scope whose currency differs from the reporting currency, and are therefore
  /// excluded from every total above (ADR 0057 §4: scoped, not summed).
  otherCurrency: number;
};

export function debtStats(
  debts: ScopedDebt[],
  reportingCurrency: string,
): DebtStats {
  // Mixed currencies are scoped, not summed — there is no offline FX, so adding unlike
  // amounts would invent a number (ADR 0057 §4). The count is returned so the surface can
  // say what it left out.
  const inCurrency = debts.filter(
    (d) => d.account.balance.currency === reportingCurrency,
  );
  const otherCurrency = debts.length - inCurrency.length;

  const totalOwedMinor = inCurrency.reduce((sum, d) => sum + Math.max(d.owedMinor, 0), 0);
  const { aprBps, excluded } = weightedAverageAprBps(inCurrency);

  const payments = inCurrency.map(monthlyPaymentMinor);
  const minimumsMinor = payments.some((p) => p === null)
    ? null
    : payments.reduce((sum: number, p) => sum + (p ?? 0), 0);

  const interests = inCurrency.map(monthlyInterestMinor);
  const interestMinor = interests.some((i) => i === null)
    ? null
    : interests.reduce((sum: number, i) => sum + (i ?? 0), 0);

  return {
    totalOwedMinor,
    aprBps,
    aprExcluded: excluded,
    minimumsMinor,
    interestMinor,
    nextDue: nextPaymentDueDay(inCurrency, new Date().getDate()),
    otherCurrency,
  };
}
