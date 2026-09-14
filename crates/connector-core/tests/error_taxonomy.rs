//! The ADR 0060 §5 error taxonomy, driven end to end through the mock's
//! failure-injection modes — the states the connection-health surface
//! (personal-cfo-ul5d) and connector-error inbox items (personal-cfo-zfyo)
//! will build on. Rate-limited is a *healthy* state and must stay
//! distinguishable from expired/needs-user-action.

use connector_core::mock::{FailureMode, MockConnector};
use connector_core::{
    Connection, ConnectorAdapter, ConnectorError, Credential, HealthStatus, LinkInput, LinkSession,
};

fn conn() -> Connection {
    Connection {
        credential: Credential::new("mock-access-url"),
    }
}

#[test]
fn each_failure_mode_maps_to_its_typed_error_and_health_status() {
    let cases = [
        (FailureMode::RateLimited, "rate_limited"),
        (FailureMode::Expired, "expired"),
        (FailureMode::NeedsUserAction, "needs_user_action"),
    ];
    for (mode, label) in cases {
        let adapter = MockConnector::failing_with(mode);
        let err = adapter.fetch_accounts(&conn()).unwrap_err();
        let health = adapter.health(&conn()).unwrap();
        match mode {
            FailureMode::RateLimited => {
                assert!(matches!(err, ConnectorError::RateLimited(_)), "{label}");
                assert_eq!(health, HealthStatus::RateLimited, "{label}");
            }
            FailureMode::Expired => {
                assert!(matches!(err, ConnectorError::Expired(_)), "{label}");
                assert_eq!(health, HealthStatus::Expired, "{label}");
            }
            FailureMode::NeedsUserAction => {
                assert!(
                    matches!(err, ConnectorError::NeedsUserAction { .. }),
                    "{label}"
                );
                assert!(
                    matches!(health, HealthStatus::NeedsUserAction { .. }),
                    "{label}"
                );
            }
            FailureMode::None => unreachable!(),
        }
    }
}

#[test]
fn a_healthy_connection_reports_healthy() {
    let adapter = MockConnector::with_fixture();
    assert_eq!(adapter.health(&conn()).unwrap(), HealthStatus::Healthy);
}

#[test]
fn link_accepts_the_valid_token_and_rejects_the_rest() {
    let adapter = MockConnector::with_fixture();

    // No token at all → prompt for one.
    let err = adapter.link(&LinkInput::default()).unwrap_err();
    assert!(
        matches!(&err, ConnectorError::NeedsUserAction { code, .. } if code == "token.missing")
    );

    // Wrong/reused token → the single-use re-link prompt (SimpleFIN's 403).
    let reused = LinkInput {
        user_token: Some(Credential::new("already-claimed-token")),
        ..LinkInput::default()
    };
    let err = adapter.link(&reused).unwrap_err();
    assert!(
        matches!(&err, ConnectorError::NeedsUserAction { code, .. } if code == "setup_token_used")
    );

    // The accepted token → an established credential.
    let good = LinkInput {
        user_token: Some(Credential::new("mock-setup-token")),
        ..LinkInput::default()
    };
    match adapter.link(&good).unwrap() {
        LinkSession::Established { display_hint, .. } => {
            assert!(display_hint.is_some());
        }
        LinkSession::ExternalAuth { .. } => panic!("user-token tier must establish directly"),
    }
}
