//! personal-cfo-x6dr AC: a capability mismatch surfaces a typed
//! `CapabilityMissing` error, never a runtime crash.

use chrono::NaiveDate;
use connector_core::mock::MockConnector;
use connector_core::{
    Capability, CapabilitySet, Connection, ConnectorAdapter, ConnectorError, Credential,
};

fn conn() -> Connection {
    Connection {
        credential: Credential::new("mock-access-url"),
    }
}

#[test]
fn feature_code_checks_capabilities_up_front() {
    let accounts_only = MockConnector::with_capabilities(CapabilitySet {
        accounts: true,
        ..CapabilitySet::default()
    });
    let caps = accounts_only.capabilities();
    assert!(caps.ensure(Capability::Accounts).is_ok());
    for missing in [
        Capability::Transactions,
        Capability::Balances,
        Capability::Holdings,
        Capability::Liabilities,
    ] {
        let err = caps.ensure(missing).unwrap_err();
        assert!(
            matches!(err, ConnectorError::CapabilityMissing(c) if c == missing),
            "expected CapabilityMissing({missing:?}), got {err:?}"
        );
    }
}

#[test]
fn fetch_against_a_missing_capability_is_a_typed_error() {
    let accounts_only = MockConnector::with_capabilities(CapabilitySet {
        accounts: true,
        ..CapabilitySet::default()
    });
    let since = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let err = accounts_only
        .fetch_transactions(&conn(), "mock-acct-checking", Some(since))
        .unwrap_err();
    assert!(matches!(
        err,
        ConnectorError::CapabilityMissing(Capability::Transactions)
    ));

    let err = accounts_only
        .fetch_balances(&conn(), "mock-acct-checking")
        .unwrap_err();
    assert!(matches!(
        err,
        ConnectorError::CapabilityMissing(Capability::Balances)
    ));
}

#[test]
fn default_sync_skips_fetches_the_adapter_cannot_do() {
    // accounts + transactions, no balances: sync must succeed and simply not
    // produce balance records — a mismatch inside the default composition is
    // handled, not crashed on.
    let no_balances = MockConnector::with_capabilities(CapabilitySet {
        accounts: true,
        transactions: true,
        ..CapabilitySet::default()
    });
    let since = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let synced = no_balances.sync(&conn(), Some(since)).unwrap();
    assert!(synced.batch.records.iter().any(|r| r.transaction.is_some()));
    assert!(synced.batch.records.iter().all(|r| r.balance.is_none()));
}
