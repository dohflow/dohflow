//! The household timezone changes only through the logged `SetHouseholdTimezone`
//! command (personal-cfo-g3m.8): it carries full `CommandMeta`, writes an operation-log
//! row and an audit event, replays on an idempotent retry, and refuses a bad zone with a
//! typed error and no write.

use finance_kernel::{
    new_idempotency_key, ActorType, CommandEnvelope, CommandMeta, Kernel, KernelError, Outcome,
    SetHouseholdTimezone,
};
use tempfile::TempDir;
use uuid::Uuid;

const EVENT: &str = "household_timezone_changed";

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: new_idempotency_key(),
        issued_at: finance_kernel::IssuedAt::now(),
    }
}

fn kernel() -> (TempDir, Kernel) {
    let dir = TempDir::new().unwrap();
    let kernel = Kernel::open(dir.path().join("vault.db"), "correct horse battery staple").unwrap();
    (dir, kernel)
}

#[test]
fn a_change_logs_an_operation_and_an_audit_event() {
    let (_dir, kernel) = kernel();
    assert_eq!(kernel.vault_metadata().unwrap().household_timezone, "UTC");

    kernel
        .set_household_timezone(&meta(), "America/Los_Angeles")
        .unwrap();

    assert_eq!(
        kernel.vault_metadata().unwrap().household_timezone,
        "America/Los_Angeles"
    );
    assert_eq!(kernel.operation_count().unwrap(), 1);
    assert_eq!(kernel.audit_event_count(EVENT).unwrap(), 1);
}

#[test]
fn the_command_can_be_dispatched_directly_and_replays_on_a_retry() {
    let (_dir, kernel) = kernel();
    let m = meta();
    let first = kernel
        .dispatch(CommandEnvelope::new(
            m.clone(),
            SetHouseholdTimezone::new("Asia/Tokyo"),
        ))
        .unwrap();
    let second = kernel
        .dispatch(CommandEnvelope::new(
            m,
            SetHouseholdTimezone::new("Asia/Tokyo"),
        ))
        .unwrap();
    assert!(matches!(first, Outcome::Applied { .. }));
    assert!(matches!(second, Outcome::Replayed { .. }));
    assert_eq!(kernel.operation_count().unwrap(), 1);
    assert_eq!(kernel.audit_event_count(EVENT).unwrap(), 1);
}

#[test]
fn a_bad_zone_is_a_typed_validation_error_and_writes_nothing() {
    let (_dir, kernel) = kernel();
    for bad in ["Not/A_Real_Zone", "", "   "] {
        let err = kernel.set_household_timezone(&meta(), bad).unwrap_err();
        assert!(
            matches!(err, KernelError::Validation(_)),
            "{bad:?}: {err:?}"
        );
    }
    assert_eq!(kernel.vault_metadata().unwrap().household_timezone, "UTC");
    assert_eq!(kernel.operation_count().unwrap(), 0);
    assert_eq!(kernel.audit_event_count(EVENT).unwrap(), 0);
}

#[test]
fn a_key_that_is_not_a_uuid_v4_is_refused_like_every_other_command() {
    let (_dir, kernel) = kernel();
    let mut bad = meta();
    bad.idempotency_key = format!("connector-refresh-{}", Uuid::new_v4());
    let err = kernel
        .set_household_timezone(&bad, "Asia/Tokyo")
        .unwrap_err();
    assert!(matches!(err, KernelError::InvalidIdempotencyKey), "{err:?}");
    assert_eq!(kernel.vault_metadata().unwrap().household_timezone, "UTC");
}
