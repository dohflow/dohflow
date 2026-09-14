import type { AccountViewDto, DebtTermsDto } from "@/bindings";

import {
  debtStats,
  minimumDue,
  monthlyInterestMinor,
  monthlyPaymentMinor,
  nextPaymentDueDay,
  termRemainingMonths,
  weightedAverageAprBps,
  type ScopedDebt,
} from "./debtStats";

const terms = (over: Partial<DebtTermsDto> = {}): DebtTermsDto =>
  ({
    account_id: "card-1",
    apr_bps: 2199,
    statement_close_day: 5,
    payment_due_day: 25,
    grace_period_days: 21,
    credit_limit_minor: 1_000_000,
    repayment_philosophy: "pay_minimum",
    fixed_amount_minor: null,
    min_payment_percent_bps: null,
    min_payment_floor_minor: null,
    paying_source_account_id: null,
    original_principal_minor: null,
    ...over,
  }) as DebtTermsDto;

const debt = (
  owedMinor: number,
  t: DebtTermsDto | null = terms(),
  currency = "USD",
): ScopedDebt => ({
  account: {
    id: t?.account_id ?? "x",
    name: "Visa",
    balance: { minor_units: -owedMinor, currency },
  } as AccountViewDto,
  terms: t,
  owedMinor,
});

describe("minimumDue — mirrors forecast_engine::revolving::minimum_due", () => {
  it("takes the greater of percent and floor", () => {
    expect(minimumDue(500_000, 100, 2_500)).toBe(5_000); // 1% of $5,000 = $50 > $25 floor
    expect(minimumDue(100_000, 100, 2_500)).toBe(2_500); // 1% of $1,000 = $10 < $25 floor
  });
  it("never exceeds the balance", () => {
    // A $25 floor on a $10 balance is $10 — otherwise the surface would show a payment
    // larger than the debt.
    expect(minimumDue(1_000, 100, 2_500)).toBe(1_000);
  });
  it("is zero on a cleared debt", () => {
    expect(minimumDue(0, 100, 2_500)).toBe(0);
  });
});

describe("monthlyPaymentMinor", () => {
  it("uses the ADR 0035 §5 defaults only when nothing is recorded", () => {
    expect(monthlyPaymentMinor(debt(500_000))).toBe(5_000); // 1% default
  });
  it("respects a recorded floor with a zero percent", () => {
    // Recording a $10 floor and no percent MEANS $10 — the defaults must not override a
    // deliberate setting.
    expect(
      monthlyPaymentMinor(
        debt(500_000, terms({ min_payment_percent_bps: 0, min_payment_floor_minor: 1_000 })),
      ),
    ).toBe(1_000);
  });
  it("uses the fixed amount for a fixed payer", () => {
    expect(
      monthlyPaymentMinor(
        debt(500_000, terms({ repayment_philosophy: "pay_fixed_amount", fixed_amount_minor: 30_000 })),
      ),
    ).toBe(30_000);
  });
  it("clears the balance for a full payer", () => {
    expect(
      monthlyPaymentMinor(debt(120_000, terms({ repayment_philosophy: "pay_statement_balance" }))),
    ).toBe(120_000);
  });
  it("is UNKNOWN, not zero, when no terms are recorded", () => {
    // Guessing a default here would put a number in a household total they never agreed to.
    expect(monthlyPaymentMinor(debt(500_000, null))).toBeNull();
  });
});

describe("monthlyInterestMinor", () => {
  it("is the balance at the monthly rate", () => {
    // $5,000 at 21.99% → 5_000_00 * 0.2199 / 12 ≈ $91.63
    expect(monthlyInterestMinor(debt(500_000))).toBe(9_163);
  });
  it("is UNKNOWN, not zero, with no APR on record", () => {
    // The whole reason debt_terms_list omits termless accounts: no rate recorded is not 0%.
    expect(monthlyInterestMinor(debt(500_000, terms({ apr_bps: null })))).toBeNull();
  });
});

describe("weightedAverageAprBps", () => {
  it("weights by balance, not by count", () => {
    // $9,000 at 5% and $1,000 at 25% averages to 7%, not 15%.
    const { aprBps } = weightedAverageAprBps([
      debt(900_000, terms({ apr_bps: 500 })),
      debt(100_000, terms({ apr_bps: 2_500 })),
    ]);
    expect(aprBps).toBe(700);
  });
  it("excludes and COUNTS debts with no APR rather than averaging in a zero", () => {
    const { aprBps, excluded } = weightedAverageAprBps([
      debt(500_000, terms({ apr_bps: 2_000 })),
      debt(500_000, terms({ apr_bps: null })),
    ]);
    expect(aprBps).toBe(2_000);
    expect(excluded).toBe(1);
  });
  it("is null when nothing is owed", () => {
    expect(weightedAverageAprBps([debt(0)]).aprBps).toBeNull();
  });
});

describe("termRemainingMonths", () => {
  it("amortizes a normal debt", () => {
    // $5,000 at 21.99%, paying $300/mo → about 20 months.
    const months = termRemainingMonths(
      debt(500_000, terms({ repayment_philosophy: "pay_fixed_amount", fixed_amount_minor: 30_000 })),
    );
    expect(months).toBeGreaterThan(17);
    expect(months).toBeLessThan(24);
  });
  it("says NOT AT THIS PAYMENT when the payment does not beat the interest", () => {
    // THE case worth getting right: $5,000 at 21.99% accrues ~$92/mo, so a $25 minimum
    // never amortizes. The formula returns a negative/NaN here — rendering it would tell a
    // household their debt clears in −4 months.
    expect(
      termRemainingMonths(
        debt(500_000, terms({ repayment_philosophy: "pay_fixed_amount", fixed_amount_minor: 2_500 })),
      ),
    ).toBeNull();
  });
  it("divides plainly at 0% APR", () => {
    expect(
      termRemainingMonths(
        debt(
          120_000,
          terms({ apr_bps: 0, repayment_philosophy: "pay_fixed_amount", fixed_amount_minor: 30_000 }),
        ),
      ),
    ).toBe(4);
  });
  it("claims no payoff date with no APR on record", () => {
    expect(
      termRemainingMonths(
        debt(
          500_000,
          terms({ apr_bps: null, repayment_philosophy: "pay_fixed_amount", fixed_amount_minor: 30_000 }),
        ),
      ),
    ).toBeNull();
  });
});

describe("nextPaymentDueDay", () => {
  it("picks the next day still ahead this month", () => {
    expect(
      nextPaymentDueDay([debt(1, terms({ payment_due_day: 5 })), debt(1, terms({ payment_due_day: 25 }))], 10),
    ).toEqual({ day: 25, count: 1 });
  });
  it("rolls to the earliest day when all have passed", () => {
    expect(
      nextPaymentDueDay([debt(1, terms({ payment_due_day: 5 })), debt(1, terms({ payment_due_day: 8 }))], 20),
    ).toEqual({ day: 5, count: 1 });
  });
  it("counts debts sharing the day", () => {
    expect(
      nextPaymentDueDay([debt(1, terms({ payment_due_day: 15 })), debt(1, terms({ payment_due_day: 15 }))], 1),
    ).toEqual({ day: 15, count: 2 });
  });
  it("is null when no debt records a due day", () => {
    expect(nextPaymentDueDay([debt(1, terms({ payment_due_day: null }))], 1)).toBeNull();
  });
});

describe("debtStats", () => {
  it("refuses to total when a debt has no terms", () => {
    // A total that silently omits a debt is worse than one that admits it is incomplete.
    const stats = debtStats([debt(500_000), debt(200_000, null)], "USD");
    expect(stats.minimumsMinor).toBeNull();
    expect(stats.interestMinor).toBeNull();
    // The owed total still stands — a balance is known even when its terms are not.
    expect(stats.totalOwedMinor).toBe(700_000);
  });

  it("scopes to the reporting currency rather than summing unlike amounts", () => {
    // ADR 0057 §4: there is no offline FX, so adding them would invent a number.
    const stats = debtStats([debt(500_000), debt(300_000, terms(), "EUR")], "USD");
    expect(stats.totalOwedMinor).toBe(500_000);
    expect(stats.otherCurrency).toBe(1);
  });

  it("totals minimums and interest across the scope", () => {
    const stats = debtStats([debt(500_000), debt(100_000)], "USD");
    expect(stats.minimumsMinor).toBe(5_000 + 2_500);
    expect(stats.interestMinor).toBe(9_163 + 1_833);
  });
});
