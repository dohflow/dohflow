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

use finance_kernel::{
    BackupCadence, BackupHistoryKind, BackupRun, BackupSink, BackupSinkEntry, BackupSinkError,
    FolderSink, Kernel, PutMode, PutOutcome,
};
use tempfile::tempdir;

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

/// How many times an uncancelled export asks whether to stop.
fn checkpoints_of_an_uncancelled_run() -> usize {
    let root = tempdir().unwrap();
    let destination = root.path().join("backups");
    fs::create_dir(&destination).unwrap();
    let kernel = vault_with_schedule(root.path(), &destination);
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
    let total = checkpoints_of_an_uncancelled_run();
    let root = tempdir().unwrap();
    let destination = root.path().join("backups");
    fs::create_dir(&destination).unwrap();
    let kernel = vault_with_schedule(root.path(), &destination);
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
