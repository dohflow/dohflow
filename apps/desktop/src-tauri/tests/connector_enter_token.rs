//! A connection that arrived from another device (personal-cfo-07u): identity and mapping here,
//! no token. It lists as needing one, never refreshes, and entering a token writes only the local
//! secret row. Real temp vault and the in-memory mock connector; no network.

use std::sync::OnceLock;

use app_lib::connector_refresh::run_due_connector_refresh;
use app_lib::ipc::commands::{
    connector_connections_impl, connector_enter_token_impl, connector_enter_token_registered_impl,
    connector_sync_impl,
};
use app_lib::ipc::dto::{ConnectorEnterTokenInput, ConnectorSyncInput};
use app_lib::ipc::IpcError;
use app_lib::AppState;
use chrono::{DateTime, Utc};
use connector_core::mock::{mock_registration, MockConnector};
use connector_core::{ConnectorAdapter, ConnectorRegistration};
use finance_kernel::{Clock, VaultController};
use tempfile::TempDir;
use uuid::Uuid;

struct At(DateTime<Utc>);
impl Clock for At {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn open_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(b"test-key").expect("create vault");
    (dir, AppState::new(controller))
}

fn mock() -> &'static MockConnector {
    static ADAPTER: OnceLock<MockConnector> = OnceLock::new();
    ADAPTER.get_or_init(|| MockConnector::with_fixture().with_id("other"))
}

fn resolve(_: &str) -> Option<&'static dyn ConnectorAdapter> {
    Some(mock())
}

fn resolve_registration(id: &str) -> Option<&'static ConnectorRegistration> {
    static REGISTRATION: OnceLock<&'static ConnectorRegistration> = OnceLock::new();
    (id == "other").then(|| {
        *REGISTRATION
            .get_or_init(|| mock_registration(MockConnector::with_fixture().with_id("other"), true))
    })
}

/// A connection created on another device: identity only.
fn arrived(state: &AppState, adapter_id: &str) -> Uuid {
    let id = Uuid::now_v7();
    state
        .lock_controller()
        .unwrap()
        .kernel()
        .unwrap()
        .create_connector_identity(id, adapter_id, Some("Primary"))
        .unwrap();
    id
}

fn input(id: Uuid, token: &str) -> ConnectorEnterTokenInput {
    ConnectorEnterTokenInput {
        connection_id: id.to_string(),
        setup_token: token.to_owned(),
    }
}

fn dto(state: &AppState, id: Uuid) -> app_lib::ipc::dto::ConnectorConnectionDto {
    connector_connections_impl(state)
        .unwrap()
        .into_iter()
        .find(|c| c.id == id.to_string())
        .expect("listed")
}

#[test]
fn a_connection_without_a_token_lists_as_needing_one_and_never_refreshes() {
    let (_dir, state) = open_state();
    let id = arrived(&state, "other");
    assert!(dto(&state, id).needs_token);

    // The sweep creates its job and then skips it: no provider call, no sync stamped.
    let report = run_due_connector_refresh(&state, resolve, "w1", &At(Utc::now())).expect("sweep");
    assert_eq!((report.refreshed, report.failed), (0, 0));
    assert_eq!(dto(&state, id).last_synced_at, None);

    // A manual sync says to enter the token first and changes nothing.
    let error = connector_sync_impl(
        &state,
        mock(),
        ConnectorSyncInput {
            connection_id: id.to_string(),
            idempotency_key: Uuid::new_v4().to_string(),
        },
    )
    .unwrap_err();
    assert!(
        matches!(&error, IpcError::Validation(m) if m.contains("enter it first")),
        "{error:?}"
    );
    assert_eq!(dto(&state, id).last_synced_at, None);
}

#[test]
fn entering_a_token_writes_only_the_local_secret_and_then_refresh_runs() {
    let (_dir, state) = open_state();
    let id = arrived(&state, "other");

    connector_enter_token_impl(&state, mock(), input(id, "mock-setup-token")).unwrap();

    let row = dto(&state, id);
    assert!(!row.needs_token);
    let all = connector_connections_impl(&state).unwrap();
    assert_eq!(all.len(), 1, "no second identity");
    // Now the sweep refreshes it.
    let report = run_due_connector_refresh(&state, resolve, "w1", &At(Utc::now())).expect("sweep");
    assert_eq!(report.refreshed, 1);
    // A discovery pass proves the connection healthy without stamping a sync (nothing is mapped).
    assert_eq!(dto(&state, id).last_error, None);
}

#[test]
fn entering_a_token_is_refused_for_an_unknown_connection_and_for_another_provider() {
    let (_dir, state) = open_state();
    let unknown = Uuid::now_v7();
    assert!(matches!(
        connector_enter_token_impl(&state, mock(), input(unknown, "mock-setup-token")),
        Err(IpcError::Validation(m)) if m.contains("unknown")
    ));
    let other_provider = arrived(&state, "simplefin");
    assert!(matches!(
        connector_enter_token_impl(&state, mock(), input(other_provider, "mock-setup-token")),
        Err(IpcError::Validation(m)) if m.contains("another provider")
    ));
    assert!(
        dto(&state, other_provider).needs_token,
        "nothing was stored"
    );
}

#[test]
fn the_registered_path_resolves_the_connections_own_provider() {
    let (_dir, state) = open_state();
    let id = arrived(&state, "other");
    connector_enter_token_registered_impl(
        &state,
        resolve_registration,
        input(id, "mock-setup-token"),
    )
    .unwrap();
    assert!(!dto(&state, id).needs_token);
    // A provider the registry does not know is refused before any token is claimed.
    let unknown_provider = arrived(&state, "no-such-provider");
    assert!(matches!(
        connector_enter_token_registered_impl(
            &state,
            resolve_registration,
            input(unknown_provider, "mock-setup-token")
        ),
        Err(IpcError::Validation(_))
    ));
    assert!(dto(&state, unknown_provider).needs_token);
}

#[test]
fn the_token_input_never_prints_its_secret() {
    let shown = format!("{:?}", input(Uuid::nil(), "super-secret-setup-token"));
    assert!(!shown.contains("super-secret"), "{shown}");
}
