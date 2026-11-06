//! The rebase executor (personal-cfo-bxsbz.4; ADR 0074 decision 2 step 4, ADR 0073).
//!
//! It accepts only commands in the plan and gives each the ordinary validated apply, with three
//! differences the [`StepContext`] states: its idempotency memo must be absent, the row records
//! the new base and the `op_seq` it replaced, and the rebuilt operation-log row, outbox entry and
//! (fresh) memo are written in the same transaction as the command, so the memo points at the
//! rebuilt `op_seq` and never at the pre-rebase number.
//!
//! **Validation decides the plan's second predicate.** A command is only known to validate by
//! applying it, and a correlation group is never partially applied, so the executor works on a
//! trial copy of the post-pull scratch vault. A command the new base refuses is reported back,
//! the plan is recomputed with it (its group now queues), and the trial starts again from the
//! post-pull copy. Each restart queues at least one more command, so it ends.

use std::collections::BTreeSet;
use std::fs;

use chrono::Utc;
use rusqlite::params;
use sync_core::envelope::{EnvelopePlaintext, IdempotencyKey, ImmutableMeta};
use uuid::Uuid;

use super::{
    is_command_refusal, plan, Disposition, OutboxCommand, Plan, QueueCause, RebaseError, Scratch,
    StepContext,
};
use crate::base_image::copy_file_synced;
use crate::{ActorType, CommandMeta, DbError, DbWorker, WriteCommand};

/// An unpushed local command with everything needed to re-apply or queue it: the ids and write
/// set the plan reads, its complete original metadata, and the payload exactly as it was recorded.
#[derive(Debug, Clone)]
pub struct LocalCommand {
    /// The plan's view: ids and recorded write-set row keys.
    pub command: OutboxCommand,
    /// The complete original metadata, `issued_at` included.
    pub meta: CommandMeta,
    /// The decoded command.
    pub cmd: WriteCommand,
    /// The command's `op_seq` in the live vault, the number the rebuilt row replaces.
    pub old_op_seq: i64,
    /// The canonical payload bytes recorded for it.
    pub payload: Vec<u8>,
    /// The payload schema version.
    pub payload_version: u32,
    /// The write-set hash recorded for it.
    pub write_set_hash: [u8; 32],
}

/// A command re-applied on the new base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replayed {
    /// The command.
    pub command_id: Uuid,
    /// Its `op_seq` before the rebase.
    pub old_op_seq: i64,
    /// Its `op_seq` in the rebuilt vault.
    pub new_op_seq: i64,
}

/// What a rebase did with the unpushed local commands.
#[derive(Debug, Clone)]
pub struct Executed {
    /// The final plan, after any command the new base refused moved its group to the queue.
    pub plan: Plan,
    /// The commands re-applied, in local order.
    pub replayed: Vec<Replayed>,
}

impl Scratch<'_> {
    /// Re-apply the unpushed local commands on the new base and queue the rest, once.
    ///
    /// `local` is the whole outbox in local order. The scratch vault must have had the server's
    /// envelopes applied ([`Scratch::apply_pulled`]); it is replaced by the rebuilt vault, whose
    /// operation log, memos and outbox hold exactly the re-applied commands and whose
    /// `outbox_queue` holds the queued ones. Class 2 is rebuilt once at the end.
    ///
    /// # Errors
    /// [`RebaseError::AlreadyRebased`] on a second call, [`RebaseError::UnsealableLocalCommand`]
    /// for a command whose metadata cannot be recorded, or [`RebaseError::Db`] for a failure of
    /// the database or the filesystem. A command the new base refuses is not an error: it queues.
    pub fn rebase_local(&mut self, local: &[LocalCommand]) -> Result<Executed, RebaseError> {
        if self.rebased {
            return Err(RebaseError::AlreadyRebased);
        }
        // A consistent copy of the scratch vault as it stands after the pulled envelopes.
        self.worker.checkpoint_truncate()?;
        let post_pull = self.path.with_extension("post-pull.db");
        fs::remove_file(&post_pull).ok();
        copy_file_synced(&self.path, &post_pull)?;

        let outbox: Vec<OutboxCommand> = local.iter().map(|c| c.command.clone()).collect();
        let mut invalid: BTreeSet<Uuid> = BTreeSet::new();
        let result = self.run_trials(local, &outbox, &post_pull, &mut invalid);
        fs::remove_file(&post_pull).ok();
        let (trial_path, trial, executed) = result?;

        // The trial becomes the scratch vault; the pre-trial copy is no longer needed.
        let old_path = std::mem::replace(&mut self.path, trial_path);
        drop(std::mem::replace(&mut self.worker, trial));
        fs::remove_file(&old_path).ok();
        self.rebased = true;
        Ok(executed)
    }

    fn run_trials(
        &self,
        local: &[LocalCommand],
        outbox: &[OutboxCommand],
        post_pull: &std::path::Path,
        invalid: &mut BTreeSet<Uuid>,
    ) -> Result<(std::path::PathBuf, DbWorker, Executed), RebaseError> {
        let mut attempt = 0_u32;
        loop {
            attempt += 1;
            let planned = plan(outbox, &self.touched, invalid);
            let trial_path = self.path.with_extension(format!("trial-{attempt}.db"));
            fs::remove_file(&trial_path).ok();
            copy_file_synced(post_pull, &trial_path)?;
            let trial = self.live.open_scratch(&trial_path)?;
            match self.replay_planned(&trial, local, &planned) {
                Ok(replayed) => {
                    queue_planned(&trial, local, &planned, self.base_seq, self.head_seq)?;
                    rebuild_class_2(&trial)?;
                    return Ok((
                        trial_path,
                        trial,
                        Executed {
                            plan: planned,
                            replayed,
                        },
                    ));
                }
                Err(Refused::Command(command_id)) => {
                    // The new base refuses this command: its group queues, from a fresh copy.
                    drop(trial);
                    fs::remove_file(&trial_path).ok();
                    invalid.insert(command_id);
                }
                Err(Refused::Abort(error)) => {
                    drop(trial);
                    fs::remove_file(&trial_path).ok();
                    return Err(error);
                }
            }
        }
    }

    /// Apply every planned command, in local order, to `trial`.
    fn replay_planned(
        &self,
        trial: &DbWorker,
        local: &[LocalCommand],
        planned: &Plan,
    ) -> Result<Vec<Replayed>, Refused> {
        let mut replayed = Vec::new();
        for entry in &planned.commands {
            if entry.disposition != Disposition::Replay {
                continue;
            }
            let Some(command) = local
                .iter()
                .find(|c| c.command.command_id == entry.command.command_id)
            else {
                return Err(Refused::Abort(RebaseError::Db(DbError::SelfTestFailed(
                    "a planned command is not in the outbox".to_owned(),
                ))));
            };
            let step = StepContext {
                applied_base_seq: Some(self.head_seq),
                rebased_from: Some(command.old_op_seq),
                record_outbox: true,
            };
            match trial.dispatch_rebase_step(command.meta.clone(), &command.cmd, step) {
                Ok(outcome) => {
                    let crate::Outcome::Applied { op_seq } = outcome else {
                        return Err(Refused::Abort(RebaseError::MemoPresent {
                            command_id: command.meta.command_id,
                        }));
                    };
                    replayed.push(Replayed {
                        command_id: command.meta.command_id,
                        old_op_seq: command.old_op_seq,
                        new_op_seq: op_seq,
                    });
                }
                Err(RebaseError::Db(error)) if is_command_refusal(&error) => {
                    return Err(Refused::Command(command.meta.command_id));
                }
                Err(other) => return Err(Refused::Abort(other)),
            }
        }
        Ok(replayed)
    }
}

/// Why a trial stopped: the new base refused a command (the plan moves), or the rebase cannot go on.
enum Refused {
    Command(Uuid),
    Abort(RebaseError),
}

/// Write the queued commands into the trial's `outbox_queue`, preserving each one's envelope and
/// the evidence for why it queued. No memo is installed for them: a retry is told
/// `QueuedForDisposition`.
fn queue_planned(
    trial: &DbWorker,
    local: &[LocalCommand],
    planned: &Plan,
    base_seq: i64,
    head_seq: i64,
) -> Result<(), RebaseError> {
    let node = trial.node_id;
    let guard = trial.lock();
    let tx = guard.conn.unchecked_transaction()?;
    let now = Utc::now().to_rfc3339();
    for (queued, cause, reason) in planned.queued() {
        let Some(command) = local
            .iter()
            .find(|c| c.command.command_id == queued.command_id)
        else {
            continue;
        };
        let envelope = preserved_envelope(command, node)?;
        let (why, root) = match cause {
            QueueCause::Touched => ("touched", None),
            QueueCause::Invalid => ("invalid", None),
            QueueCause::Closure { root } => ("closure", Some(root)),
        };
        let evidence = serde_json::json!({
            "why": why,
            "root": root.map(|id| id.to_string()),
            "base_seq": base_seq,
            "rebased_onto_seq": head_seq,
            "op_seq_before": command.old_op_seq,
            "write_set_hash": command.write_set_hash.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        });
        tx.execute(
            "INSERT INTO outbox_queue
               (command_id, correlation_id, causation_id, reason, origin_node_id, queued_at,
                envelope, audit_evidence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                command.meta.command_id,
                command.meta.correlation_id,
                command.meta.causation_id,
                reason.as_column(),
                node,
                now,
                envelope,
                evidence.to_string().into_bytes(),
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// The queued command's envelope as it will be sealed if the person keeps it: the canonical
/// plaintext of its complete original metadata, write-set hash and payload.
fn preserved_envelope(command: &LocalCommand, node: Uuid) -> Result<Vec<u8>, RebaseError> {
    let unsealable = || RebaseError::UnsealableLocalCommand {
        command_id: command.meta.command_id,
    };
    let actor_type = match command.meta.actor_type {
        ActorType::User => 1,
        ActorType::System => 2,
        ActorType::Importer => 3,
        ActorType::Agent => 4,
    };
    let plaintext = EnvelopePlaintext {
        meta: ImmutableMeta {
            command_id: *command.meta.command_id.as_bytes(),
            correlation_id: *command.meta.correlation_id.as_bytes(),
            causation_id: command.meta.causation_id.map(|id| *id.as_bytes()),
            actor_type,
            actor_id: command.meta.actor_id.clone(),
            idempotency_key: IdempotencyKey::parse(&command.meta.idempotency_key)
                .map_err(|_| unsealable())?,
            issued_at: command.meta.issued_at.as_unix_micros(),
            device_id: *node.as_bytes(),
        },
        write_set_hash: command.write_set_hash,
        delta_manifest: sync_core::envelope::ArtifactManifest::new(Vec::new())
            .map_err(|_| unsealable())?,
        payload: command.payload.clone(),
    };
    plaintext.encode().map_err(|_| unsealable())
}

/// Rebuild class 2 once, after the replay, through the existing full-rebuild entry points. No
/// rebuild inside an apply moved (bxsbz.4 AC6).
fn rebuild_class_2(trial: &DbWorker) -> Result<(), DbError> {
    trial.rebuild_commitments()?;
    trial.rebuild_money_inbox()?;
    trial.rebuild_recurring_instances()?;
    trial.rebuild_transaction_display()?;
    Ok(())
}
