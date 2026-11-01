//! Cancelling the scheduled backup export (personal-cfo-g3m.29).
//!
//! The export reads a cancel request at its checkpoints. These tests cancel at EVERY checkpoint, not
//! at a lucky moment: the closure counts how often it is asked and starts answering "cancelled" at the
//! k-th ask, for every k up to the number of asks an uncancelled run makes. At each k a cancelled run
//! must leave the destination exactly as it was (no whole or partial file, nothing temporary), no
//! receipt, and the next run must go through. A run that ends before the k-th ask completes with its
//! receipt and file, never half of that.

use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use chrono::{DateTime, Utc};
use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, BackupCadence, BackupHistoryKind, BackupRun,
    BackupSink, BackupSinkEntry, BackupSinkError, CashflowRole, CommandEnvelope, CommandMeta,
    CreateAccount, Currency, FolderSink, Kernel, LedgerAccountId, Money, PutMode, PutOutcome,
    RecordTransaction, TransactionId,
};
use observability::diagnostics::{parse_bundle, BundleHeader, Metric, Outcome, Value};
use tempfile::tempdir;
use uuid::Uuid;

const PASSWORD: &[u8] = b"correct horse battery";
const VERSION: &str = "0.0.0-test";

fn vault_with_schedule(root: &Path, destination: &Path) -> Kernel {
    let kernel = Kernel::create_vault(root.join("vault.db"), PASSWORD).unwrap();
    kernel
        .configure_backup_schedule(BackupCadence::Weekly, Some(destination))
        .unwrap();
    kernel
}

fn entries(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: Uuid::new_v4().to_string(),
        issued_at: finance_kernel::IssuedAt::now(),
    }
}

/// Give the vault `count` distinct attachments (each one is a blob the export reads, hashes and frames).
fn seed_attachments(kernel: &Kernel, count: usize) {
    if count == 0 {
        return;
    }
    let account = Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        "Checking",
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    );
    let account_id = account.id();
    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::with_opening_balance(account, Money::new(100_000, Currency::Usd)),
        ))
        .unwrap();
    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            RecordTransaction::new(
                TransactionId::new(),
                account_id,
                Money::new(-1_234, Currency::Usd),
                "2026-06-05T00:00:00Z".parse::<DateTime<Utc>>().unwrap(),
            ),
        ))
        .unwrap();
    let transaction = kernel.transactions(1).unwrap()[0].transaction_id;
    for n in 0..count {
        kernel
            .attach_document(
                transaction,
                format!("receipt number {n}").as_bytes(),
                Some("text/plain"),
                Some("receipt.txt"),
            )
            .unwrap();
    }
}

/// How many times an uncancelled export asks whether to stop, for a vault with `attachments` blobs.
fn checkpoints_with_attachments(attachments: usize) -> usize {
    let root = tempdir().unwrap();
    let destination = root.path().join("backups");
    fs::create_dir(&destination).unwrap();
    let kernel = vault_with_schedule(root.path(), &destination);
    seed_attachments(&kernel, attachments);
    let asked = Cell::new(0usize);
    let run = kernel
        .run_scheduled_backup_cancellable(VERSION, &|| {
            asked.set(asked.get() + 1);
            false
        })
        .unwrap();
    assert!(matches!(run, BackupRun::Done(_)));
    assert_eq!(entries(&destination).len(), 1);
    asked.get()
}

/// How many times an uncancelled export asks whether to stop, for a vault with no attachments.
fn checkpoints_of_an_uncancelled_run() -> usize {
    checkpoints_with_attachments(0)
}

#[test]
fn every_attachment_adds_its_read_its_manifest_hash_and_its_frame_to_the_checkpoints() {
    // One look before each blob is read, one after each is hashed into the manifest and one after each is
    // framed: three per attachment. A removed blob-read checkpoint (the slow one for a large vault) shows
    // as a count that is too small.
    let none = checkpoints_with_attachments(0);
    let three = checkpoints_with_attachments(3);
    assert_eq!(
        three - none,
        3 * 3,
        "{none} without, {three} with three blobs"
    );
}

#[test]
fn the_export_has_checkpoints_in_every_phase() {
    // Assemble (start, snapshot, manifest, frame, seal x2), the write (open + each chunk), the
    // read-back, the verification and the receipt: a run that asks fewer times has lost one.
    assert!(
        checkpoints_of_an_uncancelled_run() >= 10,
        "the export asks {} times",
        checkpoints_of_an_uncancelled_run()
    );
}

#[test]
fn a_cancel_at_any_checkpoint_leaves_the_destination_and_the_receipts_untouched() {
    // With attachments, so the checkpoints between blob reads are among the ones cancelled at.
    let total = checkpoints_with_attachments(2);
    let root = tempdir().unwrap();
    let destination = root.path().join("backups");
    fs::create_dir(&destination).unwrap();
    let kernel = vault_with_schedule(root.path(), &destination);
    seed_attachments(&kernel, 2);
    let before = entries(&destination);

    for k in 1..=total {
        let asked = Cell::new(0usize);
        let run = kernel
            .run_scheduled_backup_cancellable(VERSION, &|| {
                asked.set(asked.get() + 1);
                asked.get() >= k
            })
            .unwrap();

        assert_eq!(
            run,
            BackupRun::Cancelled,
            "cancel at checkpoint {k} of {total}"
        );
        assert_eq!(
            entries(&destination),
            before,
            "cancel at checkpoint {k} of {total}: the destination changed"
        );
        assert!(
            kernel.backup_history().unwrap().is_empty(),
            "cancel at checkpoint {k} of {total}: a receipt was recorded"
        );
    }

    // The next run proceeds normally, after the vault is closed and reopened.
    drop(kernel);
    let reopened = Kernel::unlock_vault(root.path().join("vault.db"), PASSWORD).unwrap();
    assert!(reopened.backup_history().unwrap().is_empty());
    let next = reopened
        .run_scheduled_backup_cancellable(VERSION, &|| false)
        .unwrap();
    let BackupRun::Done(entry) = next else {
        panic!("the run after the cancelled ones was cancelled");
    };
    assert!(entry.verified);
    assert_eq!(entry.kind, BackupHistoryKind::Scheduled);
    assert_eq!(entries(&destination).len(), 1);
    assert_eq!(reopened.backup_history().unwrap().len(), 1);
}

#[test]
fn a_cancel_after_the_last_checkpoint_is_too_late_and_the_run_is_whole() {
    let total = checkpoints_of_an_uncancelled_run();
    let root = tempdir().unwrap();
    let destination = root.path().join("backups");
    fs::create_dir(&destination).unwrap();
    let kernel = vault_with_schedule(root.path(), &destination);
    let asked = Cell::new(0usize);
    let run = kernel
        .run_scheduled_backup_cancellable(VERSION, &|| {
            asked.set(asked.get() + 1);
            asked.get() > total
        })
        .unwrap();
    assert!(matches!(run, BackupRun::Done(_)));
    assert_eq!(entries(&destination).len(), 1, "the file is there");
    assert_eq!(
        kernel.backup_history().unwrap().len(),
        1,
        "with its receipt"
    );
}

#[test]
fn a_folder_sink_stops_between_chunks_and_removes_its_partial_file() {
    let dir = tempdir().unwrap();
    let sink = FolderSink::new(dir.path());
    let package = vec![7u8; 3 << 20];
    let asked = Cell::new(0usize);
    // Asked once before opening and once per megabyte: stop at the third look, mid-write.
    let outcome = sink
        .put_cancellable("a.pcfobk", &package, PutMode::CreateNew, &|| {
            asked.set(asked.get() + 1);
            asked.get() >= 3
        })
        .unwrap();
    assert_eq!(outcome, PutOutcome::Cancelled);
    assert!(entries(dir.path()).is_empty(), "no partial file is left");

    let stored = sink
        .put_cancellable("b.pcfobk", &package, PutMode::CreateNew, &|| false)
        .unwrap();
    assert_eq!(stored, PutOutcome::Stored(package.len() as u64));
    assert_eq!(fs::read(dir.path().join("b.pcfobk")).unwrap(), package);
}

/// A cancel must not claim the destination is as it was when the partial file could not be removed:
/// that is a failed write, and the partial file stays visible. (Unix: a read-only folder refuses the
/// removal.)
#[cfg(unix)]
#[test]
fn a_cancel_whose_partial_file_cannot_be_removed_is_a_failure_not_a_clean_cancel() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().unwrap();
    let sink = FolderSink::new(dir.path());
    let package = vec![7u8; 3 << 20];
    let asked = Cell::new(0usize);
    let result = sink.put_cancellable("a.pcfobk", &package, PutMode::CreateNew, &|| {
        asked.set(asked.get() + 1);
        if asked.get() == 3 {
            // The cancel arrives mid-write, and the folder stops accepting removals.
            fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
            return true;
        }
        false
    });
    // Restore access so the temporary folder can be cleaned up whatever happened.
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(result, Err(BackupSinkError));
    assert_eq!(
        entries(dir.path()),
        vec!["a.pcfobk".to_owned()],
        "the partial file is still there, so nothing claims otherwise"
    );
}

#[test]
fn a_replace_never_stops_halfway_and_never_damages_the_item_it_replaces() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("keep.pcfobk"), b"the earlier backup").unwrap();
    let sink = FolderSink::new(dir.path());
    let package = vec![1u8; 3 << 20];

    // Cancelled before it touches anything: the earlier item is as it was.
    let outcome = sink
        .put_cancellable("keep.pcfobk", &package, PutMode::Replace, &|| true)
        .unwrap();
    assert_eq!(outcome, PutOutcome::Cancelled);
    assert_eq!(
        fs::read(dir.path().join("keep.pcfobk")).unwrap(),
        b"the earlier backup"
    );

    // A cancel that arrives once the replace has begun does not leave half a file in its place.
    let asked = Cell::new(0usize);
    let outcome = sink
        .put_cancellable("keep.pcfobk", &package, PutMode::Replace, &|| {
            asked.set(asked.get() + 1);
            asked.get() >= 3
        })
        .unwrap();
    assert_eq!(outcome, PutOutcome::Stored(package.len() as u64));
    assert_eq!(fs::read(dir.path().join("keep.pcfobk")).unwrap(), package);
}

/// A sink that cannot take an item back: [`BackupSink::discard_created`] keeps its refusing default.
/// It says when the write has finished.
struct KeepsEverything {
    inner: FolderSink,
    written: Rc<Cell<bool>>,
}

impl BackupSink for KeepsEverything {
    fn describe(&self, name: &str) -> String {
        self.inner.describe(name)
    }
    fn put(&self, name: &str, package: &[u8], mode: PutMode) -> Result<u64, BackupSinkError> {
        let size = self.inner.put(name, package, mode)?;
        self.written.set(true);
        Ok(size)
    }
    fn get(&self, name: &str) -> Result<Vec<u8>, BackupSinkError> {
        self.inner.get(name)
    }
    fn list(&self) -> Result<Vec<BackupSinkEntry>, BackupSinkError> {
        self.inner.list()
    }
}

#[test]
fn a_sink_that_cannot_undo_finishes_a_run_that_is_cancelled_after_the_write() {
    let root = tempdir().unwrap();
    let destination = root.path().join("backups");
    fs::create_dir(&destination).unwrap();
    let kernel = vault_with_schedule(root.path(), &destination);
    let written = Rc::new(Cell::new(false));
    let sink = KeepsEverything {
        inner: FolderSink::new(&destination),
        written: Rc::clone(&written),
    };

    // The request arrives the moment the write has finished. The sink cannot remove the item, so
    // the run completes with its receipt rather than leave a bundle nobody recorded.
    let flag = Rc::clone(&written);
    let run = kernel
        .run_backup_to_sink_cancellable(&sink, VERSION, BackupHistoryKind::Scheduled, &move || {
            flag.get()
        })
        .unwrap();
    assert!(matches!(run, BackupRun::Done(_)), "{run:?}");
    assert_eq!(entries(&destination).len(), 1);
    assert_eq!(kernel.backup_history().unwrap().len(), 1);
}

/// A measurement, not a gate (absolute timings fail under load): the export time on the fixture vault
/// through the entry point that never cancels and the one that does. Run by hand: `cargo test -p
/// finance-kernel --test backup_cancel --release -- --ignored --nocapture backup_export_time`. Both
/// now share the checkpointed code, so they agree trivially; the before/after for the change itself
/// was taken against main (16.36 ms before, 16.36 ms after, median of 25, release) and is in the PR.
#[test]
#[ignore = "a manual timing measurement (personal-cfo-g3m.29): run by hand with --nocapture"]
fn backup_export_time_with_and_without_checkpoints() {
    const RUNS: usize = 25;
    let root = tempdir().unwrap();
    let destination = root.path().join("backups");
    fs::create_dir(&destination).unwrap();
    let kernel = vault_with_schedule(root.path(), &destination);
    let median = |mut samples: Vec<std::time::Duration>| {
        samples.sort();
        samples[samples.len() / 2]
    };
    let mut plain = Vec::new();
    let mut checked = Vec::new();
    for _ in 0..RUNS {
        let started = std::time::Instant::now();
        kernel.run_scheduled_backup(VERSION).unwrap();
        plain.push(started.elapsed());
        let started = std::time::Instant::now();
        kernel
            .run_scheduled_backup_cancellable(VERSION, &|| false)
            .unwrap();
        checked.push(started.elapsed());
    }
    println!(
        "export median over {RUNS} runs: run_scheduled_backup {:?}, run_scheduled_backup_cancellable {:?}",
        median(plain),
        median(checked)
    );
}

// ---- the diagnostics counter (personal-cfo-g3m.29, 04b's F1) --------------------------------------

fn header() -> BundleHeader {
    BundleHeader {
        build_version: "0.2.0".into(),
        build_channel: "dev".into(),
        platform: "macos".into(),
        created_on: "2026-09-28".into(),
    }
}

fn backup_outcomes(kernel: &Kernel) -> Vec<Value> {
    parse_bundle(
        kernel
            .diagnostics()
            .preview(&header())
            .expect("preview")
            .bytes(),
    )
    .expect("bundle parses")
    .records
    .into_iter()
    .filter(|record| record.metric == Metric::BackupOutcome)
    .map(|record| record.value)
    .collect()
}

#[test]
fn a_cancelled_run_records_no_backup_outcome_and_a_completed_one_records_exactly_one_success() {
    let root = tempdir().unwrap();
    let destination = root.path().join("backups");
    fs::create_dir(&destination).unwrap();
    let kernel = vault_with_schedule(root.path(), &destination);
    assert!(backup_outcomes(&kernel).is_empty());

    // Cancelled at the very first look, mid-run and at the last look before the receipt: none of them is
    // a success or a failure for the diagnostics counter.
    let total = checkpoints_of_an_uncancelled_run();
    for k in [1, total / 2, total] {
        let asked = Cell::new(0usize);
        let run = kernel
            .run_scheduled_backup_cancellable(VERSION, &|| {
                asked.set(asked.get() + 1);
                asked.get() >= k
            })
            .unwrap();
        assert_eq!(run, BackupRun::Cancelled, "cancel at {k}");
        assert!(
            backup_outcomes(&kernel).is_empty(),
            "a cancelled run (at {k}) recorded an outcome: {:?}",
            backup_outcomes(&kernel)
        );
    }

    let run = kernel
        .run_scheduled_backup_cancellable(VERSION, &|| false)
        .unwrap();
    assert!(matches!(run, BackupRun::Done(_)));
    assert_eq!(
        backup_outcomes(&kernel),
        vec![Value::Outcome(Outcome::Success)],
        "exactly one success after a completed run"
    );
}
