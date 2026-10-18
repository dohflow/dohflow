//! The connector currency guard (personal-cfo-049p6; ADR 0076 decision 7;
//! plan R38).
//!
//! Until multi-currency ships (Milestone M-Multicurrency: `personal-cfo-d63`,
//! `-rlx`, `-k2u3`, `-il6n`), a provider account can only be mapped when its
//! currency is KNOWN and EQUAL to the household's base (reporting) currency.
//! Anything else would put money the app cannot sum into net worth, the
//! dashboard and Future Cash. There is deliberately no fallback: an unknown
//! currency is never assumed to be the base currency (owner decision
//! 2026-09-28).
//!
//! This module is the ONE place the rule lives. [`crate::Kernel`] enforces
//! it on every mapping, and the IPC layer shows its message on the
//! connection's account rows. The lift bead (`personal-cfo-naerm`) removes
//! it here once foreign-currency ledger accounts are supported.

/// Why a provider account can't be mapped (yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectorCurrencyRefusal {
    /// The provider hasn't reported this account's currency (a link saved
    /// before currencies were recorded, or a provider that stated none).
    Unknown,
    /// The account is in a currency other than the household's.
    Foreign {
        account_currency: String,
        base_currency: String,
    },
}

impl std::fmt::Display for ConnectorCurrencyRefusal {
    /// Plain, present-tense copy naming both currencies; no dates or
    /// promises.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => f.write_str(
                "The provider hasn't reported this account's currency yet. Refresh the \
                 connection, then map it.",
            ),
            Self::Foreign {
                account_currency,
                base_currency,
            } => write!(
                f,
                "This account is in {account_currency}, but your base currency is \
                 {base_currency}. Accounts in a currency other than your base currency \
                 aren't supported yet."
            ),
        }
    }
}

/// Whether a provider account with `account_currency` may be mapped in a
/// household whose base currency is `base_currency`. Codes compare
/// case-insensitively.
///
/// # Errors
/// [`ConnectorCurrencyRefusal`] when the currency is unknown or differs.
pub fn connector_currency_guard(
    account_currency: Option<&str>,
    base_currency: &str,
) -> Result<(), ConnectorCurrencyRefusal> {
    let base = base_currency.trim().to_ascii_uppercase();
    match account_currency.map(|code| code.trim().to_ascii_uppercase()) {
        None => Err(ConnectorCurrencyRefusal::Unknown),
        Some(code) if code.is_empty() => Err(ConnectorCurrencyRefusal::Unknown),
        Some(code) if code == base => Ok(()),
        Some(code) => Err(ConnectorCurrencyRefusal::Foreign {
            account_currency: code,
            base_currency: base,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_currency_maps_and_anything_else_is_refused() {
        assert_eq!(connector_currency_guard(Some("USD"), "USD"), Ok(()));
        assert_eq!(connector_currency_guard(Some("usd"), "USD"), Ok(()));
        assert_eq!(
            connector_currency_guard(Some("EUR"), "USD"),
            Err(ConnectorCurrencyRefusal::Foreign {
                account_currency: "EUR".to_owned(),
                base_currency: "USD".to_owned(),
            })
        );
        assert_eq!(
            connector_currency_guard(None, "USD"),
            Err(ConnectorCurrencyRefusal::Unknown)
        );
        assert_eq!(
            connector_currency_guard(Some(" "), "USD"),
            Err(ConnectorCurrencyRefusal::Unknown)
        );
    }

    #[test]
    fn the_refusal_copy_names_both_currencies_without_promises() {
        let copy = connector_currency_guard(Some("GBP"), "USD")
            .unwrap_err()
            .to_string();
        assert!(copy.contains("GBP") && copy.contains("USD"), "{copy}");
        assert!(copy.contains("aren't supported yet"), "{copy}");
        for promise in ["soon", "will ", "coming", "2026", "2027"] {
            assert!(!copy.contains(promise), "{copy}");
        }
    }
}
