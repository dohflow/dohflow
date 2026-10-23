//! Re-authentication notices end to end (personal-cfo-pxi.22; ADR 0060 addendum 2026-10-23): a
//! real temp-vault kernel and the deterministic mock connector, no network and no mocked DB.
//!
//! The notice is device-local state written by a refresh and cleared by the next healthy one:
//! show (a bank needs a sign-in, the rest of the sync still ingests), partial (only that bank is
//! named), clear (the next healthy refresh), plus the failure paths and `open_provider_site`.

use std::cell::RefCell;

use app_lib::ipc::commands::{
    connector_connections_impl, connector_forget_impl, connector_link_impl,
    connector_set_account_link_impl, connector_sync_impl, create_account_impl,
    open_provider_site_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, CashflowRoleDto, ConnectorAttentionDto, ConnectorConnectionDto,
    ConnectorForgetInput, ConnectorLinkInput, ConnectorProviderSiteInput,
    ConnectorSetAccountLinkInput, ConnectorSyncInput, ConnectorSyncResultDto, CreateAccountInput,
};
use app_lib::ipc::IpcError;
use app_lib::AppState;
use connector_core::mock::{FailureMode, MockConnector};
use connector_core::ConnectorAdapter;
use finance_kernel::VaultController;
use tempfile::TempDir;

fn open_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(b"test-key").expect("create vault");
    (dir, AppState::new(controller))
}

fn healthy(adapter_id: &'static str) -> MockConnector {
    MockConnector::with_fixture().with_id(adapter_id)
}

fn make_account(state: &AppState, name: &str) -> String {
    create_account_impl(
        state,
        CreateAccountInput {
            name: name.to_owned(),
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
    .account_id
}

fn link(state: &AppState, adapter: &dyn ConnectorAdapter, adapter_id: &str) -> String {
    connector_link_impl(
        state,
        adapter,
        ConnectorLinkInput {
            adapter_id: adapter_id.to_owned(),
            setup_token: "mock-setup-token".to_owned(),
        },
    )
    .unwrap()
    .connection_id
}

/// A linked connection with both mock accounts mapped, so syncs ingest real rows.
fn mapped_connection(state: &AppState, adapter_id: &'static str) -> String {
    let connection_id = link(state, &healthy(adapter_id), adapter_id);
    for (external, name) in [
        ("mock-acct-checking", "Checking"),
        ("mock-acct-card", "Card"),
    ] {
        let account = make_account(state, name);
        connector_set_account_link_impl(
            state,
            ConnectorSetAccountLinkInput {
                connection_id: connection_id.clone(),
                external_id: external.to_owned(),
                account_id: Some(account),
                allow_shared_feed: false,
            },
        )
        .unwrap();
    }
    connection_id
}

fn sync(state: &AppState, adapter: &dyn ConnectorAdapter, id: &str) -> ConnectorSyncResultDto {
    connector_sync_impl(
        state,
        adapter,
        ConnectorSyncInput {
            connection_id: id.to_owned(),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
        },
    )
    .unwrap()
}

fn connection(state: &AppState, id: &str) -> ConnectorConnectionDto {
    connector_connections_impl(state)
        .unwrap()
        .into_iter()
        .find(|c| c.id == id)
        .expect("the connection is listed")
}

fn attention(kind: &str, institution: Option<&str>) -> ConnectorAttentionDto {
    ConnectorAttentionDto {
        kind: kind.to_owned(),
        institution: institution.map(str::to_owned),
    }
}

const REAUTH: &str = "reauth_institution";
const RELINK: &str = "relink_connection";

#[test]
fn a_new_connection_has_no_notice() {
    let (_dir, state) = open_state();
    let id = mapped_connection(&state, "other");
    assert!(connection(&state, &id).attention.is_empty());
}

#[test]
fn show_a_bank_that_needs_a_sign_in_is_named_and_the_rest_of_the_sync_still_ingests() {
    let (_dir, state) = open_state();
    let id = mapped_connection(&state, "other");

    let one_bank = healthy("other").with_reauth_required(&[Some("First Bank")]);
    let result = sync(&state, &one_bank, &id);

    // Acceptance 3: the healthy data is neither hidden nor blocked (mock fixture: 6 rows).
    assert_eq!(result.status, "synced", "{result:?}");
    assert_eq!(result.committed, 6, "{result:?}");
    let after = connection(&state, &id);
    assert_eq!(after.attention, vec![attention(REAUTH, Some("First Bank"))]);
    assert!(after.last_error.is_none(), "the sync itself was healthy");
    assert!(after.last_synced_at.is_some());
}

#[test]
fn partial_only_the_banks_that_need_attention_are_named_and_each_once() {
    let (_dir, state) = open_state();
    let id = mapped_connection(&state, "other");

    let two = healthy("other").with_reauth_required(&[
        Some("First Bank"),
        None,
        Some("First Bank"),
        Some("Third Bank"),
    ]);
    sync(&state, &two, &id);
    assert_eq!(
        connection(&state, &id).attention,
        vec![
            attention(REAUTH, Some("First Bank")),
            attention(REAUTH, None),
            attention(REAUTH, Some("Third Bank")),
        ]
    );

    // The next refresh says only one bank is still out of date: the notice shrinks to it.
    let one = healthy("other").with_reauth_required(&[Some("Third Bank")]);
    sync(&state, &one, &id);
    assert_eq!(
        connection(&state, &id).attention,
        vec![attention(REAUTH, Some("Third Bank"))]
    );
}

#[test]
fn clear_the_next_healthy_refresh_removes_the_notice() {
    let (_dir, state) = open_state();
    let id = mapped_connection(&state, "other");
    sync(
        &state,
        &healthy("other").with_reauth_required(&[Some("First Bank")]),
        &id,
    );
    assert_eq!(connection(&state, &id).attention.len(), 1);

    let result = sync(&state, &healthy("other"), &id);
    assert_eq!(result.status, "synced", "{result:?}");
    assert!(connection(&state, &id).attention.is_empty());
}

#[test]
fn an_expired_credential_asks_for_the_connection_to_be_set_up_again_until_a_healthy_refresh() {
    let (_dir, state) = open_state();
    let id = mapped_connection(&state, "other");

    let expired = MockConnector::failing_with(FailureMode::Expired).with_id("other");
    let failed = sync(&state, &expired, &id);
    assert_eq!(failed.status, "expired", "{failed:?}");
    let after = connection(&state, &id);
    assert_eq!(after.attention, vec![attention(RELINK, None)]);
    assert!(after.last_error.is_some());

    sync(&state, &healthy("other"), &id);
    let healed = connection(&state, &id);
    assert!(healed.attention.is_empty());
    assert!(healed.last_error.is_none());
}

#[test]
fn a_bank_sign_in_that_is_fatal_on_this_path_is_a_notice_without_a_name() {
    let (_dir, state) = open_state();
    let id = mapped_connection(&state, "other");
    // The mock's NeedsUserAction mode answers `con.auth` from sync (the provider's fatal path).
    let broken = MockConnector::failing_with(FailureMode::NeedsUserAction).with_id("other");
    let failed = sync(&state, &broken, &id);
    assert_eq!(failed.status, "needs_user_action", "{failed:?}");
    assert_eq!(
        connection(&state, &id).attention,
        vec![attention(REAUTH, None)]
    );
}

#[test]
fn a_rate_limit_is_not_a_healthy_refresh_and_leaves_the_notice_alone() {
    let (_dir, state) = open_state();
    let id = mapped_connection(&state, "other");
    sync(
        &state,
        &healthy("other").with_reauth_required(&[Some("First Bank")]),
        &id,
    );
    let throttled = MockConnector::failing_with(FailureMode::RateLimited).with_id("other");
    let result = sync(&state, &throttled, &id);
    assert_eq!(result.status, "rate_limited", "{result:?}");
    assert_eq!(
        connection(&state, &id).attention,
        vec![attention(REAUTH, Some("First Bank"))],
        "a refresh that fetched nothing proves nothing about the bank"
    );
}

#[test]
fn a_successful_discovery_clears_a_stale_notice() {
    let (_dir, state) = open_state();
    // Linked but unmapped: the refresh is the balances-only discovery pass.
    let id = link(&state, &healthy("other"), "other");
    let expired = MockConnector::failing_with(FailureMode::Expired).with_id("other");
    assert_eq!(sync(&state, &expired, &id).status, "expired");
    assert_eq!(
        connection(&state, &id).attention,
        vec![attention(RELINK, None)]
    );

    let healed = sync(&state, &healthy("other"), &id);
    assert_eq!(healed.status, "discovered_accounts", "{healed:?}");
    assert!(connection(&state, &id).attention.is_empty());
}

#[test]
fn each_connection_has_its_own_notice() {
    let (_dir, state) = open_state();
    let a = mapped_connection(&state, "other");
    let b = link(&state, &healthy("other"), "other");
    sync(
        &state,
        &healthy("other").with_reauth_required(&[Some("First Bank")]),
        &a,
    );
    assert_eq!(connection(&state, &a).attention.len(), 1);
    assert!(connection(&state, &b).attention.is_empty());
}

#[test]
fn forgetting_a_connection_takes_its_notice_with_it() {
    let (_dir, state) = open_state();
    let id = mapped_connection(&state, "other");
    sync(
        &state,
        &healthy("other").with_reauth_required(&[Some("First Bank")]),
        &id,
    );
    connector_forget_impl(
        &state,
        ConnectorForgetInput {
            connection_id: id.clone(),
        },
    )
    .unwrap();
    assert!(connector_connections_impl(&state).unwrap().is_empty());
}

#[test]
fn the_listing_carries_no_credential_and_the_notice_only_the_bank_name() {
    let (_dir, state) = open_state();
    let id = mapped_connection(&state, "other");
    sync(
        &state,
        &healthy("other").with_reauth_required(&[Some("First Bank")]),
        &id,
    );
    let wire = serde_json::to_string(&connector_connections_impl(&state).unwrap()).unwrap();
    assert!(wire.contains("First Bank"));
    for forbidden in ["mock-access-url", "credential", "http://", "https://"] {
        assert!(!wire.contains(forbidden), "{forbidden:?} in {wire}");
    }
}

// ---------------------------------------------------------------------------
// open_provider_site: the webview names a connection, never an address.
// ---------------------------------------------------------------------------

fn open_with_recorder(
    state: &AppState,
    connection_id: &str,
) -> (Result<(), IpcError>, Vec<&'static str>) {
    let opened = RefCell::new(Vec::new());
    let result = open_provider_site_impl(
        state,
        ConnectorProviderSiteInput {
            connection_id: connection_id.to_owned(),
        },
        &|url| {
            opened.borrow_mut().push(url);
            Ok(())
        },
    );
    (result, opened.into_inner())
}

#[test]
fn a_known_simplefin_connection_opens_the_bridge_and_nothing_else() {
    let (_dir, state) = open_state();
    let id = link(&state, &healthy("simplefin"), "simplefin");
    let (result, opened) = open_with_recorder(&state, &id);
    result.unwrap();
    assert_eq!(opened, vec!["https://bridge.simplefin.org"]);
}

#[test]
fn an_unknown_or_malformed_connection_id_opens_nothing() {
    let (_dir, state) = open_state();
    let (result, opened) = open_with_recorder(&state, &uuid::Uuid::new_v4().to_string());
    assert!(matches!(result, Err(IpcError::Validation(_))), "{result:?}");
    assert!(opened.is_empty());

    let (result, opened) = open_with_recorder(&state, "not-a-uuid");
    assert!(matches!(result, Err(IpcError::Validation(_))), "{result:?}");
    assert!(opened.is_empty());
}

#[test]
fn an_adapter_with_no_registered_site_opens_nothing_and_names_nothing() {
    let (_dir, state) = open_state();
    // "other" is the mock's schema token; no provider is registered under it.
    let id = link(&state, &healthy("other"), "other");
    let (result, opened) = open_with_recorder(&state, &id);
    match result {
        Err(IpcError::Unavailable(message)) => {
            assert!(!message.contains("other"), "{message}");
            assert!(!message.contains(&id), "{message}");
        }
        other => panic!("expected Unavailable, got {other:?}"),
    }
    assert!(opened.is_empty());
}

#[test]
fn a_failure_to_open_is_reported_not_swallowed() {
    let (_dir, state) = open_state();
    let id = link(&state, &healthy("simplefin"), "simplefin");
    let result = open_provider_site_impl(
        &state,
        ConnectorProviderSiteInput { connection_id: id },
        &|_| {
            Err(IpcError::Unavailable(
                "the provider's site could not be opened".to_owned(),
            ))
        },
    );
    assert!(
        matches!(result, Err(IpcError::Unavailable(_))),
        "{result:?}"
    );
}

// ---------------------------------------------------------------------------
// Redaction (DoD §2.4): the notice paths put no bank name, address or credential in logs or spans.
// ---------------------------------------------------------------------------

fn capture<F: FnOnce()>(f: F) -> String {
    use observability::RedactingMakeWriter;
    use std::io;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::layer::SubscriberExt as _;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);
    impl io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Buffer;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }
    let buffer = Buffer::default();
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(RedactingMakeWriter::new(buffer.clone()));
    observability::testing::install_global_test_subscriber();
    tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), f);
    let logged = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    logged
}

#[test]
fn the_notice_paths_log_no_bank_name_address_or_credential() {
    const BANK: &str = "Zeta Canary Credit Union";
    let (_dir, state) = open_state();
    let id = mapped_connection(&state, "simplefin");

    let opened = RefCell::new(Vec::new());
    let logged = capture(|| {
        // A sync that writes a notice, then a failed refresh that rewrites it, then the link.
        sync(
            &state,
            &healthy("simplefin").with_reauth_required(&[Some(BANK)]),
            &id,
        );
        let expired = MockConnector::failing_with(FailureMode::Expired).with_id("simplefin");
        sync(&state, &expired, &id);
        open_provider_site_impl(
            &state,
            ConnectorProviderSiteInput {
                connection_id: id.clone(),
            },
            &|url| {
                opened.borrow_mut().push(url);
                Ok(())
            },
        )
        .unwrap();
    });

    // DoD §2.4: prove the capture saw the work before trusting a no-leak check.
    assert!(
        logged.contains("connector operation completed"),
        "the capture saw nothing of the refreshes: {logged:?}"
    );
    assert_eq!(
        opened.into_inner(),
        ["https://bridge.simplefin.org"],
        "the address was resolved (and, being a constant, never logged)"
    );
    for needle in [
        BANK,
        "bridge.simplefin.org",
        "https://",
        "mock-setup-token",
        "mock-access-url",
    ] {
        assert!(
            !logged.contains(needle),
            "{needle:?} reached the logs: {logged}"
        );
    }
    // And the listing the notice renders from carries the name only, no address or credential.
    let wire = serde_json::to_string(&connector_connections_impl(&state).unwrap()).unwrap();
    for needle in [
        "bridge.simplefin.org",
        "https://",
        "mock-setup-token",
        "mock-access-url",
    ] {
        assert!(!wire.contains(needle), "{needle:?} in the listing: {wire}");
    }
}
