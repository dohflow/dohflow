//! No-reset-warning acknowledgement audit (ADR 0002 / personal-cfo-n7bo).
//!
//! Recording the acknowledgement persists an immutable audit event (carrying the
//! command + correlation provenance and a timestamp) that survives close/reopen,
//! so onboarding can prove the user saw and accepted the no-password-reset
//! warning before committing real data.

use finance_kernel::{ActorType, CommandMeta, Kernel, NO_RESET_WARNING_ACKNOWLEDGED};
use uuid::Uuid;

const PW: &[u8] = b"audit ack password";

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "n7bo".to_owned(),
        idempotency_key: Uuid::now_v7().to_string(),
    }
}

#[test]
fn acknowledgement_records_a_durable_audit_event() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("vault.db");
    let kernel = Kernel::create_vault(&db_path, PW).unwrap();

    // Nothing acknowledged on a fresh vault.
    assert_eq!(
        kernel
            .audit_event_count(NO_RESET_WARNING_ACKNOWLEDGED)
            .unwrap(),
        0
    );

    kernel.acknowledge_no_reset_warning(&meta()).unwrap();
    assert_eq!(
        kernel
            .audit_event_count(NO_RESET_WARNING_ACKNOWLEDGED)
            .unwrap(),
        1,
        "acknowledgement was not persisted"
    );

    // The audit record is durable: it survives closing and re-opening the vault.
    drop(kernel);
    let reopened = Kernel::unlock_vault(&db_path, PW).unwrap();
    assert_eq!(
        reopened
            .audit_event_count(NO_RESET_WARNING_ACKNOWLEDGED)
            .unwrap(),
        1,
        "acknowledgement did not survive close/reopen"
    );
}
