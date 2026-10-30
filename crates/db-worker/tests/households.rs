//! The household record (personal-cfo-339): the genesis seed, the eight commands, their write
//! sets and rejections, and the absence of any secret-bearing column.

mod common;

use common::*;
use db_worker::households::{household_id, primary_member_id};
use db_worker::{DbError, DbWorker, HouseholdRole, WriteCommand};
use uuid::Uuid;

fn count(worker: &DbWorker, sql: &str) -> i64 {
    worker
        .read_connection()
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

fn text(worker: &DbWorker, sql: &str) -> String {
    worker
        .read_connection()
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

fn tables(worker: &DbWorker) -> Vec<String> {
    let mut tables: Vec<String> = worker
        .last_write_set()
        .expect("a write set")
        .rows
        .into_iter()
        .map(|(table, _)| table)
        .collect();
    tables.dedup();
    tables
}

fn member(n: u128) -> Uuid {
    Uuid::from_u128(0xA000_0000_0000_4000_8000_0000_0000_0000 + n)
}

fn add(worker: &DbWorker, id: Uuid, name: &str, role: HouseholdRole) {
    worker
        .dispatch(
            meta(),
            WriteCommand::AddHouseholdMember {
                member_id: id,
                display_name: name.to_owned(),
                role,
            },
        )
        .unwrap();
}

fn rejected(worker: &DbWorker, command: WriteCommand) -> String {
    match worker.dispatch(meta(), command) {
        Err(DbError::InvalidCommand(why)) => why,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

fn dispatch(worker: &DbWorker, command: WriteCommand) {
    worker.dispatch(meta(), command).unwrap();
}

#[test]
fn a_fresh_vault_is_seeded_with_one_household_and_one_primary_from_its_own_id() {
    let (_dir, worker) = worker();
    let metadata = worker.vault_metadata().unwrap();
    assert_eq!(count(&worker, "SELECT COUNT(*) FROM households"), 1);
    assert_eq!(count(&worker, "SELECT COUNT(*) FROM household_members"), 1);
    assert_eq!(count(&worker, "SELECT COUNT(*) FROM profiles"), 0);
    let household = household_id(&metadata.vault_id);
    let primary = primary_member_id(&metadata.vault_id);
    assert_eq!(
        count(
            &worker,
            &format!(
                "SELECT COUNT(*) FROM households WHERE id = x'{}' AND name = 'My household'
                   AND household_timezone = 'UTC' AND platform_household_id IS NULL
                   AND created_at = updated_at",
                household.simple()
            )
        ),
        1
    );
    assert_eq!(
        count(
            &worker,
            &format!(
                "SELECT COUNT(*) FROM household_members WHERE id = x'{}' AND household_id = x'{}'
                   AND role = 'primary' AND display_name = 'Me' AND platform_member_id IS NULL
                   AND deleted_at IS NULL",
                primary.simple(),
                household.simple()
            )
        ),
        1
    );
    // Every column is a pure function of the vault: the seed's timestamps are the vault's own.
    assert_eq!(
        text(&worker, "SELECT created_at FROM households"),
        text(&worker, "SELECT created_at FROM vault_metadata")
    );
    assert_eq!(metadata.household_timezone, "UTC");
}

#[test]
fn changing_the_timezone_writes_exactly_the_households_row() {
    let (_dir, worker) = worker();
    set_household_timezone(&worker, "America/Chicago").unwrap();
    let write_set = worker.last_write_set().unwrap();
    assert_eq!(write_set.rows.len(), 1, "the whole write set is one row");
    assert_eq!(tables(&worker), ["households"]);
    assert_eq!(
        worker.vault_metadata().unwrap().household_timezone,
        "America/Chicago"
    );
    // The same zone again is an accepted no-op that writes no class-1 row.
    set_household_timezone(&worker, "America/Chicago").unwrap();
    assert!(worker.last_write_set().unwrap().rows.is_empty());
    // A name is not a zone: the household's other columns are untouched by the zone.
    assert_eq!(text(&worker, "SELECT name FROM households"), "My household");
}

#[test]
fn naming_the_household_writes_the_households_row_and_validates_the_name() {
    let (_dir, worker) = worker();
    dispatch(
        &worker,
        WriteCommand::SetHouseholdName {
            name: "  The Garcias  ".to_owned(),
        },
    );
    assert_eq!(text(&worker, "SELECT name FROM households"), "The Garcias");
    assert_eq!(tables(&worker), ["households"]);
    dispatch(
        &worker,
        WriteCommand::SetHouseholdName {
            name: "The Garcias".to_owned(),
        },
    );
    assert!(
        worker.last_write_set().unwrap().rows.is_empty(),
        "unchanged is a no-op"
    );
    for bad in ["", "   ", &"x".repeat(101), "tab\there"] {
        rejected(
            &worker,
            WriteCommand::SetHouseholdName {
                name: bad.to_owned(),
            },
        );
    }
}

#[test]
fn members_are_added_renamed_re_roled_and_given_a_profile_by_command() {
    let (_dir, worker) = worker();
    let sam = member(1);
    add(&worker, sam, "Sam", HouseholdRole::Partner);
    assert_eq!(tables(&worker), ["household_members"]);
    dispatch(
        &worker,
        WriteCommand::RenameHouseholdMember {
            member_id: sam,
            display_name: "Samuel".to_owned(),
        },
    );
    dispatch(
        &worker,
        WriteCommand::SetMemberRole {
            member_id: sam,
            role: HouseholdRole::Viewer,
        },
    );
    assert_eq!(
        text(
            &worker,
            &format!(
                "SELECT display_name || '/' || role FROM household_members WHERE id = x'{}'",
                sam.simple()
            )
        ),
        "Samuel/viewer"
    );
    dispatch(
        &worker,
        WriteCommand::SetMemberProfile {
            member_id: sam,
            initials: Some("SG".to_owned()),
            color: Some("#3366cc".to_owned()),
        },
    );
    assert_eq!(tables(&worker), ["profiles"]);
    // The same profile again is a no-op; clearing keeps the row with empty values.
    dispatch(
        &worker,
        WriteCommand::SetMemberProfile {
            member_id: sam,
            initials: Some("SG".to_owned()),
            color: Some("#3366cc".to_owned()),
        },
    );
    assert!(worker.last_write_set().unwrap().rows.is_empty());
    dispatch(
        &worker,
        WriteCommand::SetMemberProfile {
            member_id: sam,
            initials: None,
            color: None,
        },
    );
    assert_eq!(
        count(&worker, "SELECT COUNT(*) FROM profiles"),
        1,
        "a profile is never deleted"
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM profiles WHERE initials IS NULL AND color IS NULL"
        ),
        1
    );
}

#[test]
fn the_primary_seat_moves_only_by_an_atomic_state_validated_transfer() {
    let (_dir, worker) = worker();
    let primary = primary_member_id(&worker.vault_metadata().unwrap().vault_id);
    let (sam, kim) = (member(1), member(2));
    add(&worker, sam, "Sam", HouseholdRole::Partner);
    add(&worker, kim, "Kim", HouseholdRole::Viewer);

    // The primary seat is never added, re-roled or removed.
    rejected(
        &worker,
        WriteCommand::AddHouseholdMember {
            member_id: member(3),
            display_name: "X".to_owned(),
            role: HouseholdRole::Primary,
        },
    );
    rejected(
        &worker,
        WriteCommand::SetMemberRole {
            member_id: sam,
            role: HouseholdRole::Primary,
        },
    );
    rejected(
        &worker,
        WriteCommand::SetMemberRole {
            member_id: primary,
            role: HouseholdRole::Viewer,
        },
    );
    rejected(
        &worker,
        WriteCommand::RemoveHouseholdMember { member_id: primary },
    );

    dispatch(
        &worker,
        WriteCommand::TransferPrimary {
            from_member_id: primary,
            to_member_id: sam,
        },
    );
    assert_eq!(tables(&worker), ["household_members"]);
    assert_eq!(
        worker.last_write_set().unwrap().rows.len(),
        2,
        "both seats changed"
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM household_members WHERE role = 'primary' AND deleted_at IS NULL"
        ),
        1
    );
    assert_eq!(
        text(
            &worker,
            &format!(
                "SELECT role FROM household_members WHERE id = x'{}'",
                sam.simple()
            )
        ),
        "primary"
    );
    assert_eq!(
        text(
            &worker,
            &format!(
                "SELECT role FROM household_members WHERE id = x'{}'",
                primary.simple()
            )
        ),
        "partner"
    );

    // A stale transfer (the old primary no longer holds the seat) is a rejection, never a second primary.
    let why = rejected(
        &worker,
        WriteCommand::TransferPrimary {
            from_member_id: primary,
            to_member_id: kim,
        },
    );
    assert!(why.contains("not the primary"), "{why}");
    rejected(
        &worker,
        WriteCommand::TransferPrimary {
            from_member_id: sam,
            to_member_id: sam,
        },
    );
    rejected(
        &worker,
        WriteCommand::TransferPrimary {
            from_member_id: sam,
            to_member_id: member(99),
        },
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM household_members WHERE role = 'primary' AND deleted_at IS NULL"
        ),
        1
    );
}

#[test]
fn removal_is_a_tombstone_with_a_restore_and_never_a_hard_delete() {
    let (_dir, worker) = worker();
    let sam = member(1);
    add(&worker, sam, "Sam", HouseholdRole::Partner);
    dispatch(
        &worker,
        WriteCommand::RemoveHouseholdMember { member_id: sam },
    );
    assert_eq!(
        count(&worker, "SELECT COUNT(*) FROM household_members"),
        2,
        "the row stays"
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM household_members WHERE deleted_at IS NOT NULL"
        ),
        1
    );
    // Removing again is an accepted no-op; a removed member cannot be renamed or re-roled.
    dispatch(
        &worker,
        WriteCommand::RemoveHouseholdMember { member_id: sam },
    );
    assert!(worker.last_write_set().unwrap().rows.is_empty());
    rejected(
        &worker,
        WriteCommand::RenameHouseholdMember {
            member_id: sam,
            display_name: "S".to_owned(),
        },
    );
    rejected(
        &worker,
        WriteCommand::SetMemberRole {
            member_id: sam,
            role: HouseholdRole::Viewer,
        },
    );
    rejected(
        &worker,
        WriteCommand::SetMemberProfile {
            member_id: sam,
            initials: None,
            color: None,
        },
    );
    rejected(
        &worker,
        WriteCommand::TransferPrimary {
            from_member_id: primary_member_id(&worker.vault_metadata().unwrap().vault_id),
            to_member_id: sam,
        },
    );
    dispatch(
        &worker,
        WriteCommand::RestoreHouseholdMember { member_id: sam },
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM household_members WHERE deleted_at IS NOT NULL"
        ),
        0
    );
    dispatch(
        &worker,
        WriteCommand::RestoreHouseholdMember { member_id: sam },
    );
    assert!(
        worker.last_write_set().unwrap().rows.is_empty(),
        "restoring a live member is a no-op"
    );
}

#[test]
fn bad_input_is_rejected_before_any_write() {
    let (_dir, worker) = worker();
    let sam = member(1);
    add(&worker, sam, "Sam", HouseholdRole::Partner);
    let logged = count(&worker, "SELECT COUNT(*) FROM operation_log");
    // A reused id, an unknown member, bad names, bad initials and colors.
    rejected(
        &worker,
        WriteCommand::AddHouseholdMember {
            member_id: sam,
            display_name: "Again".to_owned(),
            role: HouseholdRole::Viewer,
        },
    );
    rejected(
        &worker,
        WriteCommand::RenameHouseholdMember {
            member_id: member(77),
            display_name: "Z".to_owned(),
        },
    );
    rejected(
        &worker,
        WriteCommand::RenameHouseholdMember {
            member_id: sam,
            display_name: " ".to_owned(),
        },
    );
    for initials in ["", "ABCDE", "A B"] {
        rejected(
            &worker,
            WriteCommand::SetMemberProfile {
                member_id: sam,
                initials: Some(initials.to_owned()),
                color: None,
            },
        );
    }
    for color in ["red", "#12345", "#12345G", "123456", "#1234567"] {
        rejected(
            &worker,
            WriteCommand::SetMemberProfile {
                member_id: sam,
                initials: None,
                color: Some(color.to_owned()),
            },
        );
    }
    assert_eq!(
        count(&worker, "SELECT COUNT(*) FROM operation_log"),
        logged,
        "nothing was logged"
    );
}

#[test]
fn no_column_of_the_three_tables_can_hold_a_secret_and_the_platform_links_stay_null() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    for table in ["households", "household_members", "profiles"] {
        let mut stmt = conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(!columns.is_empty());
        for column in &columns {
            for secret in [
                "secret",
                "token",
                "password",
                "passphrase",
                "credential",
                "key",
                "hash",
                "salt",
                "nonce",
                "seed",
                "pin",
                "ssn",
                "email",
                "phone",
            ] {
                assert!(
                    !column.contains(secret),
                    "{table}.{column} looks secret-bearing ({secret})"
                );
            }
        }
    }
    // With no account, no command writes the platform links.
    let sam = member(1);
    add(&worker, sam, "Sam", HouseholdRole::Partner);
    dispatch(
        &worker,
        WriteCommand::SetMemberProfile {
            member_id: sam,
            initials: Some("S".to_owned()),
            color: None,
        },
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM households WHERE platform_household_id IS NOT NULL"
        ),
        0
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM household_members WHERE platform_member_id IS NOT NULL"
        ),
        0
    );
}

#[test]
fn the_vault_metadata_row_no_longer_carries_the_timezone() {
    let (_dir, worker) = worker();
    let has: i64 = count(
        &worker,
        "SELECT COUNT(*) FROM pragma_table_info('vault_metadata') WHERE name = 'household_timezone'",
    );
    assert_eq!(has, 0);
}
