//! Property tests for `core-money` (plan DoD §1.5).
//!
//! Covers: addition associativity/commutativity, the additive identity,
//! lossless serde round-trips, and half-even ("bankers'") division correctness
//! for division by integer ratios.

use core_money::{Currency, Money, MoneyError, RoundingMode};
use proptest::prelude::*;

// Bound so that summing three values cannot overflow i64 (3e12 << i64::MAX).
const BOUND: i64 = 1_000_000_000_000;

proptest! {
    #[test]
    fn add_is_associative(a in -BOUND..BOUND, b in -BOUND..BOUND, c in -BOUND..BOUND) {
        let a = Money::new(a, Currency::Usd);
        let b = Money::new(b, Currency::Usd);
        let c = Money::new(c, Currency::Usd);
        let left = a.checked_add(b).unwrap().checked_add(c).unwrap();
        let right = a.checked_add(b.checked_add(c).unwrap()).unwrap();
        prop_assert_eq!(left, right);
    }

    #[test]
    fn add_is_commutative(a in -BOUND..BOUND, b in -BOUND..BOUND) {
        let a = Money::new(a, Currency::Usd);
        let b = Money::new(b, Currency::Usd);
        prop_assert_eq!(a.checked_add(b).unwrap(), b.checked_add(a).unwrap());
    }

    #[test]
    fn zero_is_the_additive_identity(units in any::<i64>()) {
        let m = Money::new(units, Currency::Usd);
        let zero = Money::zero(Currency::Usd);
        prop_assert_eq!(m.checked_add(zero).unwrap(), m);
        prop_assert_eq!(zero.checked_add(m).unwrap(), m);
    }

    #[test]
    fn serde_round_trip_is_lossless(units in any::<i64>()) {
        for currency in [Currency::Usd, Currency::Eur] {
            let m = Money::new(units, currency);
            let json = serde_json::to_string(&m).unwrap();
            let back: Money = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(m, back);
        }
    }

    /// Half-even division is the correctly-rounded nearest minor unit: the
    /// result is within half a unit of the exact quotient, and exact when the
    /// amount divides evenly.
    #[test]
    fn half_even_div_is_nearest_and_exact(
        n in (i64::MIN + 1)..=i64::MAX,
        d in -1000i64..=1000,
    ) {
        prop_assume!(d != 0);
        let result = Money::new(n, Currency::Usd)
            .checked_div_int(d, RoundingMode::HalfEven)
            .unwrap();

        let r = i128::from(result.minor_units());
        let n = i128::from(n);
        let d = i128::from(d);

        // |r - n/d| <= 1/2  <=>  |r*d - n| * 2 <= |d|
        prop_assert!((r * d - n).abs() * 2 <= d.abs());
        // Exact division loses nothing.
        if n % d == 0 {
            prop_assert_eq!(r * d, n);
        }
    }

    /// On an exact half (remainder == divisor/2), half-even rounds to the even
    /// neighbour.
    #[test]
    fn half_even_ties_round_to_even(
        k in -1_000_000_000i64..1_000_000_000,
        half_d in 1i64..=1000,
    ) {
        let d = half_d * 2; // even divisor, so half_d is exactly d/2
        let n = k * d + half_d; // n/d == k + 0.5 exactly
        let r = Money::new(n, Currency::Usd)
            .checked_div_int(d, RoundingMode::HalfEven)
            .unwrap()
            .minor_units();
        prop_assert_eq!(r % 2, 0);
    }

    /// Mixing currencies through the checked API always errors, never silently
    /// produces a value.
    #[test]
    fn cross_currency_add_always_errors(a in any::<i64>(), b in any::<i64>()) {
        let usd = Money::new(a, Currency::Usd);
        let eur = Money::new(b, Currency::Eur);
        prop_assert_eq!(
            usd.checked_add(eur),
            Err(MoneyError::CurrencyMismatch {
                left: Currency::Usd,
                right: Currency::Eur,
            })
        );
    }
}
