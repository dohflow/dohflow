//! One bank account reached through more than one connection
//! (personal-cfo-6evt; ADR 0014 §3 addendum 2026-10-04; ADR 0076 decision 3a).
//!
//! A connector's identity is its connection. These tests link real connector
//! connections (the in-memory mock, reporting the `simplefin` and `lunchflow`
//! source types with provider-shaped ids) and refresh them through
//! `connector_sync_impl` — the same staging, pre-check, `CommitStaged` and
//! Money Inbox path a real refresh takes. The single-connection vault is the
//! baseline: however the account is reached, the ledger holds one committed
//! transaction per real transaction, the balance and the Future Cash forecast
//! equal the baseline, and every overlap waits in the Money Inbox for review.
//! No network, no real credentials.

use app_lib::ipc::commands::{
    account_balance_impl, connector_link_impl, connector_set_account_link_impl,
    connector_sync_impl, create_account_impl, future_cash_forecast_impl, money_inbox_list_impl,
    transaction_list_impl, void_transaction_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, CashflowRoleDto, ConnectorLinkInput, ConnectorSetAccountLinkInput,
    ConnectorSetAccountLinkResultDto, ConnectorSyncInput, ConnectorSyncResultDto,
    CreateAccountInput, ForecastViewDto, MoneyDto, MoneyInboxItemDto,
};
use app_lib::AppState;
use connector_core::mock::MockConnector;
use finance_kernel::VaultController;
use tempfile::TempDir;

const CHECKING: &str = "mock-acct-checking";
/// The mock's checking account holds three posted transactions.
const REAL_TRANSACTIONS: usize = 3;

fn open_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(b"test-key").expect("create vault");
    (dir, AppState::new(controller))
}

fn account(state: &AppState, name: &str) -> String {
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
    .expect("account")
    .account_id
}

/// The `simplefin`-typed connection a household member links.
fn simplefin(prefix: &'static str) -> MockConnector {
    MockConnector::with_fixture()
        .with_id("simplefin")
        .with_provider_ids("sfin", prefix)
        .refetching_everything()
}

/// A `lunchflow`-typed connection: another provider, its own id namespace.
fn lunchflow() -> MockConnector {
    MockConnector::with_fixture()
        .with_id("lunchflow")
        .with_provider_ids("lflow", "LF")
        .refetching_everything()
}

fn link(state: &AppState, adapter: &MockConnector) -> String {
    connector_link_impl(
        state,
        adapter,
        ConnectorLinkInput {
            adapter_id: adapter_id(adapter).to_owned(),
            setup_token: "mock-setup-token".to_owned(),
        },
    )
    .expect("link")
    .connection_id
}

fn adapter_id(adapter: &MockConnector) -> &'static str {
    use connector_core::ConnectorAdapter;
    adapter.id()
}

fn map(
    state: &AppState,
    connection_id: &str,
    account_id: Option<&str>,
    allow_shared_feed: bool,
) -> ConnectorSetAccountLinkResultDto {
    connector_set_account_link_impl(
        state,
        ConnectorSetAccountLinkInput {
            connection_id: connection_id.to_owned(),
            external_id: CHECKING.to_owned(),
            account_id: account_id.map(str::to_owned),
            allow_shared_feed,
        },
    )
    .expect("set link")
}

fn refresh(
    state: &AppState,
    adapter: &MockConnector,
    connection_id: &str,
) -> ConnectorSyncResultDto {
    connector_sync_impl(
        state,
        adapter,
        ConnectorSyncInput {
            connection_id: connection_id.to_owned(),
            idempotency_key: uuid::Uuid::now_v7().to_string(),
        },
    )
    .expect("refresh")
}

fn ledger_count(state: &AppState, account_id: &str) -> usize {
    transaction_list_impl(state)
        .unwrap()
        .iter()
        .filter(|t| t.account_id == account_id)
        .count()
}

fn waiting(state: &AppState) -> Vec<MoneyInboxItemDto> {
    money_inbox_list_impl(state)
        .unwrap()
        .into_iter()
        .filter(|item| item.item_kind == "imported_waiting_commit")
        .collect()
}

fn reason(item: &MoneyInboxItemDto) -> String {
    let payload: serde_json::Value = serde_json::from_str(&item.payload_json).unwrap();
    payload["dedupe_reason"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// What a household should see for the account: balance and Future Cash.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    committed: usize,
    balance: Option<MoneyDto>,
    forecast: ForecastViewDto,
}

fn outcome(state: &AppState, account_id: &str) -> Outcome {
    Outcome {
        committed: ledger_count(state, account_id),
        balance: account_balance_impl(state, account_id.to_owned()).unwrap(),
        forecast: future_cash_forecast_impl(state, 90, Vec::new()).unwrap(),
    }
}

/// The single-connection baseline every multi-connection vault must equal.
fn baseline() -> Outcome {
    let (_dir, state) = open_state();
    let checking = account(&state, "Joint Checking");
    let adapter = simplefin("A");
    let connection = link(&state, &adapter);
    assert!(map(&state, &connection, Some(&checking), false).linked);
    refresh(&state, &adapter, &connection);
    let out = outcome(&state, &checking);
    assert_eq!(out.committed, REAL_TRANSACTIONS);
    assert!(waiting(&state).is_empty());
    out
}

// ---- the mapping guard -------------------------------------------------------

#[test]
fn a_second_feed_onto_an_account_is_warned_about_and_saved_only_on_request() {
    let (_dir, state) = open_state();
    let checking = account(&state, "Joint Checking");
    let savings = account(&state, "Savings");
    let alex = simplefin("A");
    let sam = lunchflow();
    let alex_connection = link(&state, &alex);
    let sam_connection = link(&state, &sam);

    // The first feed maps silently; re-saving the same link is not a "second".
    assert!(map(&state, &alex_connection, Some(&checking), false).linked);
    assert!(map(&state, &alex_connection, Some(&checking), false).linked);

    // A second connection onto the same account: nothing is saved, and the
    // result names the existing feed.
    let warned = map(&state, &sam_connection, Some(&checking), false);
    assert!(!warned.linked);
    assert_eq!(warned.existing_feeds.len(), 1);
    let feed = &warned.existing_feeds[0];
    assert_eq!(feed.connection_id, alex_connection);
    assert_eq!(feed.adapter_id, "simplefin");
    assert_eq!(feed.external_id, CHECKING);
    let sam_link = |state: &AppState| {
        app_lib::ipc::commands::connector_connections_impl(state)
            .unwrap()
            .into_iter()
            .find(|c| c.id == sam_connection)
            .unwrap()
            .links
            .into_iter()
            .find(|l| l.external_id == CHECKING)
            .unwrap()
            .account_id
    };
    assert_eq!(sam_link(&state), None, "cancel leaves the link unmapped");

    // "Map it to a different account" and unmapping never warn.
    assert!(map(&state, &sam_connection, Some(&savings), false).linked);
    assert!(map(&state, &sam_connection, None, false).linked);
    assert_eq!(sam_link(&state), None);

    // "Import from both" saves it.
    assert!(map(&state, &sam_connection, Some(&checking), true).linked);
    assert_eq!(sam_link(&state).as_deref(), Some(checking.as_str()));
}

#[test]
fn two_connections_to_one_provider_are_warned_about_too() {
    let (_dir, state) = open_state();
    let checking = account(&state, "Joint Checking");
    let alex = simplefin("A");
    let sam = simplefin("B");
    let alex_connection = link(&state, &alex);
    let sam_connection = link(&state, &sam);
    assert!(map(&state, &alex_connection, Some(&checking), false).linked);
    let warned = map(&state, &sam_connection, Some(&checking), false);
    assert!(!warned.linked);
    assert_eq!(warned.existing_feeds[0].connection_id, alex_connection);
}

// ---- one account, several feeds: no double count -----------------------------

/// Link a second feed onto the baseline account with "Import from both", refresh
/// both twice, and check the account equals the single-connection baseline with
/// every overlap waiting for review.
fn shared_account(second: &MockConnector, expected_reason: &str) {
    let expected = baseline();
    let (_dir, state) = open_state();
    let checking = account(&state, "Joint Checking");
    let first = simplefin("A");
    let first_connection = link(&state, &first);
    let second_connection = link(&state, second);
    assert!(map(&state, &first_connection, Some(&checking), false).linked);
    assert!(map(&state, &second_connection, Some(&checking), true).linked);

    let one = refresh(&state, &first, &first_connection);
    assert_eq!(one.committed as usize, REAL_TRANSACTIONS);
    let two = refresh(&state, second, &second_connection);
    assert_eq!(
        (two.committed, two.flagged as usize),
        (0, REAL_TRANSACTIONS),
        "every overlap is flagged, none committed again"
    );
    let items = waiting(&state);
    assert_eq!(items.len(), REAL_TRANSACTIONS);
    assert!(
        items.iter().all(|item| reason(item) == expected_reason),
        "{:?}",
        items.iter().map(reason).collect::<Vec<_>>()
    );
    assert_eq!(
        outcome(&state, &checking),
        expected,
        "balance and Future Cash equal the baseline"
    );

    // Repeat refreshes of both connections change nothing.
    for _ in 0..2 {
        let again = refresh(&state, &first, &first_connection);
        assert_eq!(
            again.staged as usize, REAL_TRANSACTIONS,
            "the rows really are re-fetched"
        );
        assert_eq!((again.committed, again.flagged), (0, 0));
        let again = refresh(&state, second, &second_connection);
        assert_eq!(again.staged as usize, REAL_TRANSACTIONS);
        assert_eq!((again.committed, again.flagged), (0, 0));
    }
    assert_eq!(
        waiting(&state).len(),
        REAL_TRANSACTIONS,
        "no repeated inbox items"
    );
    assert_eq!(outcome(&state, &checking), expected);
}

#[test]
fn two_providers_on_one_account_never_double_count() {
    shared_account(
        &lunchflow(),
        "same date and amount already recorded from another source",
    );
}

#[test]
fn two_connections_of_one_provider_with_their_own_ids_never_double_count() {
    // Before 6evt both rows committed: same source type, different ids.
    shared_account(
        &simplefin("B"),
        "same date and amount already recorded by another connection",
    );
}

#[test]
fn two_connections_reporting_the_same_ids_are_reviewed_not_silently_skipped() {
    // A shared namespace is not proven across connections, so a matching id
    // from another connection goes to review instead of being trusted.
    shared_account(
        &simplefin("A"),
        "duplicate of an already-committed transaction",
    );
}

// ---- pending, then posted, across providers ----------------------------------

/// The newest row (row 2) is still pending at one provider when the other
/// reports it posted; later the first reports it posted too. Either arrival order ends with one
/// committed transaction per real transaction, the overlap flagged, and the
/// baseline balance and forecast.
fn pending_then_posted(pending_first: bool) {
    let expected = baseline();
    let (_dir, state) = open_state();
    let checking = account(&state, "Joint Checking");
    let sfin = simplefin("A");
    let lflow = lunchflow();
    let sfin_connection = link(&state, &sfin);
    let lflow_connection = link(&state, &lflow);
    assert!(map(&state, &sfin_connection, Some(&checking), false).linked);
    assert!(map(&state, &lflow_connection, Some(&checking), true).linked);

    let (early, early_connection, late, late_connection) = if pending_first {
        (&sfin, &sfin_connection, &lflow, &lflow_connection)
    } else {
        (&lflow, &lflow_connection, &sfin, &sfin_connection)
    };
    // The early provider still has row 2 pending: two posted rows commit.
    let early_pending = early.clone().with_pending(&[2]);
    let first = refresh(&state, &early_pending, early_connection);
    assert_eq!(first.committed, 2);
    assert!(
        first.warnings.iter().any(|w| w.contains("pending")),
        "{:?}",
        first.warnings
    );
    // The late provider reports all three posted: row 2 is new to the ledger;
    // rows 0 and 1 overlap.
    let second = refresh(&state, late, late_connection);
    assert_eq!((second.committed, second.flagged), (1, 2));
    // Now the early provider reports row 2 posted: it overlaps the late
    // provider's row 2 and is flagged, not committed twice.
    let third = refresh(&state, early, early_connection);
    assert_eq!((third.committed, third.flagged), (0, 1));

    assert_eq!(waiting(&state).len(), REAL_TRANSACTIONS);
    assert_eq!(
        outcome(&state, &checking),
        expected,
        "no double-counted cash"
    );

    // Repeat refreshes are stable.
    for (adapter, connection) in [(early, early_connection), (late, late_connection)] {
        let again = refresh(&state, adapter, connection);
        assert_eq!(
            again.staged as usize, REAL_TRANSACTIONS,
            "the rows really are re-fetched"
        );
        assert_eq!((again.committed, again.flagged), (0, 0));
    }
    assert_eq!(waiting(&state).len(), REAL_TRANSACTIONS);
    assert_eq!(outcome(&state, &checking), expected);
}

#[test]
fn a_transaction_pending_at_one_provider_and_posted_at_the_other_counts_once() {
    pending_then_posted(true);
}

#[test]
fn the_same_holds_in_the_reverse_arrival_order() {
    pending_then_posted(false);
}

// ---- one connection's own refetch stays silent ------------------------------

#[test]
fn a_connection_refreshing_its_own_rows_stays_silent() {
    let (_dir, state) = open_state();
    let checking = account(&state, "Checking");
    let adapter = simplefin("A");
    let connection = link(&state, &adapter);
    assert!(map(&state, &connection, Some(&checking), false).linked);
    refresh(&state, &adapter, &connection);
    for _ in 0..3 {
        let again = refresh(&state, &adapter, &connection);
        assert_eq!(
            again.staged as usize, REAL_TRANSACTIONS,
            "the rows really are re-fetched"
        );
        assert_eq!((again.committed, again.flagged), (0, 0));
    }
    assert_eq!(ledger_count(&state, &checking), REAL_TRANSACTIONS);
    assert!(waiting(&state).is_empty());
}

/// Batches synced before the connection was recorded carry none and count as
/// the same connection as any later batch of their type (the addendum's
/// upgrade rule): the first refresh after upgrading flags nothing it re-fetches.
#[test]
fn rows_synced_before_connections_were_recorded_do_not_flood_the_first_refresh() {
    let (_dir, state) = open_state();
    let checking = account(&state, "Checking");
    let adapter = simplefin("A");
    let connection = link(&state, &adapter);
    assert!(map(&state, &connection, Some(&checking), false).linked);

    // A pre-upgrade refresh: the same rows ingested with no connection recorded.
    {
        use connector_core::ConnectorAdapter;
        let synced = adapter
            .sync(
                &connector_core::Connection {
                    credential: connector_core::Credential::new("mock-access-url"),
                },
                None,
            )
            .expect("mock sync");
        let guard = state.lock_controller().unwrap();
        let kernel = guard.kernel().unwrap();
        let map = std::collections::BTreeMap::from([(
            CHECKING.to_owned(),
            uuid::Uuid::parse_str(&checking).unwrap(),
        )]);
        let legacy = kernel
            .ingest_sync_batch(
                "simplefin",
                "0.1.0",
                "legacy",
                None,
                &synced.batch,
                &map,
                &finance_kernel::CommandMeta {
                    command_id: uuid::Uuid::now_v7(),
                    correlation_id: uuid::Uuid::now_v7(),
                    causation_id: None,
                    actor_type: finance_kernel::ActorType::User,
                    actor_id: "tester".to_owned(),
                    idempotency_key: uuid::Uuid::now_v7().to_string(),
                },
            )
            .expect("legacy ingest");
        assert_eq!(legacy.batch.committed as usize, REAL_TRANSACTIONS);
    }

    // The first refresh after upgrading records its connection and re-fetches
    // the same rows: all silently recognized.
    let after = refresh(&state, &adapter, &connection);
    assert_eq!(
        after.staged as usize, REAL_TRANSACTIONS,
        "the rows really are re-fetched"
    );
    assert_eq!((after.committed, after.flagged), (0, 0));
    assert!(waiting(&state).is_empty());
    assert_eq!(ledger_count(&state, &checking), REAL_TRANSACTIONS);
}

/// Void one committed row from the first connection: the second connection's
/// copy is no longer a duplicate of anything live, so it imports; the others
/// still wait for review.
#[test]
fn a_voided_row_from_one_connection_lets_the_other_connections_copy_import() {
    let (_dir, state) = open_state();
    let checking = account(&state, "Joint Checking");
    let first = simplefin("A");
    let second = simplefin("B");
    let first_connection = link(&state, &first);
    let second_connection = link(&state, &second);
    assert!(map(&state, &first_connection, Some(&checking), false).linked);
    assert!(map(&state, &second_connection, Some(&checking), true).linked);
    refresh(&state, &first, &first_connection);

    let voided = transaction_list_impl(&state)
        .unwrap()
        .into_iter()
        .find(|t| t.account_id == checking)
        .expect("a committed row");
    void_transaction_impl(
        &state,
        voided.transaction_id,
        uuid::Uuid::now_v7().to_string(),
    )
    .unwrap();

    let second_refresh = refresh(&state, &second, &second_connection);
    assert_eq!(
        (second_refresh.committed, second_refresh.flagged as usize),
        (1, REAL_TRANSACTIONS - 1)
    );
    assert_eq!(ledger_count(&state, &checking), REAL_TRANSACTIONS);
    assert_eq!(waiting(&state).len(), REAL_TRANSACTIONS - 1);
}
