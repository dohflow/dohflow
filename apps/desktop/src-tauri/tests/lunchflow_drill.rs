//! The LunchFlow live drill (personal-cfo-r2pow AC6, mirroring
//! `simplefin_drill.rs`): a REAL end-to-end run against the owner's LunchFlow
//! account — link, discovery, one full refresh through staged ingestion into
//! a real temp vault, committed via the Money Inbox pipeline.
//!
//! Network test, `#[ignore]`d and gated on a real key, so the default suite
//! stays deterministic and never needs a secret. Run deliberately, with the
//! key typed at a hidden prompt so it never lands in shell history:
//!
//! ```sh
//! read -rs LUNCHFLOW_DRILL_KEY && export LUNCHFLOW_DRILL_KEY
//! cargo test --test lunchflow_drill -- --ignored --nocapture
//! unset LUNCHFLOW_DRILL_KEY
//! ```
//!
//! It links through `connector_link_impl`, which resolves no registry entry,
//! because the registered LunchFlow entry is disabled until its release
//! (`connector_link` would refuse it — asserted in `connector_ipc.rs`). The
//! drill prints counts, currencies and dates only: never the key, an amount,
//! a merchant, or an account name.

use std::collections::BTreeSet;

use app_lib::ipc::commands::{
    connector_connections_impl, connector_link_impl, connector_set_account_link_impl,
    connector_sync_impl, create_account_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, CashflowRoleDto, ConnectorLinkInput, ConnectorSetAccountLinkInput,
    ConnectorSyncInput, CreateAccountInput,
};
use app_lib::AppState;
use finance_kernel::VaultController;
use tempfile::TempDir;

#[test]
#[ignore = "network: drives the owner's live LunchFlow account — set LUNCHFLOW_DRILL_KEY and run with --ignored"]
fn live_key_end_to_end_refresh_stages_and_commits() {
    let Ok(key) = std::env::var("LUNCHFLOW_DRILL_KEY") else {
        println!("LUNCHFLOW_DRILL_KEY is not set — drill skipped");
        return;
    };
    let dir = TempDir::new().expect("temp dir");
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(b"drill-key").expect("create vault");
    let state = AppState::new(controller);
    let adapter =
        connector_core::connector_by_id("lunchflow").expect("lunchflow adapter registered");

    // Link: one validating request, the key stored as the connection secret.
    let linked = connector_link_impl(
        &state,
        adapter,
        ConnectorLinkInput {
            adapter_id: "lunchflow".to_owned(),
            setup_token: key.clone(),
        },
    )
    .expect("LunchFlow accepted the key");
    println!(
        "linked: {} account(s) discovered, discovery error: {}",
        linked.accounts.len(),
        linked.fetch_error.is_some()
    );
    assert!(
        !linked.accounts.is_empty(),
        "the account exposes at least one account"
    );

    // Discovery detail: currencies, and which accounts the app can hold today.
    let credential = connector_core::Credential::new(key);
    let connection = connector_core::Connection { credential };
    let discovered = adapter
        .fetch_accounts(&connection)
        .expect("accounts reachable");
    let currencies: BTreeSet<String> = discovered
        .iter()
        .map(|a| a.currency.clone().unwrap_or_else(|| "(none)".to_owned()))
        .collect();
    println!("account currencies: {currencies:?}");

    // History depth: the earliest and latest posted date per account on a
    // first (full-history) fetch.
    for account in &discovered {
        let id = account.external_id.as_deref().expect("stable account id");
        let records = adapter
            .fetch_transactions(&connection, id, None)
            .expect("transactions reachable");
        let dates: Vec<_> = records
            .iter()
            .filter_map(|r| r.transaction.as_ref().map(|t| t.posted_date))
            .collect();
        let negatives = records
            .iter()
            .filter_map(|r| r.transaction.as_ref())
            .filter(|t| t.amount.minor_units() < 0)
            .count();
        println!(
            "account {id}: {} transaction(s), {negatives} negative, earliest {:?}, latest {:?}",
            dates.len(),
            dates.iter().min(),
            dates.iter().max()
        );
    }

    // Map every USD account onto a fresh real account (other currencies are
    // refused at mapping once personal-cfo-049p6 ships; skip them here).
    let mut mapped = 0;
    for account in discovered
        .iter()
        .filter(|a| a.currency.as_deref() == Some("USD"))
    {
        let real = create_account_impl(
            &state,
            CreateAccountInput {
                name: format!("Drill account {mapped}"),
                cashflow_role: CashflowRoleDto::LiquidCash,
                currency: "USD".to_owned(),
                flags: Some(AccountFlagsDto {
                    retirement: false,
                    tax_advantaged: false,
                    joint: false,
                    business: false,
                }),
                opening_balance: None,
                subtype: None,
                idempotency_key: String::new(),
            },
        )
        .unwrap()
        .account_id;
        connector_set_account_link_impl(
            &state,
            ConnectorSetAccountLinkInput {
                connection_id: linked.connection_id.clone(),
                external_id: account.external_id.clone().unwrap(),
                account_id: Some(real),
                allow_shared_feed: false,
            },
        )
        .unwrap();
        mapped += 1;
    }
    println!("mapped {mapped} USD account(s)");

    // The drill proper: one full refresh through staged ingestion.
    let result = connector_sync_impl(
        &state,
        adapter,
        ConnectorSyncInput {
            connection_id: linked.connection_id.clone(),
            idempotency_key: format!("drill-{}", uuid::Uuid::now_v7()),
        },
    )
    .unwrap();
    println!(
        "refresh: status {}, staged {}, committed {}, flagged {}, skipped_unmapped {}, {} warning(s)",
        result.status,
        result.staged,
        result.committed,
        result.flagged,
        result.skipped_unmapped,
        result.warnings.len()
    );
    assert!(
        result.status == "synced" || result.status == "partially_committed",
        "expected a successful refresh, got status {}",
        result.status
    );

    let connections = connector_connections_impl(&state).unwrap();
    assert!(connections[0].last_synced_at.is_some());
    assert!(connections[0].last_error.is_none());
}
