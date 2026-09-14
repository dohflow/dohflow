//! The Launch-gate item-8 verification drill (personal-cfo-gglk, ADR 0060
//! §2): a REAL end-to-end run against the SimpleFIN demo endpoint — sync
//! through staged ingestion into a real temp vault, committed via the Money
//! Inbox pipeline.
//!
//! Network test, `#[ignore]`d so the default suite stays deterministic. Run
//! deliberately:
//!
//! ```sh
//! cargo test --test simplefin_drill -- --ignored --nocapture
//! ```
//!
//! The drill uses the Bridge's standing demo access URL (`demo:demo@…`)
//! directly rather than claiming a setup token: demo setup tokens are
//! single-use and cannot be embedded in a repeatable test, and the claim
//! exchange itself is covered by the adapter's fixture tests
//! (crates/connectors/simplefin-adapter/tests/simplefin_flow.rs).

use app_lib::ipc::commands::{
    connector_connections_impl, connector_set_account_link_impl, connector_sync_impl,
    create_account_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, CashflowRoleDto, ConnectorSetAccountLinkInput, ConnectorSyncInput,
    CreateAccountInput,
};
use app_lib::AppState;
use finance_kernel::VaultController;
use tempfile::TempDir;

const DEMO_ACCESS_URL: &str = "https://demo:demo@beta-bridge.simplefin.org/simplefin";

#[test]
#[ignore = "network: drives the live SimpleFIN demo endpoint — run with --ignored"]
fn demo_token_end_to_end_sync_stages_and_commits() {
    let dir = TempDir::new().expect("temp dir");
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(b"drill-key").expect("create vault");
    let state = AppState::new(controller);

    let adapter =
        connector_core::connector_by_id("simplefin").expect("simplefin adapter registered");

    // Store the demo connection the way connector_link would, minus the
    // single-use claim (see module doc).
    let connection_id = uuid::Uuid::now_v7();
    {
        let guard = state.lock_controller().unwrap();
        let kernel = guard.kernel().unwrap();
        kernel
            .create_connector_connection(
                connection_id,
                "simplefin",
                DEMO_ACCESS_URL,
                Some("SimpleFIN demo drill"),
            )
            .unwrap();
    }

    // Discover the demo accounts (network request #1) and map each onto a
    // fresh real account.
    let discovered = adapter
        .fetch_accounts(&connector_core::Connection {
            credential: connector_core::Credential::new(DEMO_ACCESS_URL),
        })
        .expect("demo /accounts reachable");
    assert!(
        !discovered.is_empty(),
        "the demo endpoint exposes at least one account"
    );
    {
        let guard = state.lock_controller().unwrap();
        let kernel = guard.kernel().unwrap();
        for account in &discovered {
            let external_id = account.external_id.as_deref().expect("stable account id");
            kernel
                .upsert_connector_link(connection_id, external_id, account.external_name.as_deref())
                .unwrap();
        }
    }
    for account in &discovered {
        let real = create_account_impl(
            &state,
            CreateAccountInput {
                name: account
                    .external_name
                    .clone()
                    .unwrap_or_else(|| "Demo".to_owned()),
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
                connection_id: connection_id.to_string(),
                external_id: account.external_id.clone().unwrap(),
                account_id: Some(real),
            },
        )
        .unwrap();
    }

    // The drill proper: one full sync through staged ingestion (network
    // request(s) #2+, chunked walk).
    let result = connector_sync_impl(
        &state,
        adapter,
        ConnectorSyncInput {
            connection_id: connection_id.to_string(),
            idempotency_key: format!("drill-{}", uuid::Uuid::now_v7()),
        },
    )
    .unwrap();

    println!("drill outcome: {result:?}");
    assert!(
        result.status == "synced" || result.status == "partially_committed",
        "expected a successful sync, got {result:?}"
    );
    assert!(
        result.staged > 0,
        "the demo endpoint returns transactions; staged_transactions must populate"
    );

    // The connection row reflects the successful sync (health surface data).
    let connections = connector_connections_impl(&state).unwrap();
    assert!(connections[0].last_synced_at.is_some());
    assert!(connections[0].last_error.is_none());
}
