//! Connector integration tests (personal-cfo-gglk): link → map → sync →
//! staged ingestion, against a real temp-vault kernel and the deterministic
//! mock connector — no network, no mocked DB (per the project DoD). The mock
//! reports adapter id `"other"` (a `source_batches.source_type` schema token)
//! so its batches ride the real CHECK-constrained staging path.

use std::collections::BTreeMap;

use app_lib::ipc::commands::{
    account_count_impl, account_list_impl, connector_auto_sync_impl, connector_connections_impl,
    connector_forget_impl, connector_link_impl, connector_set_account_link_impl,
    connector_sync_impl, create_account_impl, create_category_impl,
    create_manual_future_entry_impl, import_batch_impl, manual_future_entry_list_impl,
    money_inbox_list_impl, recategorize_transaction_impl, record_transaction_impl,
    set_auto_categorize_on_import_impl, transaction_page_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, CashflowRoleDto, ConnectorForgetInput, ConnectorLinkInput,
    ConnectorSetAccountLinkInput, ConnectorSyncInput, CreateAccountInput, CreateCategoryInput,
    ImportBatchInput, MoneyDto, RecordTransactionInput, TransactionPageInput,
};
use app_lib::AppState;
use connector_core::mock::MockConnector;
use connector_core::ConnectorAdapter;
use finance_kernel::VaultController;
use tempfile::TempDir;

fn open_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(b"test-key").expect("create vault");
    (dir, AppState::new(controller))
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

fn mock() -> MockConnector {
    MockConnector::with_fixture().with_id("other")
}

fn link(state: &AppState, adapter: &dyn ConnectorAdapter) -> String {
    let result = connector_link_impl(
        state,
        adapter,
        ConnectorLinkInput {
            adapter_id: "other".to_owned(),
            setup_token: "mock-setup-token".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(
        result.accounts.len(),
        2,
        "mock fixture exposes two accounts"
    );
    assert!(result.fetch_error.is_none());
    result.connection_id
}

fn map_account(state: &AppState, connection_id: &str, external_id: &str, account_id: &str) {
    connector_set_account_link_impl(
        state,
        ConnectorSetAccountLinkInput {
            connection_id: connection_id.to_owned(),
            external_id: external_id.to_owned(),
            account_id: Some(account_id.to_owned()),
        },
    )
    .unwrap();
}

fn sync(
    state: &AppState,
    adapter: &dyn ConnectorAdapter,
    connection_id: &str,
) -> app_lib::ipc::dto::ConnectorSyncResultDto {
    connector_sync_impl(
        state,
        adapter,
        ConnectorSyncInput {
            connection_id: connection_id.to_owned(),
            idempotency_key: uuid::Uuid::now_v7().to_string(),
        },
    )
    .unwrap()
}

fn page_query() -> TransactionPageInput {
    TransactionPageInput {
        query: None,
        account_ids: vec![],
        with_balances: false,
        category_id: None,
        tag_id: None,
        recurring_event_id: None,
        from_date: None,
        to_date: None,
        unreviewed_only: false,
        sort: "newest".to_owned(),
        limit: 50,
        offset: 0,
    }
}

#[test]
fn link_stores_the_connection_and_discovers_accounts() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);

    let connections = connector_connections_impl(&state).unwrap();
    assert_eq!(connections.len(), 1);
    let connection = &connections[0];
    assert_eq!(connection.id, connection_id);
    assert_eq!(connection.adapter_id, "other");
    assert_eq!(connection.links.len(), 2);
    assert!(connection.links.iter().all(|l| l.account_id.is_none()));
    assert!(connection.links.iter().all(|l| l.last_synced_on.is_none()));
}

#[test]
fn sync_without_mappings_discovers_accounts_instead_of_dead_ending() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);

    // Unmapped sync = a discovery pass, never a transaction walk: no ledger
    // writes, but the outcome says what was found (personal-cfo-k025).
    let result = sync(&state, &adapter, &connection_id);
    assert_eq!(result.status, "discovered_accounts", "{result:?}");
    assert_eq!(result.staged, 0);
    assert_eq!(account_count_impl(&state).unwrap(), 0);
    // No last_synced_at stamp: the badge stays "Never synced" and the unlock
    // auto-sync keeps re-discovering until an account is mapped.
    let connection = &connector_connections_impl(&state).unwrap()[0];
    assert!(connection.last_synced_at.is_none());
    assert!(connection.last_error.is_none());
}

/// personal-cfo-k025 (owner dogfooding, real Bridge): link-time discovery
/// returned zero accounts, and the old early-return made "sync to discover
/// them" a lie. Now the unmapped sync fetches the account list and the links
/// appear as soon as the provider has them.
#[test]
fn sync_discovers_accounts_when_link_time_discovery_was_empty() {
    let (_dir, state) = open_state();
    let not_ready = MockConnector::with_fixture()
        .with_empty_discovery()
        .with_id("other");
    let ready = mock();

    // Link while the provider has nothing: zero links discovered.
    let result = connector_link_impl(
        &state,
        &not_ready,
        ConnectorLinkInput {
            adapter_id: "other".to_owned(),
            setup_token: "mock-setup-token".to_owned(),
        },
    )
    .unwrap();
    assert!(result.accounts.is_empty());
    assert!(result.fetch_error.is_none());
    let connection_id = result.connection_id;
    assert!(connector_connections_impl(&state).unwrap()[0]
        .links
        .is_empty());

    // Still nothing at the provider: honest "no accounts yet" outcome.
    let still_empty = sync(&state, &not_ready, &connection_id);
    assert_eq!(still_empty.status, "no_mapped_accounts", "{still_empty:?}");
    assert!(still_empty
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("no accounts yet — new connections can take a while"));

    // The provider's list is ready now: the same button discovers both.
    let discovered = sync(&state, &ready, &connection_id);
    assert_eq!(discovered.status, "discovered_accounts", "{discovered:?}");
    let links = &connector_connections_impl(&state).unwrap()[0].links;
    assert_eq!(links.len(), 2);
    assert!(links.iter().all(|l| l.account_id.is_none()));

    // Map + sync now works end to end.
    let checking = make_account(&state, "Checking");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);
    let synced = sync(&state, &ready, &connection_id);
    assert_eq!(synced.status, "synced", "{synced:?}");
}

/// A failing discovery must reach the health surface (badge + inbox), and a
/// later successful discovery must clear it — resolution stays intrinsic.
#[test]
fn discovery_failures_record_health_and_success_clears_it() {
    let (_dir, state) = open_state();
    let not_ready = MockConnector::with_fixture()
        .with_empty_discovery()
        .with_id("other");
    let connection_id = connector_link_impl(
        &state,
        &not_ready,
        ConnectorLinkInput {
            adapter_id: "other".to_owned(),
            setup_token: "mock-setup-token".to_owned(),
        },
    )
    .unwrap()
    .connection_id;

    let broken =
        MockConnector::failing_with(connector_core::mock::FailureMode::Expired).with_id("other");
    let failed = sync(&state, &broken, &connection_id);
    assert_eq!(failed.status, "expired", "{failed:?}");
    let connection = &connector_connections_impl(&state).unwrap()[0];
    assert!(connection.last_error.is_some(), "{connection:?}");

    let healed = sync(&state, &mock(), &connection_id);
    assert_eq!(healed.status, "discovered_accounts", "{healed:?}");
    let connection = &connector_connections_impl(&state).unwrap()[0];
    assert!(connection.last_error.is_none(), "{connection:?}");
}

#[test]
fn a_mapped_sync_stages_and_commits_through_the_real_pipeline() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);

    let checking = make_account(&state, "Checking");
    let card = make_account(&state, "Card");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);
    map_account(&state, &connection_id, "mock-acct-card", &card);

    let result = sync(&state, &adapter, &connection_id);
    // Mock fixture: 3 transactions + 1 balance per account, 2 accounts.
    assert_eq!(result.status, "synced");
    assert_eq!(result.staged, 6);
    assert_eq!(result.committed, 6);
    assert_eq!(result.flagged, 0);
    assert_eq!(result.skipped_unmapped, 0);

    // The transactions are real ledger rows now.
    let page = transaction_page_impl(
        &state,
        TransactionPageInput {
            query: None,
            account_ids: vec![],
            with_balances: false,
            category_id: None,
            tag_id: None,
            recurring_event_id: None,
            from_date: None,
            to_date: None,
            unreviewed_only: false,
            sort: "newest".to_owned(),
            limit: 50,
            offset: 0,
        },
    )
    .unwrap();
    assert_eq!(page.total, 6);

    // Watermarks advanced for both mapped links.
    let connections = connector_connections_impl(&state).unwrap();
    assert!(connections[0]
        .links
        .iter()
        .all(|l| l.last_synced_on.is_some()));
    assert!(connections[0].last_synced_at.is_some());
    assert!(connections[0].last_error.is_none());
}

#[test]
fn a_second_sync_dedupes_instead_of_duplicating() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let checking = make_account(&state, "Checking");
    let card = make_account(&state, "Card");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);
    map_account(&state, &connection_id, "mock-acct-card", &card);

    let first = sync(&state, &adapter, &connection_id);
    assert_eq!(first.committed, 6);
    let second = sync(&state, &adapter, &connection_id);
    // The watermark bounds the refetch, and anything re-fetched through the
    // 5-day-rewind overlap is a CERTAIN provider-id duplicate — silently
    // skipped, never flagged into the Money Inbox (review blocker: a daily
    // auto-sync must not flood the inbox with false duplicates).
    assert_eq!(second.committed, 0, "no duplicates on re-sync: {second:?}");
    assert_eq!(
        second.flagged, 0,
        "overlap re-fetches never flag: {second:?}"
    );
    let inbox = money_inbox_list_impl(&state).unwrap();
    assert!(
        inbox
            .iter()
            .all(|item| item.item_kind != "imported_waiting_commit"),
        "no duplicate-review inbox items after a re-sync: {inbox:?}"
    );

    let page = transaction_page_impl(
        &state,
        TransactionPageInput {
            query: None,
            account_ids: vec![],
            with_balances: false,
            category_id: None,
            tag_id: None,
            recurring_event_id: None,
            from_date: None,
            to_date: None,
            unreviewed_only: false,
            sort: "newest".to_owned(),
            limit: 50,
            offset: 0,
        },
    )
    .unwrap();
    assert_eq!(page.total, 6, "ledger still holds exactly the fixture rows");
}

#[test]
fn unmapped_accounts_skip_with_a_count_never_silently() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let checking = make_account(&state, "Checking");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);

    let result = sync(&state, &adapter, &connection_id);
    assert_eq!(
        result.committed, 3,
        "the mapped account's transactions commit"
    );
    // The unmapped card's 3 transactions + 1 balance are counted, not lost.
    assert_eq!(result.skipped_unmapped, 4);
}

#[test]
fn a_failing_provider_records_the_error_and_a_throttle_does_not() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let checking = make_account(&state, "Checking");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);

    let expired =
        MockConnector::failing_with(connector_core::mock::FailureMode::Expired).with_id("other");
    let result = sync(&state, &expired, &connection_id);
    assert_eq!(result.status, "expired");
    let connections = connector_connections_impl(&state).unwrap();
    assert!(connections[0].last_error.is_some());

    let throttled = MockConnector::failing_with(connector_core::mock::FailureMode::RateLimited)
        .with_id("other");
    let result = sync(&state, &throttled, &connection_id);
    assert_eq!(result.status, "rate_limited");
    // A healthy throttle CLEARS the error state (ADR 0060 §5).
    let connections = connector_connections_impl(&state).unwrap();
    assert!(connections[0].last_error.is_none());
}

#[test]
fn auto_sync_debounces_fresh_connections() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let checking = make_account(&state, "Checking");
    let card = make_account(&state, "Card");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);
    map_account(&state, &connection_id, "mock-acct-card", &card);

    // First sync stamps last_synced_at; the auto-sync must then debounce.
    let first = sync(&state, &adapter, &connection_id);
    assert_eq!(first.status, "synced");

    fn resolve(_: &str) -> Option<&'static dyn ConnectorAdapter> {
        // Leak one mock per call — test-only; the resolver shape needs
        // &'static, and a test process leaks a handful of bytes at most.
        Some(Box::leak(Box::new(
            MockConnector::with_fixture().with_id("other"),
        )))
    }
    let results = connector_auto_sync_impl(&state, resolve).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, "skipped_debounced");
}

#[test]
fn forget_removes_the_connection_but_keeps_the_ledger() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let checking = make_account(&state, "Checking");
    let card = make_account(&state, "Card");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);
    map_account(&state, &connection_id, "mock-acct-card", &card);
    assert_eq!(sync(&state, &adapter, &connection_id).committed, 6);

    connector_forget_impl(
        &state,
        ConnectorForgetInput {
            connection_id: connection_id.clone(),
        },
    )
    .unwrap();

    assert!(connector_connections_impl(&state).unwrap().is_empty());
    let page = transaction_page_impl(
        &state,
        TransactionPageInput {
            query: None,
            account_ids: vec![],
            with_balances: false,
            category_id: None,
            tag_id: None,
            recurring_event_id: None,
            from_date: None,
            to_date: None,
            unreviewed_only: false,
            sort: "newest".to_owned(),
            limit: 50,
            offset: 0,
        },
    )
    .unwrap();
    assert_eq!(page.total, 6, "synced history survives the forget");
}

/// The 1k-transaction cross-cutting half (w3gh AC, transferred to gglk): a
/// kernel-level ingest of a 1000-record connector batch populates
/// staged_transactions and commits them all.
#[test]
fn a_1k_record_sync_batch_stages_and_commits_every_row() {
    use chrono::NaiveDate;
    use finance_kernel::{ActorType, CommandMeta};
    use finance_kernel::{ParsedBatch, ParsedRecord, ParsedTransaction};

    let (_dir, state) = open_state();
    let account_id = make_account(&state, "Bulk");
    let account_uuid = uuid::Uuid::parse_str(&account_id).unwrap();

    let records: Vec<ParsedRecord> = (0..1000)
        .map(|i| {
            let posted = NaiveDate::from_ymd_opt(2026, 1, 1)
                .unwrap()
                .checked_add_days(chrono::Days::new(i % 200))
                .unwrap();
            let normalized = format!("{{\"row\":{i}}}");
            ParsedRecord {
                external_id: Some(format!("TXN-{i}")),
                source_hash: finance_kernel::content_fingerprint(
                    format!("{i}:{normalized}").as_bytes(),
                ),
                normalized_json: normalized,
                parse_confidence_bps: Some(10_000),
                transaction: Some(ParsedTransaction {
                    posted_date: posted,
                    transaction_date: None,
                    raw_date: posted.to_string(),
                    date_confidence_bps: 10_000,
                    amount: finance_kernel::Money::new(
                        -(100 + i64::try_from(i).unwrap()),
                        finance_kernel::Currency::Usd,
                    ),
                    description: Some(format!("Bulk purchase {i}")),
                    category: None,
                    normalized_merchant: Some(format!("merchant {i}")),
                    external_account: Some("EXT-BULK".to_owned()),
                    txn_fingerprint: format!("{posted}|{i}|sfin:TXN-{i}"),
                }),
                balance: None,
            }
        })
        .collect();
    let batch = ParsedBatch {
        source_format: "simplefin".to_owned(),
        accounts: vec![],
        records,
        warnings: vec![],
    };
    let map = BTreeMap::from([("EXT-BULK".to_owned(), account_uuid)]);

    let guard = state.lock_controller().unwrap();
    let kernel = guard.kernel().unwrap();
    let meta = CommandMeta {
        command_id: uuid::Uuid::now_v7(),
        correlation_id: uuid::Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "test".to_owned(),
        idempotency_key: format!("bulk-{}", uuid::Uuid::now_v7()),
    };
    let result = kernel
        .ingest_sync_batch("simplefin", "0.1.0", "bulk test", &batch, &map, &meta)
        .unwrap();
    assert_eq!(result.batch.staged, 1000);
    assert_eq!(result.batch.committed, 1000);
    assert_eq!(result.skipped_unmapped, 0);

    // Adapter provenance rides the batch: source_type = adapter id on the
    // source_batch row, adapter id + version on the parser_runs row.
    let batch_id = uuid::Uuid::parse_str(&result.batch.source_batch_id.unwrap()).unwrap();
    let (source_type, parser_name, parser_version) = kernel
        .source_batch_provenance(batch_id)
        .unwrap()
        .expect("provenance rows exist");
    assert_eq!(source_type, "simplefin");
    assert_eq!(parser_name, "simplefin");
    assert_eq!(parser_version, "0.1.0");
}

#[test]
fn retry_held_accounts_keep_their_watermark_while_others_advance() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let checking = make_account(&state, "Checking");
    let card = make_account(&state, "Card");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);
    map_account(&state, &connection_id, "mock-acct-card", &card);

    // The provider reports the checking account's window incomplete.
    let holding = MockConnector::with_fixture()
        .with_id("other")
        .with_held_accounts(&["mock-acct-checking"]);
    let result = sync(&state, &holding, &connection_id);
    assert_eq!(result.committed, 6, "data still stages: {result:?}");

    let connections = connector_connections_impl(&state).unwrap();
    let watermark = |external: &str| {
        connections[0]
            .links
            .iter()
            .find(|l| l.external_id == external)
            .unwrap()
            .last_synced_on
            .clone()
    };
    assert!(
        watermark("mock-acct-checking").is_none(),
        "the held account keeps its NULL watermark for a re-walk"
    );
    assert!(
        watermark("mock-acct-card").is_some(),
        "the healthy account's watermark advances"
    );
}

#[test]
fn remapping_an_external_account_resets_its_watermark() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let checking = make_account(&state, "Checking");
    let card = make_account(&state, "Card");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);
    map_account(&state, &connection_id, "mock-acct-card", &card);
    assert_eq!(sync(&state, &adapter, &connection_id).committed, 6);

    // Re-map checking onto a fresh account: the watermark must clear so the
    // new target receives full history on the next sync.
    let replacement = make_account(&state, "Checking Replacement");
    map_account(&state, &connection_id, "mock-acct-checking", &replacement);
    let connections = connector_connections_impl(&state).unwrap();
    let remapped = connections[0]
        .links
        .iter()
        .find(|l| l.external_id == "mock-acct-checking")
        .unwrap();
    assert!(
        remapped.last_synced_on.is_none(),
        "watermark cleared on remap"
    );

    let resync = sync(&state, &adapter, &connection_id);
    assert_eq!(
        resync.committed, 3,
        "the replacement account receives the full history: {resync:?}"
    );
}

#[test]
fn mapping_to_a_nonexistent_account_is_rejected() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let err = connector_set_account_link_impl(
        &state,
        ConnectorSetAccountLinkInput {
            connection_id,
            external_id: "mock-acct-checking".to_owned(),
            account_id: Some(uuid::Uuid::now_v7().to_string()),
        },
    )
    .unwrap_err();
    assert!(matches!(err, app_lib::ipc::IpcError::Validation(_)));
}

/// personal-cfo-yl53 (owner dogfooding: "the balance is way off"): the synced
/// provider balance must anchor the account, not the posting sum. The mock
/// observes 1234.56 on 2026-08-04 with its 2500.00 deposit posted THAT day —
/// the pre-fix posting-sum answer (2441.51) is exactly the wrong number this
/// test forbids.
#[test]
fn synced_provider_balance_anchors_the_account() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let checking = make_account(&state, "Checking");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);

    let result = sync(&state, &adapter, &connection_id);
    assert_eq!(result.status, "synced", "{result:?}");

    let view = account_list_impl(&state)
        .unwrap()
        .into_iter()
        .find(|a| a.id == checking)
        .unwrap();
    // Anchor = the provider's observed balance + postings strictly after it
    // (none in the fixture), NOT the sum of synced postings.
    assert_eq!(view.balance.minor_units, 123_456, "{view:?}");
}

/// personal-cfo-kz88: the sync path runs the same setting-gated merchant-memory
/// auto-categorization as file imports, and says so in the outcome.
#[test]
fn sync_auto_categorizes_from_merchant_memory() {
    let (_dir, state) = open_state();
    set_auto_categorize_on_import_impl(&state, true).unwrap();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let checking = make_account(&state, "Checking");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);
    let first = sync(&state, &adapter, &connection_id);
    assert_eq!(first.status, "synced", "{first:?}");

    // Teach merchant memory: categorize one committed "mock merchant 0" txn.
    let category_id = create_category_impl(
        &state,
        CreateCategoryInput {
            parent_id: None,
            name: "Coffee".to_owned(),
            category_type: "expense".to_owned(),
            color: None,
            icon: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap()
    .category_id;
    let rows = transaction_page_impl(&state, page_query()).unwrap().rows;
    let taught = rows
        .iter()
        .find(|r| r.counterparty.as_deref() == Some("mock merchant 0"))
        .expect("synced txn carries its merchant");
    recategorize_transaction_impl(
        &state,
        taught.transaction_id.clone(),
        Some(category_id.clone()),
        String::new(),
    )
    .unwrap();

    // Map the card and sync again: its "mock merchant 0" txn must arrive
    // categorized, and the outcome must say so.
    let card = make_account(&state, "Card");
    map_account(&state, &connection_id, "mock-acct-card", &card);
    let second = sync(&state, &adapter, &connection_id);
    assert_eq!(second.status, "synced", "{second:?}");
    assert!(
        second
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("auto-categorized"),
        "{second:?}"
    );
    let rows = transaction_page_impl(&state, page_query()).unwrap().rows;
    let auto = rows
        .iter()
        .filter(|r| r.counterparty.as_deref() == Some("mock merchant 0"))
        .collect::<Vec<_>>();
    assert_eq!(auto.len(), 2);
    assert!(
        auto.iter()
            .all(|r| r.category_id.as_deref() == Some(category_id.as_str())),
        "both merchant-0 rows categorized: {auto:?}"
    );
}

/// personal-cfo-tevp, direction 1: syncing over a manually recorded
/// transaction flags the collision for review instead of double-booking.
#[test]
fn sync_flags_a_manually_recorded_duplicate() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let checking = make_account(&state, "Checking");
    // The user already typed the fixture's July 15 expense by hand.
    record_transaction_impl(
        &state,
        RecordTransactionInput {
            account_id: checking.clone(),
            amount: MoneyDto {
                minor_units: -1250,
                currency: "USD".to_owned(),
            },
            occurred_at: "2026-07-15T12:00:00Z".to_owned(),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    map_account(&state, &connection_id, "mock-acct-checking", &checking);

    let result = sync(&state, &adapter, &connection_id);
    // 3 fixture txns for checking: the manual collision flags, the rest commit.
    assert_eq!(result.flagged, 1, "{result:?}");
    assert_eq!(result.committed, 2, "{result:?}");
    let inbox = money_inbox_list_impl(&state).unwrap();
    let item = inbox
        .iter()
        .find(|i| i.item_kind == "imported_waiting_commit")
        .expect("cross-source collision waits in the inbox");
    assert!(
        item.payload_json.contains("already in this account"),
        "{item:?}"
    );
    // The review panel gets the matched counterpart even though the
    // fingerprints can never match across sources.
    assert!(
        item.payload_json.contains("suspected_committed_txn_id"),
        "{item:?}"
    );

    // Re-syncs must NOT mint a fresh inbox item per rewind re-fetch while the
    // collision sits unresolved (review blocker): the flagged fingerprint is
    // tracked, so the second sync skips it silently.
    let second = sync(&state, &adapter, &connection_id);
    assert_eq!(second.flagged, 0, "{second:?}");
    assert_eq!(second.committed, 0, "{second:?}");
    let items: Vec<_> = money_inbox_list_impl(&state)
        .unwrap()
        .into_iter()
        .filter(|i| i.item_kind == "imported_waiting_commit")
        .collect();
    assert_eq!(
        items.len(),
        1,
        "exactly one item for one collision: {items:?}"
    );
}

/// personal-cfo-tevp, direction 2: a CSV backfill over already-synced history
/// flags instead of double-booking (fingerprints could never match — the CSV
/// hashes its description, the connector its provider id).
#[test]
fn csv_backfill_flags_already_synced_rows() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let checking = make_account(&state, "Checking");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);
    let synced = sync(&state, &adapter, &connection_id);
    assert_eq!(synced.status, "synced", "{synced:?}");

    // A backfill export containing the synced July 15 expense.
    let csv = "date,description,amount\n2026-07-15,Mock purchase 0,-12.50\n";
    let result = import_batch_impl(
        &state,
        ImportBatchInput {
            data: csv.as_bytes().to_vec(),
            filename: Some("backfill.csv".to_owned()),
            target_account_id: checking,
            plugin_id: Some("generic-csv".to_owned()),
            column_mapping: None,
            default_currency: Some("USD".to_owned()),
            date_format: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    assert_eq!(result.staged, 1, "{result:?}");
    assert_eq!(result.committed, 0, "{result:?}");
    assert_eq!(result.flagged, 1, "{result:?}");
}

/// personal-cfo-xtz5 (ADR 0026 addendum): a manual future entry auto-matches
/// the synced transaction that realizes it — visible on the entries surface,
/// no longer projecting.
#[test]
fn synced_transaction_matches_a_manual_future_entry() {
    let (_dir, state) = open_state();
    let adapter = mock();
    let connection_id = link(&state, &adapter);
    let checking = make_account(&state, "Checking");
    map_account(&state, &connection_id, "mock-acct-checking", &checking);

    // The user expected this payment a day after the fixture's July 15 txn.
    create_manual_future_entry_impl(
        &state,
        app_lib::ipc::dto::CreateManualFutureEntryInput {
            amount: MoneyDto {
                minor_units: -1250,
                currency: "USD".to_owned(),
            },
            date: "2026-07-16".to_owned(),
            label: "Expected card charge".to_owned(),
            account_id: Some(checking.clone()),
        },
    )
    .unwrap();

    let result = sync(&state, &adapter, &connection_id);
    assert_eq!(result.status, "synced", "{result:?}");

    let entries = manual_future_entry_list_impl(&state).unwrap();
    let entry = entries
        .iter()
        .find(|e| e.label == "Expected card charge")
        .unwrap();
    assert!(
        entry.matched_transaction_id.is_some(),
        "the synced -12.50 on 2026-07-15 realizes the entry: {entry:?}"
    );
}
