//! Revolving-balance interest accrual + the per-cycle compounding loop (ADR 0035 §4,
//! personal-cfo-llx5).
//!
//! Pure + deterministic, like the rest of the crate: integer-cent math, no `f64` / clock / IO.
//! A credit card that is not paid in full carries a balance that accrues interest, which
//! **compounds** across cycles. Given a starting owed balance and each upcoming cycle's new
//! charges + length, [`project_revolving`] folds the balance forward — interest on, payment
//! off — producing the per-cycle statement balance, minimum due, payment, and carried balance.

/// How the card's payment is sized each cycle — the crate-neutral form of ADR 0035 §1's
/// repayment philosophy (the caller maps its philosophy token onto this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentPolicy {
    /// Pay the full statement balance (`pay_in_full` / `pay_statement_balance` /
    /// `pay_current_balance`).
    FullStatement,
    /// Pay the computed minimum (`pay_minimum` / `unknown`).
    Minimum,
    /// Pay a fixed amount (`pay_fixed_amount`), capped at the statement balance.
    Fixed(i64),
}

/// The card's fixed repayment terms across the projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevolvingTerms {
    /// Annual percentage rate in basis points.
    pub apr_bps: i64,
    /// How the payment is sized each cycle.
    pub policy: PaymentPolicy,
    /// Minimum-payment percent-of-balance in basis points.
    pub min_percent_bps: i64,
    /// Minimum-payment floor in minor units.
    pub min_floor_cents: i64,
}

/// One upcoming cycle's variable inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevolvingCycle {
    /// New charges posting in this cycle (known bills + projected spend), minor units.
    pub new_charges_cents: i64,
    /// The cycle length in days (`close − open`).
    pub days_in_cycle: u32,
    /// A user-asserted statement balance for this cycle (feedback 2026-07-03): when the
    /// real statement is known, it replaces the estimated `opening + charges + interest`
    /// figure, and the fold carries forward from the asserted amount plus any un-billed
    /// remainder — charges already in the opening but not on the recorded statement
    /// (ADR 0039 addendum 2026-07-10 §1).
    pub statement_override_cents: Option<i64>,
}

/// The projected outcome of one cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleProjection {
    /// Carried (owed) balance at cycle open.
    pub opening_cents: i64,
    /// New charges this cycle.
    pub new_charges_cents: i64,
    /// Projected finance charge (interest) this cycle.
    pub finance_charge_cents: i64,
    /// Statement balance = opening + new charges + finance charge.
    pub statement_balance_cents: i64,
    /// Minimum payment due.
    pub minimum_due_cents: i64,
    /// The payment the policy selects.
    pub payment_cents: i64,
    /// Carried balance into the next cycle = `max(0, statement − payment)` plus, when the
    /// statement was overridden, the un-billed remainder `max(0, estimate − statement)` —
    /// post-close charges never vanish (ADR 0039 addendum 2026-07-10 §1).
    pub closing_cents: i64,
}

/// Average-daily-balance finance charge for one cycle (ADR 0035 §4). When the prior statement
/// was paid in full (`opening <= 0`) new purchases get a grace period, so the charge is zero;
/// otherwise interest accrues on the **average daily balance** — the carried `opening` plus
/// half the cycle's new charges (charges assumed evenly spread). Integer cents throughout: the
/// `i128` intermediate cannot overflow, and the final narrowing to `i64` is a saturating
/// `try_from` (never a wrapping cast) so even an absurd stored APR yields a clamped charge, not
/// a negative one.
#[must_use]
pub fn cycle_finance_charge(
    opening_cents: i64,
    new_charges_cents: i64,
    days_in_cycle: u32,
    apr_bps: i64,
) -> i64 {
    if opening_cents <= 0 || apr_bps <= 0 || days_in_cycle == 0 {
        return 0;
    }
    let avg_daily = i128::from(opening_cents) + i128::from(new_charges_cents.max(0)) / 2;
    let numer = avg_daily * i128::from(apr_bps) * i128::from(days_in_cycle);
    i64::try_from(numer / (10_000 * 365)).unwrap_or(i64::MAX)
}

/// The minimum payment: greater-of percent-of-balance / floor, never more than the balance.
#[must_use]
pub fn minimum_due(statement_cents: i64, percent_bps: i64, floor_cents: i64) -> i64 {
    if statement_cents <= 0 {
        return 0;
    }
    let pct = (i128::from(statement_cents) * i128::from(percent_bps) / 10_000) as i64;
    pct.max(floor_cents).min(statement_cents)
}

fn payment_for(policy: PaymentPolicy, statement_cents: i64, minimum_cents: i64) -> i64 {
    let amount = match policy {
        PaymentPolicy::FullStatement => statement_cents,
        PaymentPolicy::Minimum => minimum_cents,
        PaymentPolicy::Fixed(fixed) => fixed,
    };
    amount.clamp(0, statement_cents.max(0))
}

/// Fold the owed balance across `cycles`: interest on the carried balance, then the policy's
/// payment off, carrying the remainder forward (the compounding loop, ADR 0035 §4). A negative
/// `opening` (a credit balance) is clamped to zero.
///
/// The carried balance is tracked as two components (ADR 0039 addendum 2026-07-10 §1):
/// **billed** — unpaid statement debt, the revolving balance that accrues interest and defeats
/// the grace period — and **un-billed** — post-close charges carried past a recorded statement,
/// which stay grace-eligible (interest-wise they behave like new charges: ADB half-weight, and
/// zero under grace when the prior statement was paid in full) until the next statement bills
/// them. `CycleProjection::opening_cents`/`closing_cents` report the sum.
#[must_use]
pub fn project_revolving(
    opening_cents: i64,
    cycles: &[RevolvingCycle],
    terms: &RevolvingTerms,
) -> Vec<CycleProjection> {
    let mut billed = opening_cents.max(0);
    let mut unbilled = 0i64;
    let mut out = Vec::with_capacity(cycles.len());
    for cycle in cycles {
        // Grace test + interest basis use only the BILLED remainder; the carried un-billed
        // charges join this cycle's new charges (half-weighted average-daily-balance).
        let finance_charge = cycle_finance_charge(
            billed,
            cycle.new_charges_cents.saturating_add(unbilled),
            cycle.days_in_cycle,
            terms.apr_bps,
        );
        let opening = billed.saturating_add(unbilled);
        // A known (user-asserted) statement replaces the estimate outright; the carry
        // into the next cycle then flows from the real number, not the projection.
        let estimate = opening
            .saturating_add(cycle.new_charges_cents)
            .saturating_add(finance_charge);
        let statement = cycle.statement_override_cents.unwrap_or(estimate);
        // The owed opening can exceed a recorded statement — the difference is charges
        // incurred after the close, on the card but not on that statement. They open the
        // next cycle instead of vanishing (ADR 0039 addendum 2026-07-10 §1). Zero when
        // there is no override (estimate == statement).
        let next_unbilled = estimate.saturating_sub(statement).max(0);
        let minimum = minimum_due(statement, terms.min_percent_bps, terms.min_floor_cents);
        let payment = payment_for(terms.policy, statement, minimum);
        let next_billed = (statement - payment).max(0);
        out.push(CycleProjection {
            opening_cents: opening,
            new_charges_cents: cycle.new_charges_cents,
            finance_charge_cents: finance_charge,
            statement_balance_cents: statement,
            minimum_due_cents: minimum,
            payment_cents: payment,
            closing_cents: next_billed.saturating_add(next_unbilled),
        });
        billed = next_billed;
        unbilled = next_unbilled;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONTH: u32 = 30;

    fn terms(apr_bps: i64, policy: PaymentPolicy) -> RevolvingTerms {
        RevolvingTerms {
            apr_bps,
            policy,
            min_percent_bps: 100, // 1%
            min_floor_cents: 2_500,
        }
    }

    fn cycles(n: usize, new_charges: i64) -> Vec<RevolvingCycle> {
        vec![
            RevolvingCycle {
                new_charges_cents: new_charges,
                days_in_cycle: MONTH,
                statement_override_cents: None,
            };
            n
        ]
    }

    /// A user-asserted statement replaces the estimate AND re-anchors the carry.
    #[test]
    fn statement_override_replaces_the_estimate_and_reanchors_the_carry() {
        // Opening $500, $200 charges/cycle, pay in full. Cycle 0's real statement is
        // asserted at $850 (the estimate would be $700): the payment follows the real
        // number and the next cycle opens from its remainder (0 here — paid in full).
        let mut cs = cycles(2, 20_000);
        cs[0].statement_override_cents = Some(85_000);
        let proj = project_revolving(50_000, &cs, &terms(0, PaymentPolicy::FullStatement));
        assert_eq!(proj[0].statement_balance_cents, 85_000, "override wins");
        assert_eq!(proj[0].payment_cents, 85_000);
        assert_eq!(proj[0].closing_cents, 0);
        // Cycle 1 is untouched: estimated from its own charges.
        assert_eq!(proj[1].statement_balance_cents, 20_000);
    }

    /// The owner scenario (personal-cfo-4d8.25.3): owed 17,082.23 but the recorded statement
    /// is 13,873.08 — the 3,209.15 of post-close charges must open the next cycle, not vanish.
    #[test]
    fn override_below_the_owed_opening_carries_post_close_charges_forward() {
        let mut cs = cycles(2, 0);
        cs[0].statement_override_cents = Some(1_387_308);
        let proj = project_revolving(1_708_223, &cs, &terms(0, PaymentPolicy::FullStatement));
        assert_eq!(proj[0].statement_balance_cents, 1_387_308, "override wins");
        assert_eq!(proj[0].payment_cents, 1_387_308, "paid in full");
        assert_eq!(
            proj[0].closing_cents, 320_915,
            "post-close charges (owed − statement) carry, not vanish"
        );
        assert_eq!(proj[1].opening_cents, 320_915);
        assert_eq!(
            proj[1].statement_balance_cents, 320_915,
            "the next statement bills the carried post-close charges"
        );
    }

    /// The grace period survives the un-billed carry (adversarial review of 4d8.25.3): a
    /// pay-in-full card that records a statement below its owed opening accrues ZERO
    /// interest on the carried post-close charges — they are grace-eligible new spend,
    /// not unpaid statement debt (ADR 0035 §4).
    #[test]
    fn unbilled_carry_keeps_the_grace_period_under_full_payment() {
        let mut cs = cycles(3, 0);
        cs[0].statement_override_cents = Some(1_387_308);
        let proj = project_revolving(1_708_223, &cs, &terms(2_400, PaymentPolicy::FullStatement));
        // Cycle 0 carries the pre-existing billed balance, so it accrues interest as today.
        assert!(proj[0].finance_charge_cents > 0);
        assert_eq!(
            proj[0].payment_cents, 1_387_308,
            "recorded statement paid in full"
        );
        // Cycle 1 opens on grace-eligible un-billed carry only: NO phantom interest.
        assert_eq!(
            proj[1].finance_charge_cents, 0,
            "paid-in-full: the un-billed carry must not defeat the grace period"
        );
        assert_eq!(
            proj[1].statement_balance_cents, proj[1].opening_cents,
            "the next statement bills exactly the carried charges, nothing more"
        );
        assert_eq!(proj[1].closing_cents, 0, "paid in full again");
        assert_eq!(proj[2].finance_charge_cents, 0);
    }

    /// Under a partial (minimum) payment the interest basis is the unpaid BILLED remainder;
    /// the un-billed carry is half-weighted like new charges — hand-computed table.
    #[test]
    fn unbilled_carry_is_half_weighted_in_interest_under_partial_payment() {
        // Opening $1,000 billed, override $600, 24% APR, 30-day cycles, minimum policy.
        // Cycle 0: fc = 100_000*2400*30/3_650_000 = 1_972; estimate 101_972; statement 60_000;
        //          unbilled 41_972; minimum max(600, 2500) = 2_500; billed' 57_500.
        // Cycle 1: fc = ADB(57_500 + 41_972/2 = 78_486)*2400*30/3_650_000 = 1_548 —
        //          NOT fc(99_472) = 1_961, which would treat the carry as billed debt.
        let mut cs = cycles(2, 0);
        cs[0].statement_override_cents = Some(60_000);
        let proj = project_revolving(100_000, &cs, &terms(2_400, PaymentPolicy::Minimum));
        assert_eq!(proj[0].finance_charge_cents, 1_972);
        assert_eq!(proj[0].closing_cents, 57_500 + 41_972);
        assert_eq!(proj[1].opening_cents, 99_472);
        assert_eq!(
            proj[1].finance_charge_cents, 1_548,
            "interest accrues on the billed remainder + half the carried un-billed"
        );
    }

    /// The un-billed carry composes with a partial payment: both the unpaid statement
    /// remainder AND the post-close charges roll into the next opening.
    #[test]
    fn override_with_partial_payment_keeps_both_remainder_and_unbilled() {
        // Opening $1,000, no new charges, override $600, minimum policy (1% / $25 floor):
        // payment = max(1% of 600 = 6, 25) = $25; closing = (600 − 25) + (1000 − 600) = $975.
        let mut cs = cycles(1, 0);
        cs[0].statement_override_cents = Some(60_000);
        let proj = project_revolving(100_000, &cs, &terms(0, PaymentPolicy::Minimum));
        assert_eq!(proj[0].payment_cents, 2_500);
        assert_eq!(proj[0].closing_cents, 57_500 + 40_000);
    }

    /// A cycle without an override is byte-identical to the pre-carry behavior: the
    /// un-billed remainder is definitionally zero when statement == estimate.
    #[test]
    fn no_override_closing_is_statement_minus_payment_exactly() {
        let proj = project_revolving(
            50_000,
            &cycles(3, 10_000),
            &terms(0, PaymentPolicy::Fixed(5_000)),
        );
        for c in &proj {
            assert_eq!(
                c.closing_cents,
                (c.statement_balance_cents - c.payment_cents).max(0)
            );
        }
    }

    /// Property (a): a zero balance accrues zero interest every cycle.
    #[test]
    fn zero_balance_accrues_no_interest() {
        let proj = project_revolving(0, &cycles(4, 0), &terms(2_999, PaymentPolicy::Minimum));
        assert!(proj.iter().all(|c| c.finance_charge_cents == 0));
        assert!(proj.iter().all(|c| c.statement_balance_cents == 0));
    }

    /// Property (b): paying the full statement each cycle accrues zero interest (grace).
    #[test]
    fn paid_in_full_each_cycle_accrues_no_interest() {
        // Start clean, charge $200/cycle, pay in full → grace applies, never any interest.
        let proj = project_revolving(
            0,
            &cycles(6, 20_000),
            &terms(2_999, PaymentPolicy::FullStatement),
        );
        assert!(
            proj.iter().all(|c| c.finance_charge_cents == 0),
            "paid-in-full each cycle must accrue zero interest, got {:?}",
            proj.iter()
                .map(|c| c.finance_charge_cents)
                .collect::<Vec<_>>()
        );
        // Each cycle is paid off, so nothing carries.
        assert!(proj.iter().all(|c| c.closing_cents == 0));
    }

    /// Property (c): fixed APR + a fixed partial payment matches a hand-computed table.
    #[test]
    fn fixed_apr_and_partial_payment_matches_a_hand_table() {
        // Opening $1,000, no new charges, 24% APR (2400 bps), 30-day cycles, pay $100/cycle.
        // daily rate = 2400/10000/365; charge = opening * 2400 * 30 / (10000*365).
        // Cycle 1: opening 100_000; interest = 100_000*2400*30/3_650_000 = 7_200_000_000/3_650_000
        //          = 1972 (floor); statement 101_972; pay 10_000; closing 91_972.
        // Cycle 2: opening 91_972; interest = 91_972*2400*30/3_650_000 = 1814; statement 93_786;
        //          pay 10_000; closing 83_786.
        let proj = project_revolving(
            100_000,
            &cycles(2, 0),
            &RevolvingTerms {
                apr_bps: 2_400,
                policy: PaymentPolicy::Fixed(10_000),
                min_percent_bps: 100,
                min_floor_cents: 2_500,
            },
        );
        assert_eq!(proj[0].finance_charge_cents, 1_972);
        assert_eq!(proj[0].statement_balance_cents, 101_972);
        assert_eq!(proj[0].payment_cents, 10_000);
        assert_eq!(proj[0].closing_cents, 91_972);
        assert_eq!(proj[1].opening_cents, 91_972);
        assert_eq!(proj[1].finance_charge_cents, 1_814);
        assert_eq!(proj[1].closing_cents, 83_786);
    }

    /// Property (d): determinism — identical inputs produce identical output.
    #[test]
    fn projection_is_deterministic() {
        let t = terms(2_400, PaymentPolicy::Minimum);
        let a = project_revolving(50_000, &cycles(5, 12_345), &t);
        let b = project_revolving(50_000, &cycles(5, 12_345), &t);
        assert_eq!(a, b);
    }

    /// A carried balance under a partial (minimum) payment grows when interest exceeds the
    /// payment — the compounding the model must capture.
    #[test]
    fn a_carried_balance_compounds_under_minimum_payments() {
        // Opening $5,000, no new charges, 30% APR, pay the 1%/$25 minimum.
        let proj = project_revolving(
            500_000,
            &cycles(3, 0),
            &terms(3_000, PaymentPolicy::Minimum),
        );
        assert!(
            proj[0].finance_charge_cents > 0,
            "interest accrues on the carried balance"
        );
        // Minimum of 1% ($50) barely dents the balance vs ~$123/mo interest, so it grows.
        assert!(
            proj[1].opening_cents > proj[0].opening_cents,
            "a minimum-only payment lets the balance grow: {} -> {}",
            proj[0].opening_cents,
            proj[1].opening_cents
        );
    }
}
