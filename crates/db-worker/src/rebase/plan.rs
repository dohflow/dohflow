//! The rebase plan (personal-cfo-bxsbz.4; ADR 0073 section 1): which unpushed local commands are
//! re-applied on the new base and which are queued for the person.
//!
//! The rule is **auto iff** two predicates hold, and there is no third outcome:
//!
//! ```text
//! write_set_untouched_since(base_seq) && validates_against(the_new_base)
//! ```
//!
//! If either is false the command's whole **correlation group** is queued, together with every
//! causal descendant of a queued command. A group is never partially applied, silently merged, or
//! discarded. The untouched predicate is pure (it compares the command's recorded write-set row
//! keys with the rows the server's envelopes wrote), so it is decided here; whether a command
//! validates is only known by applying it, so the executor reports invalid commands back and the
//! plan is recomputed with them (see `Scratch::execute`).

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use super::RowKey;

/// An unpushed local command, as the rebase reads it back from the live vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxCommand {
    /// The command's immutable id.
    pub command_id: Uuid,
    /// The correlation group.
    pub correlation_id: Uuid,
    /// The command that caused this one, if any.
    pub causation_id: Option<Uuid>,
    /// The class-1 rows the command wrote when it was applied locally.
    pub write_set_keys: Vec<RowKey>,
}

impl OutboxCommand {
    /// Whether none of the command's recorded rows was written by the server's envelopes.
    #[must_use]
    pub fn untouched(&self, touched: &BTreeSet<RowKey>) -> bool {
        self.write_set_keys.iter().all(|key| !touched.contains(key))
    }
}

/// Why a command is queued rather than re-applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueCause {
    /// The server's envelopes wrote a row the command wrote.
    Touched,
    /// The command no longer validates on the new base.
    Invalid,
    /// It is not itself in conflict: it belongs to the correlation group of, or is a causal
    /// descendant of, the queued command `root`.
    Closure {
        /// The command whose queueing pulled this one in.
        root: Uuid,
    },
}

/// The ADR 0073 outcome a queued command is offered with (`outbox_queue.reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueReason {
    /// Both values are kept and the person chooses which wins (a touched row).
    Choose,
    /// The command's intent is prefilled in its editor for the person to repair (it no longer
    /// validates).
    Edit,
}

impl QueueReason {
    /// The `outbox_queue.reason` column value.
    #[must_use]
    pub const fn as_column(self) -> &'static str {
        match self {
            Self::Choose => "queue_choose",
            Self::Edit => "queue_edit",
        }
    }
}

/// What the plan does with one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Re-apply it under its original command id and metadata.
    Replay,
    /// Queue it for the person.
    Queue {
        /// Why it queues.
        cause: QueueCause,
        /// What the person is offered.
        reason: QueueReason,
    },
}

/// The four cells of the auto-iff rule for ONE command on its own (before the group closure).
/// A command that is invalid cannot be offered as a choice between two values, so it is an edit
/// whether or not it was also touched.
#[must_use]
pub const fn auto_iff(untouched: bool, valid: bool) -> Disposition {
    match (untouched, valid) {
        (true, true) => Disposition::Replay,
        (false, true) => Disposition::Queue {
            cause: QueueCause::Touched,
            reason: QueueReason::Choose,
        },
        (_, false) => Disposition::Queue {
            cause: QueueCause::Invalid,
            reason: QueueReason::Edit,
        },
    }
}

/// One command and what the plan does with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedCommand {
    /// The command.
    pub command: OutboxCommand,
    /// Its disposition.
    pub disposition: Disposition,
}

/// The plan: every unpushed command, in local order, with its disposition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The commands in the order they were applied locally.
    pub commands: Vec<PlannedCommand>,
}

impl Plan {
    /// The commands to re-apply, in local order.
    pub fn replayed(&self) -> impl Iterator<Item = &OutboxCommand> {
        self.commands
            .iter()
            .filter_map(|planned| match planned.disposition {
                Disposition::Replay => Some(&planned.command),
                Disposition::Queue { .. } => None,
            })
    }

    /// The commands to queue, in local order, with why.
    pub fn queued(&self) -> impl Iterator<Item = (&OutboxCommand, QueueCause, QueueReason)> {
        self.commands
            .iter()
            .filter_map(|planned| match planned.disposition {
                Disposition::Queue { cause, reason } => Some((&planned.command, cause, reason)),
                Disposition::Replay => None,
            })
    }
}

/// Plan the rebase of `outbox` (in local order) against the rows the server's envelopes `touched`,
/// treating the commands in `invalid` as no longer validating on the new base.
///
/// A command is queued if it is touched or invalid, and then so is everything in its closure: the
/// other commands of its correlation group, and every command caused (directly or through a chain)
/// by one that queues. The closure is a fixed point, so a descendant's own correlation siblings
/// queue too. Each closure member records the first root (in local order) that pulled it in, and
/// inherits that root's reason.
#[must_use]
pub fn plan(
    outbox: &[OutboxCommand],
    touched: &BTreeSet<RowKey>,
    invalid: &BTreeSet<Uuid>,
) -> Plan {
    let mut disposition: BTreeMap<Uuid, Disposition> = BTreeMap::new();
    let mut roots = Vec::new();
    for command in outbox {
        let own = auto_iff(
            command.untouched(touched),
            !invalid.contains(&command.command_id),
        );
        if let Disposition::Queue { .. } = own {
            disposition.insert(command.command_id, own);
            roots.push(command.command_id);
        }
    }
    // Close over correlation groups and causation, root by root in local order so the recorded
    // root is deterministic.
    for root in roots {
        let Disposition::Queue { reason, .. } = disposition[&root] else {
            continue;
        };
        let mut frontier = vec![root];
        while let Some(current) = frontier.pop() {
            let Some(seed) = outbox.iter().find(|c| c.command_id == current) else {
                continue;
            };
            for other in outbox {
                let sibling = other.correlation_id == seed.correlation_id;
                let descendant = other.causation_id == Some(current);
                if (sibling || descendant) && !disposition.contains_key(&other.command_id) {
                    disposition.insert(
                        other.command_id,
                        Disposition::Queue {
                            cause: QueueCause::Closure { root },
                            reason,
                        },
                    );
                    frontier.push(other.command_id);
                }
            }
        }
    }
    Plan {
        commands: outbox
            .iter()
            .map(|command| PlannedCommand {
                command: command.clone(),
                disposition: disposition
                    .get(&command.command_id)
                    .copied()
                    .unwrap_or(Disposition::Replay),
            })
            .collect(),
    }
}
