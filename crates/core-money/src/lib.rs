//! Decimal-precise money for DohFlow.
//!
//! Monetary amounts are stored as **integer minor units** (`i64`) tagged with an
//! ISO-4217 [`Currency`] and its `currency_exponent`, per plan §9.1 — never a
//! floating-point type, which would lose precision silently. This crate is the
//! single source of truth for monetary math used by the ledger, forecast,
//! kernel, and importers.
//!
//! Key invariants:
//! - There is **no `Add`/`Sub` operator impl** for [`Money`]. All arithmetic
//!   goes through the [`Money::checked_add`] family, which cannot overflow
//!   silently and rejects mixing currencies. Writing `a + b` for two `Money`
//!   values is a compile error by design.
//! - Cross-currency conversion must go through an explicit [`CurrencyConverter`]
//!   (plan §3.2); this crate provides only the trait and a [`NoOpConverter`] for
//!   tests. Real FX-rate lookup lives elsewhere.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// An ISO-4217 currency.
///
/// `USD` is the initial supported currency; `EUR` is included so the
/// multi-currency machinery (mismatch rejection, conversion) is exercisable.
/// Adding a variant requires extending the exhaustive matches in [`Currency`]
/// **and** the unit test `every_currency_has_consistent_metadata`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Currency {
    /// United States dollar (exponent 2).
    #[serde(rename = "USD")]
    Usd,
    /// Euro (exponent 2).
    #[serde(rename = "EUR")]
    Eur,
}

impl Currency {
    /// The ISO-4217 alphabetic code, e.g. `"USD"`.
    pub const fn code(self) -> &'static str {
        match self {
            Currency::Usd => "USD",
            Currency::Eur => "EUR",
        }
    }

    /// The number of digits after the decimal point (the ISO-4217 minor unit
    /// exponent), e.g. `2` for USD (cents).
    pub const fn exponent(self) -> u8 {
        match self {
            Currency::Usd => 2,
            Currency::Eur => 2,
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// How to resolve a fractional minor unit when dividing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundingMode {
    /// Round half to even ("bankers' rounding"). The default for money.
    HalfEven,
    /// Round half away from zero.
    HalfUp,
    /// Round half toward zero.
    HalfDown,
    /// Round toward positive infinity.
    Ceil,
    /// Round toward negative infinity.
    Floor,
    /// Truncate toward zero.
    TowardZero,
    /// Round away from zero.
    AwayFromZero,
}

/// Errors from monetary operations. Operations return these rather than
/// panicking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MoneyError {
    /// An operation combined two different currencies without conversion.
    #[error("currency mismatch: {left} vs {right}")]
    CurrencyMismatch {
        /// The left-hand currency.
        left: Currency,
        /// The right-hand currency.
        right: Currency,
    },
    /// An arithmetic operation overflowed the `i64` minor-unit range.
    #[error("monetary arithmetic overflow")]
    Overflow,
    /// Division by zero was attempted.
    #[error("division by zero")]
    DivisionByZero,
    /// A `currency_exponent` did not match the currency's canonical exponent.
    #[error("exponent mismatch for {currency}: expected {expected}, found {found}")]
    ExponentMismatch {
        /// The currency whose exponent was wrong.
        currency: Currency,
        /// The canonical exponent for that currency.
        expected: u8,
        /// The exponent that was supplied.
        found: u8,
    },
    /// A converter could not convert between the requested currencies.
    #[error("conversion not supported: {from} -> {to}")]
    ConversionNotSupported {
        /// Source currency.
        from: Currency,
        /// Target currency.
        to: Currency,
    },
}

/// A monetary amount: integer minor units tagged with a currency.
///
/// Construct with [`Money::new`] or [`Money::zero`]; the `currency_exponent` is
/// always kept consistent with the currency. Fields are private to preserve
/// that invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Money {
    minor_units: i64,
    currency: Currency,
    currency_exponent: u8,
}

impl Money {
    /// Create an amount in the given currency. The exponent is taken from the
    /// currency, so it is always consistent.
    pub const fn new(minor_units: i64, currency: Currency) -> Self {
        Money {
            minor_units,
            currency,
            currency_exponent: currency.exponent(),
        }
    }

    /// The additive identity for a currency (zero minor units).
    pub const fn zero(currency: Currency) -> Self {
        Money::new(0, currency)
    }

    /// Reconstruct a `Money` from its stored parts (e.g. from a database row),
    /// validating that `currency_exponent` matches the currency.
    pub fn from_parts(
        minor_units: i64,
        currency: Currency,
        currency_exponent: u8,
    ) -> Result<Self, MoneyError> {
        let expected = currency.exponent();
        if currency_exponent != expected {
            return Err(MoneyError::ExponentMismatch {
                currency,
                expected,
                found: currency_exponent,
            });
        }
        Ok(Money::new(minor_units, currency))
    }

    /// The amount in minor units (e.g. cents for USD).
    pub const fn minor_units(self) -> i64 {
        self.minor_units
    }

    /// The currency tag.
    pub const fn currency(self) -> Currency {
        self.currency
    }

    /// The minor-unit exponent (digits after the decimal point).
    pub const fn currency_exponent(self) -> u8 {
        self.currency_exponent
    }

    /// Whether the amount is exactly zero.
    pub const fn is_zero(self) -> bool {
        self.minor_units == 0
    }

    fn require_same_currency(self, rhs: Money) -> Result<(), MoneyError> {
        if self.currency == rhs.currency {
            Ok(())
        } else {
            Err(MoneyError::CurrencyMismatch {
                left: self.currency,
                right: rhs.currency,
            })
        }
    }

    /// Add two amounts of the same currency. Errors on currency mismatch or
    /// overflow.
    pub fn checked_add(self, rhs: Money) -> Result<Money, MoneyError> {
        self.require_same_currency(rhs)?;
        let sum = self
            .minor_units
            .checked_add(rhs.minor_units)
            .ok_or(MoneyError::Overflow)?;
        Ok(Money::new(sum, self.currency))
    }

    /// Subtract an amount of the same currency. Errors on currency mismatch or
    /// overflow.
    pub fn checked_sub(self, rhs: Money) -> Result<Money, MoneyError> {
        self.require_same_currency(rhs)?;
        let diff = self
            .minor_units
            .checked_sub(rhs.minor_units)
            .ok_or(MoneyError::Overflow)?;
        Ok(Money::new(diff, self.currency))
    }

    /// Negate the amount. Errors only on overflow (`i64::MIN`).
    pub fn checked_neg(self) -> Result<Money, MoneyError> {
        let neg = self.minor_units.checked_neg().ok_or(MoneyError::Overflow)?;
        Ok(Money::new(neg, self.currency))
    }

    /// Multiply by an integer factor (e.g. a quantity). Errors on overflow.
    pub fn checked_mul_int(self, factor: i64) -> Result<Money, MoneyError> {
        let product = self
            .minor_units
            .checked_mul(factor)
            .ok_or(MoneyError::Overflow)?;
        Ok(Money::new(product, self.currency))
    }

    /// Divide by a nonzero integer divisor, resolving the fractional minor unit
    /// with `mode`. Use [`RoundingMode::HalfEven`] for money unless a specific
    /// policy says otherwise.
    ///
    /// Errors on division by zero or on overflow of the result.
    pub fn checked_div_int(self, divisor: i64, mode: RoundingMode) -> Result<Money, MoneyError> {
        if divisor == 0 {
            return Err(MoneyError::DivisionByZero);
        }

        // Work in i128/u128 so neither the doubled remainder nor the sign flip
        // can overflow; narrow back to i64 at the end (checked).
        let n = i128::from(self.minor_units);
        let d = i128::from(divisor);

        let negative = (n < 0) ^ (d < 0);
        let n_abs = n.unsigned_abs();
        let d_abs = d.unsigned_abs();

        let quotient = n_abs / d_abs;
        let remainder = n_abs % d_abs;

        let round_up = if remainder == 0 {
            false
        } else {
            let twice = remainder * 2;
            match mode {
                RoundingMode::TowardZero => false,
                RoundingMode::AwayFromZero => true,
                RoundingMode::Floor => negative,
                RoundingMode::Ceil => !negative,
                RoundingMode::HalfUp => twice >= d_abs,
                RoundingMode::HalfDown => twice > d_abs,
                RoundingMode::HalfEven => match twice.cmp(&d_abs) {
                    std::cmp::Ordering::Greater => true,
                    std::cmp::Ordering::Less => false,
                    // Exactly half: round to the even neighbour.
                    std::cmp::Ordering::Equal => quotient % 2 == 1,
                },
            }
        };

        let magnitude = if round_up { quotient + 1 } else { quotient };
        let magnitude = i128::try_from(magnitude).map_err(|_| MoneyError::Overflow)?;
        let signed = if negative { -magnitude } else { magnitude };
        let result = i64::try_from(signed).map_err(|_| MoneyError::Overflow)?;

        Ok(Money::new(result, self.currency))
    }
}

impl fmt::Display for Money {
    /// Renders the amount in plain decimal with the currency code, e.g.
    /// `USD 1234.50`. Per-currency symbols/locale formatting are a separate
    /// concern (see bead personal-cfo-nac8). Computed with integer math only.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let exponent = u32::from(self.currency_exponent);
        if exponent == 0 {
            return write!(f, "{} {}", self.currency.code(), self.minor_units);
        }
        let scale = 10i64.pow(exponent);
        let sign = if self.minor_units < 0 { "-" } else { "" };
        let abs = self.minor_units.unsigned_abs();
        let scale_u = scale.unsigned_abs();
        let whole = abs / scale_u;
        let frac = abs % scale_u;
        write!(
            f,
            "{} {}{}.{:0width$}",
            self.currency.code(),
            sign,
            whole,
            frac,
            width = exponent as usize
        )
    }
}

/// Converts amounts between currencies.
///
/// The real implementation (FX-rate lookup) lives outside this crate; here we
/// define only the contract plus [`NoOpConverter`] for tests.
pub trait CurrencyConverter {
    /// Convert `amount` into `to`. Implementations must reject conversions they
    /// cannot perform rather than guessing a rate.
    fn convert(&self, amount: Money, to: Currency) -> Result<Money, MoneyError>;
}

/// A converter that performs no conversion: it returns the amount unchanged
/// when it is already in the target currency, and errors otherwise. Useful in
/// tests and single-currency contexts.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOpConverter;

impl CurrencyConverter for NoOpConverter {
    fn convert(&self, amount: Money, to: Currency) -> Result<Money, MoneyError> {
        if amount.currency() == to {
            Ok(amount)
        } else {
            Err(MoneyError::ConversionNotSupported {
                from: amount.currency(),
                to,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Forces an "associated test entry" for every Currency variant: this match
    // is exhaustive (same-crate, so #[non_exhaustive] does not relax it), so
    // adding a variant fails to compile until handled here.
    fn assert_currency_metadata(currency: Currency) {
        match currency {
            Currency::Usd => {
                assert_eq!(currency.code(), "USD");
                assert_eq!(currency.exponent(), 2);
            }
            Currency::Eur => {
                assert_eq!(currency.code(), "EUR");
                assert_eq!(currency.exponent(), 2);
            }
        }
    }

    #[test]
    fn every_currency_has_consistent_metadata() {
        for &currency in &[Currency::Usd, Currency::Eur] {
            assert_currency_metadata(currency);
            // Round-trip the exponent through from_parts.
            let m = Money::new(100, currency);
            assert_eq!(m.currency_exponent(), currency.exponent());
        }
    }

    #[test]
    fn from_parts_rejects_wrong_exponent() {
        assert_eq!(
            Money::from_parts(100, Currency::Usd, 3),
            Err(MoneyError::ExponentMismatch {
                currency: Currency::Usd,
                expected: 2,
                found: 3,
            })
        );
        assert!(Money::from_parts(100, Currency::Usd, 2).is_ok());
    }

    #[test]
    fn cross_currency_add_is_rejected() {
        let usd = Money::new(100, Currency::Usd);
        let eur = Money::new(100, Currency::Eur);
        assert_eq!(
            usd.checked_add(eur),
            Err(MoneyError::CurrencyMismatch {
                left: Currency::Usd,
                right: Currency::Eur,
            })
        );
    }

    #[test]
    fn overflow_returns_err_not_panic() {
        let max = Money::new(i64::MAX, Currency::Usd);
        let one = Money::new(1, Currency::Usd);
        assert_eq!(max.checked_add(one), Err(MoneyError::Overflow));
        assert_eq!(
            Money::new(i64::MIN, Currency::Usd).checked_neg(),
            Err(MoneyError::Overflow)
        );
        assert_eq!(
            Money::new(i64::MAX, Currency::Usd).checked_mul_int(2),
            Err(MoneyError::Overflow)
        );
    }

    #[test]
    fn div_by_zero_returns_err() {
        assert_eq!(
            Money::new(100, Currency::Usd).checked_div_int(0, RoundingMode::HalfEven),
            Err(MoneyError::DivisionByZero)
        );
    }

    #[test]
    fn half_even_rounds_to_even_neighbour() {
        // 2.5 -> 2, 3.5 -> 4, -2.5 -> -2, -3.5 -> -4 (in minor units).
        let div = |n: i64| {
            Money::new(n, Currency::Usd)
                .checked_div_int(2, RoundingMode::HalfEven)
                .unwrap()
                .minor_units()
        };
        assert_eq!(div(5), 2); // 2.5 -> 2 (even)
        assert_eq!(div(7), 4); // 3.5 -> 4 (even)
        assert_eq!(div(-5), -2);
        assert_eq!(div(-7), -4);
        // Non-half cases round to nearest.
        assert_eq!(div(3), 2); // 1.5 -> 2
        assert_eq!(div(1), 0); // 0.5 -> 0 (even)
    }

    #[test]
    fn rounding_modes_behave() {
        let m = |n: i64| Money::new(n, Currency::Usd);
        // 7/2 = 3.5
        assert_eq!(
            m(7).checked_div_int(2, RoundingMode::HalfUp)
                .unwrap()
                .minor_units(),
            4
        );
        assert_eq!(
            m(7).checked_div_int(2, RoundingMode::HalfDown)
                .unwrap()
                .minor_units(),
            3
        );
        assert_eq!(
            m(7).checked_div_int(2, RoundingMode::Floor)
                .unwrap()
                .minor_units(),
            3
        );
        assert_eq!(
            m(7).checked_div_int(2, RoundingMode::Ceil)
                .unwrap()
                .minor_units(),
            4
        );
        assert_eq!(
            m(7).checked_div_int(2, RoundingMode::TowardZero)
                .unwrap()
                .minor_units(),
            3
        );
        assert_eq!(
            m(7).checked_div_int(2, RoundingMode::AwayFromZero)
                .unwrap()
                .minor_units(),
            4
        );
        // -7/2 = -3.5
        assert_eq!(
            m(-7)
                .checked_div_int(2, RoundingMode::Floor)
                .unwrap()
                .minor_units(),
            -4
        );
        assert_eq!(
            m(-7)
                .checked_div_int(2, RoundingMode::Ceil)
                .unwrap()
                .minor_units(),
            -3
        );
        assert_eq!(
            m(-7)
                .checked_div_int(2, RoundingMode::TowardZero)
                .unwrap()
                .minor_units(),
            -3
        );
    }

    #[test]
    fn display_uses_integer_math() {
        assert_eq!(
            Money::new(123_450, Currency::Usd).to_string(),
            "USD 1234.50"
        );
        assert_eq!(Money::new(-5, Currency::Usd).to_string(), "USD -0.05");
        assert_eq!(Money::new(0, Currency::Eur).to_string(), "EUR 0.00");
    }

    #[test]
    fn noop_converter_only_passes_same_currency() {
        let c = NoOpConverter;
        let usd = Money::new(100, Currency::Usd);
        assert_eq!(c.convert(usd, Currency::Usd), Ok(usd));
        assert_eq!(
            c.convert(usd, Currency::Eur),
            Err(MoneyError::ConversionNotSupported {
                from: Currency::Usd,
                to: Currency::Eur,
            })
        );
    }
}
