//! Ordinary vs extraordinary spend classification (ADR 0038, personal-cfo-pezm.1).
//!
//! Pure + deterministic, like [`crate::normalize_merchant`]: identical inputs produce
//! byte-identical labels (sorted inputs, integer-cent math; no IO / clock / RNG). Layer-2 of
//! the forecast feeds the **ordinary** postings into its spend band so a one-off vacation
//! does not teach the model that the household vacations every month (ADR 0026 §7).
//!
//! The v1 rule (ADR 0038 §2): a posting is *extraordinary* when its amount clears its
//! category's robust upper fence (`median + K·MAD`), is at least `RATIO`× the category
//! median, and is at least an absolute floor — unless its merchant recurs in the window, in
//! which case it is ordinary regardless of amount. Conservative by design (precision over
//! recall): wrongly excluding ordinary spend would understate the baseline.

/// MAD multiplier for the robust upper fence (`median + K·MAD`). Conservative (≈3.4σ for a
/// normal sample once the MAD→σ consistency factor is folded in) so only clear outliers
/// flag — v1 favours precision over recall (ADR 0038 §2).
const MAD_K: i64 = 5;

/// An extraordinary charge must also be at least this multiple of the category median,
/// expressed as a rational to stay in integer math (`amount·RATIO_DEN ≥ median·RATIO_NUM`).
/// Guards a tightly-clustered category (MAD ≈ 0) from flagging charges only modestly above
/// median. 5/2 = 2.5×.
const RATIO_NUM: i64 = 5;
const RATIO_DEN: i64 = 2;

/// Below this absolute magnitude (minor units) nothing is extraordinary — keeps a cheap
/// category's noise out of the label. $200.
const ABSOLUTE_FLOOR_CENTS: i64 = 20_000;

/// A merchant seen at least this many times in the window is *recurring* → its postings are
/// ordinary regardless of amount (a regular large grocery run / monthly bill is baseline,
/// not a surprise).
pub const RECUR_MIN: u32 = 3;

/// The classification of a single spend posting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendClass {
    /// Recurring / baseline behaviour — feeds the Layer-2 spend band.
    Ordinary,
    /// A one-off (travel, a large purchase) — kept out of the baseline.
    Extraordinary,
}

/// Why a posting was classified the way it was — a machine token for inspection, never
/// advice (ADR 0018).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassReason {
    /// Ordinary: the posting's merchant recurs in the window (the recurring-merchant guard).
    RecurringMerchant,
    /// Ordinary: amount is below the absolute floor.
    BelowFloor,
    /// Ordinary: amount sits within the category's normal range.
    WithinCategoryRange,
    /// Extraordinary: a robust amount outlier for its category.
    AmountOutlier,
}

impl ClassReason {
    /// The stable string token for this reason.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RecurringMerchant => "recurring_merchant",
            Self::BelowFloor => "below_floor",
            Self::WithinCategoryRange => "within_category_range",
            Self::AmountOutlier => "amount_outlier",
        }
    }
}

/// A per-posting classification: the label, a `0..=10_000` confidence, and the reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classification {
    /// Ordinary or extraordinary.
    pub class: SpendClass,
    /// Confidence in `0..=10_000` (basis points), monotone in distance from the fence.
    pub confidence_bps: i64,
    /// The deciding rule.
    pub reason: ClassReason,
}

/// A category's spend distribution over the window — its robust centre (`median`) and
/// spread (`mad` = median absolute deviation). Build once per category, then classify each
/// of its postings against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryProfile {
    median: i64,
    mad: i64,
}

impl CategoryProfile {
    /// Build the profile from a category's posting magnitudes (positive minor units).
    /// Order-independent: the input is copied and sorted.
    #[must_use]
    pub fn from_amounts(amounts: &[i64]) -> Self {
        let median = median_of(&mut amounts.to_vec());
        let mut deviations: Vec<i64> = amounts.iter().map(|a| (a - median).abs()).collect();
        let mad = median_of(&mut deviations);
        Self { median, mad }
    }

    /// The robust upper fence: amounts strictly above this are amount-outliers.
    #[must_use]
    pub fn extraordinary_fence(&self) -> i64 {
        self.median.saturating_add(MAD_K.saturating_mul(self.mad))
    }
}

/// Classify one spend posting (magnitude in positive minor units) against its category
/// profile, given how many times its merchant recurs in the window (`0` when unknown — the
/// v1 production case, ADR 0038 §2). See the module docs for the rule.
#[must_use]
pub fn classify(
    amount_cents: i64,
    merchant_occurrences: u32,
    profile: &CategoryProfile,
) -> Classification {
    // Recurring merchant → baseline, regardless of amount.
    if merchant_occurrences >= RECUR_MIN {
        return ordinary(ClassReason::RecurringMerchant, 10_000);
    }
    // Too small to ever be extraordinary.
    if amount_cents < ABSOLUTE_FLOOR_CENTS {
        return ordinary(ClassReason::BelowFloor, 10_000);
    }
    let fence = profile.extraordinary_fence();
    let ratio_ok =
        amount_cents.saturating_mul(RATIO_DEN) >= profile.median.saturating_mul(RATIO_NUM);
    if amount_cents > fence && ratio_ok {
        // Confidence grows with the distance above the fence, saturating at 2× fence.
        let over = amount_cents - fence;
        let confidence = (over.saturating_mul(10_000) / fence.max(1)).clamp(0, 10_000);
        Classification {
            class: SpendClass::Extraordinary,
            confidence_bps: confidence,
            reason: ClassReason::AmountOutlier,
        }
    } else {
        // Ordinary: confidence grows the further below the fence it sits.
        let under = (fence - amount_cents).max(0);
        let confidence = (under.saturating_mul(10_000) / fence.max(1)).clamp(0, 10_000);
        ordinary(ClassReason::WithinCategoryRange, confidence)
    }
}

fn ordinary(reason: ClassReason, confidence_bps: i64) -> Classification {
    Classification {
        class: SpendClass::Ordinary,
        confidence_bps,
        reason,
    }
}

/// Median of a slice (sorted in place). Even length → integer mean of the two central
/// values. Empty → 0.
fn median_of(values: &mut [i64]) -> i64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canonical month: stable restaurant/grocery spend plus one big vacation charge. The
    /// vacation is the only extraordinary posting; everything else is ordinary. (The
    /// deterministic equivalent of an insta snapshot — exact labels asserted.)
    #[test]
    fn vacation_charge_is_the_only_extraordinary_posting() {
        // Magnitudes in cents: a month of *above-floor* ordinary spend (~$380–$620 grocery
        // runs, so each is judged by the category band rather than short-circuited by the
        // absolute floor), then a $3,000 vacation.
        let amounts: Vec<i64> = vec![
            42_000, 38_000, 51_000, 45_000, 39_000, 62_000, 48_000, 54_000, 41_000, 47_000, 300_000,
        ];
        let profile = CategoryProfile::from_amounts(&amounts);
        let labels: Vec<Classification> =
            amounts.iter().map(|&a| classify(a, 0, &profile)).collect();

        // Exactly one extraordinary, and it is the vacation.
        let extraordinary: Vec<i64> = amounts
            .iter()
            .zip(&labels)
            .filter(|(_, c)| c.class == SpendClass::Extraordinary)
            .map(|(a, _)| *a)
            .collect();
        assert_eq!(extraordinary, vec![300_000]);
        assert_eq!(
            classify(300_000, 0, &profile).reason,
            ClassReason::AmountOutlier
        );
        // The largest ordinary charge is spared by the category band (it is above the floor),
        // not by the floor short-circuit — so the "only extraordinary" assertion is
        // load-bearing on the fence/ratio logic, not on the floor.
        let largest_ordinary = classify(62_000, 0, &profile);
        assert_eq!(largest_ordinary.class, SpendClass::Ordinary);
        assert_eq!(largest_ordinary.reason, ClassReason::WithinCategoryRange);
    }

    /// The core property (pezm.1 AC): removing the extraordinary postings reduces the
    /// month-over-month variance of the category's monthly totals. Deterministic fixture —
    /// six months of steady ordinary spend, two of them carrying a one-off spike.
    #[test]
    fn removing_extraordinary_reduces_monthly_total_variance() {
        // Six months; each a handful of ordinary charges. Months 3 and 5 add a big spike.
        let ordinary_month: [i64; 5] = [4_000, 4_500, 3_800, 4_200, 4_100];
        let spike: i64 = 280_000;
        let mut months: Vec<Vec<i64>> = (0..6).map(|_| ordinary_month.to_vec()).collect();
        months[2].push(spike);
        months[4].push(spike);

        let all: Vec<i64> = months.iter().flatten().copied().collect();
        let profile = CategoryProfile::from_amounts(&all);

        // Monthly totals with everything vs after dropping classified-extraordinary.
        let total = |m: &[i64]| m.iter().sum::<i64>() as i128;
        let kept_total = |m: &[i64]| {
            m.iter()
                .filter(|&&a| classify(a, 0, &profile).class == SpendClass::Ordinary)
                .sum::<i64>() as i128
        };
        let with: Vec<i128> = months.iter().map(|m| total(m)).collect();
        let without: Vec<i128> = months.iter().map(|m| kept_total(m)).collect();

        assert!(
            variance_times_n2(&without) < variance_times_n2(&with),
            "dropping extraordinary should shrink monthly-total variance: with={with:?} without={without:?}"
        );
    }

    /// `n²·variance` as an exact integer (`n·Σx² − (Σx)²`) — avoids fractional means while
    /// preserving the ordering of variances for equal-length samples.
    fn variance_times_n2(xs: &[i128]) -> i128 {
        let n = xs.len() as i128;
        let sum: i128 = xs.iter().sum();
        let sum_sq: i128 = xs.iter().map(|x| x * x).sum();
        n * sum_sq - sum * sum
    }

    #[test]
    fn classification_is_order_independent_and_deterministic() {
        let a: Vec<i64> = vec![5_000, 4_000, 6_000, 4_500, 250_000, 3_800];
        let mut b = a.clone();
        b.reverse();
        let pa = CategoryProfile::from_amounts(&a);
        let pb = CategoryProfile::from_amounts(&b);
        assert_eq!(pa, pb, "profile must not depend on input order");
        for amt in [3_800, 5_000, 250_000_i64] {
            assert_eq!(classify(amt, 0, &pa), classify(amt, 0, &pb));
        }
    }

    #[test]
    fn recurring_merchant_is_ordinary_even_when_large() {
        // A merchant seen RECUR_MIN+ times: a large monthly charge is baseline, not a spike.
        let amounts: Vec<i64> = vec![4_000, 4_500, 3_800, 4_200];
        let profile = CategoryProfile::from_amounts(&amounts);
        let c = classify(300_000, RECUR_MIN, &profile);
        assert_eq!(c.class, SpendClass::Ordinary);
        assert_eq!(c.reason, ClassReason::RecurringMerchant);
    }

    #[test]
    fn below_the_floor_is_never_extraordinary() {
        // A category whose spend is tiny: even a relative spike under the floor stays ordinary.
        let amounts: Vec<i64> = vec![300, 250, 280, 100, 19_999];
        let profile = CategoryProfile::from_amounts(&amounts);
        let c = classify(19_999, 0, &profile);
        assert_eq!(c.class, SpendClass::Ordinary);
        assert_eq!(c.reason, ClassReason::BelowFloor);
    }

    #[test]
    fn a_tightly_clustered_category_has_no_false_positives() {
        // MAD ≈ 0 but the ratio guard keeps modestly-above-median charges ordinary.
        let amounts: Vec<i64> = vec![40_000, 41_000, 39_000, 40_500, 40_200, 50_000];
        let profile = CategoryProfile::from_amounts(&amounts);
        for &a in &amounts {
            assert_eq!(
                classify(a, 0, &profile).class,
                SpendClass::Ordinary,
                "amount {a} should be ordinary"
            );
        }
    }

    #[test]
    fn fence_is_median_plus_k_mad() {
        // amounts: median 100, deviations [50,30,0,30,50] → MAD 30 → fence 100 + 5·30 = 250.
        let profile = CategoryProfile::from_amounts(&[50, 70, 100, 130, 150]);
        assert_eq!(profile.extraordinary_fence(), 250);
    }
}
