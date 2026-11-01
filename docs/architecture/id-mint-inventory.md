# Id mint inventory (personal-cfo-32fmp.2; ADR 0013 Addendum A, section 10)

Every place `crates/db-worker/src` mints an identifier, and what kind of mint it is. The rule is
by SITE and by reachability from `apply_command`, not by count:

* **Inside `apply` (and every helper it reaches): DERIVE.** The id is a pure function of the
  command (`core_ids::DerivedIds`, one per command, created by `apply_command`). A mint that
  happens unconditionally once per payload element takes the next **ordinal**
  (`DerivedIds::next(tag)`); a mint under a **base-dependent branch** (it may or may not run,
  depending on rows that already exist) takes a **keyed** id (`DerivedIds::keyed(tag, natural_key)`,
  HMAC over the command id, the tag and the length-prefixed natural key) and consumes no ordinal, so
  the number of ordinals a command consumes depends only on its payload (section 10(a)).
* **A caller-minted payload id** (`RecordTransaction`, `CreateCategory`, `AddHouseholdMember` ...)
  is not minted in apply.
* **A genesis seeder, a device-local (class 3) row, a rebuilt (class 2) row, or a test:** KEEP the
  random or time-ordered id; none is replicated state decided by apply.

`tests/id_mint_inventory.rs` scans the source for the mint tokens (`Uuid::now_v7`, `Uuid::new_v4`,
`new_v7`, `ContextV7`, `next_id()`, and `<Entity>Id::new()`) and fails unless this table lists
every remaining site, with its count, and unless no site under `apply/` or in a helper module that
`apply` reaches (`debt.rs`, `money_inbox.rs`, `commitments.rs`) is listed. A new mint is therefore a
deliberate row here, in the same change.

## Remaining random mints (KEEP)

| File | Function | Mints | Class | Why it is not a derived id |
| --- | --- | --- | --- | --- |
| `connectors.rs` | `scrub_tombstoned_connector_secrets` | 2 | keep-class-3 | The post-rebase device-local cleanup writes a class-3 audit row for a system event; not reached from `apply_command`. |
| `connectors.rs` | `local_meta` | 2 | keep-caller | The producer's command metadata (a command id and a correlation id) for the connector writes, built at the `DbWorker` method; not a row `apply` writes and not reached from `apply_command`. |
| `forecast_persist.rs` | `capture_snapshot` | 1 | keep-class-2 | A rebuilt forecast input snapshot (class 2). |
| `forecast_persist.rs` | `persist_run_and_rows` | 1 | keep-class-2 | A rebuilt forecast run (class 2). |
| `forecast_persist.rs` | `insert_row` | 1 | keep-class-2 | A rebuilt forecast row (class 2). |
| `ingestion.rs` | `record_parser_run` | 1 | keep-class-3 | Device-local import staging. |
| `ingestion.rs` | `stage_transaction` | 1 | keep-class-3 | Device-local import staging. |
| `ingestion.rs` | `stage_account` | 1 | keep-class-3 | Device-local import staging. |
| `ingestion.rs` | `stage_balance` | 1 | keep-class-3 | Device-local import staging. |
| `lib.rs` | `new_idempotency_key` | 1 | keep-device | The producer's idempotency key; a command's metadata, not a row apply writes. |
| `lib.rs` | `import_attachment` | 1 | keep-caller | An attachment id minted by the import call before it is recorded; not an apply path. |
| `lib.rs` | `record_audit_event` | 1 | keep-class-3 | A device-local security audit row. |
| `lib.rs` | `stage_file_batch` | 1 | keep-class-3 | Device-local import staging. |
| `lib.rs` | `stage_sync_batch` | 1 | keep-class-3 | Device-local import staging. |
| `lib.rs` | `clone_scenario` | 1 | keep-pending | A direct writer outside the bus; `personal-cfo-5ymg8.5` routes it as a command with a derived id. |
| `lib.rs` | `ensure_node_id` | 1 | keep-class-3 | The device's own node id. |
| `lib.rs` | `ensure_vault_metadata` | 1 | keep-seeder | The vault id, minted once at creation (genesis). |
| `merchant_identity.rs` | `create_identity` | 1 | keep-caller | Reached from a direct writer outside the bus (`lib.rs`), not from `apply`; `personal-cfo-5ymg8.5` routes it. |

## Derived mints (inside `apply` and its helpers)

| Site | Kind | Tag | Natural key (keyed only) |
| --- | --- | --- | --- |
| `apply/accounts.rs` opening-balance transaction | ordinal | `txn.opening-balance` | |
| `apply/transactions.rs` `apply_transfer` | ordinal | `txn.transfer` | |
| `apply/transactions.rs` `apply_convert_unexplained_to_transaction` | ordinal | `txn.convert-unexplained` | |
| `apply/transactions.rs` `apply_set_splits`, one per payload line | ordinal | `split-line` | |
| `apply/inbox.rs` snooze and dismiss journal rows | ordinal | `change-journal` | |
| `apply/transactions.rs` `apply_confirm_obligation_early` (under "not already confirmed") | keyed | `txn.confirm-obligation` | recurring event id, scheduled date |
| `lib.rs` `reverse_and_hide_transaction` (void and unconfirm; the original may already be voided) | keyed | `txn.reversal` | the original transaction id |
| `apply/scenario_apply.rs` one promoted event per active scenario event (the count is base state) | keyed | `scenario-event` | scenario id, source event id |
| `debt.rs` `apply_set_card_statement_balance` (upsert; the id is used only on insert) | keyed | `card-statement` | account id, cycle close |
| `apply/ingestion_cmds.rs` `apply_commit_staged` transaction (reads class-3 staging; see below) | keyed | `txn.commit-staged` | staged transaction id |
| `apply/ingestion_cmds.rs` dedupe decisions | keyed | `dedupe-decision` | staged transaction id, decision |
| `apply/balance_cmds.rs` connector balance observation (insert branch of the upsert) | content | `balance-obs.connector` | account id, day (independent of the command: later commands update the row in place) |
| `ensure_system_ledger_account` (`lib.rs`; from `apply` and the vault-creating transaction), only on a miss in the class-1 map | content | `system-ledger-account` | role, currency code (independent of the command; looked up first, so a legacy vault keeps its random ids; `personal-cfo-32fmp.3`) |
| `seed_default_categories` (`lib.rs`), in the vault-creating transaction only, never reached from `apply` | content | `category.seed` | group name, or group name and leaf name (never recomputed to find a row: users rename and move seeded categories) |
| `apply/ingestion_cmds.rs` provenance link | keyed | `provenance-link` | staged transaction id, relationship |
| `apply/household.rs` `SetHouseholdTimezone` audit row (class 3, written once per command: `INSERT OR IGNORE`) | keyed | `audit.household-timezone` | none (the command id is in every keyed id) |
| `apply/recurring.rs` `apply_dismiss_recurring_suggestion` | none | | the suppression row has a natural key; the affected entity recorded with the command is the command id |

## The nine base-dependent upsert and ignore statements in `apply/` (and the helpers they call)

Each is a statement whose outcome depends on rows that may already exist. None mints an id: every
one is keyed by the row's natural key, so a replay or a rebase onto another base converges on the
same row. Disposition: **idempotent on a natural key, fine**; the one that does carry a minted id is
listed as keyed above.

| Statement | Table | Natural key | Disposition |
| --- | --- | --- | --- |
| `apply/recurring.rs` `INSERT OR IGNORE` | `recurring_event_tags` | (recurring event, tag) | idempotent on the composite key |
| `apply/recurring.rs` `ON CONFLICT ... DO UPDATE` | `recurring_suggestion_suppressions` | (merchant key, currency) | idempotent on the key; no id |
| `apply/household.rs` `INSERT OR IGNORE` | `audit_events` (class 3) | the keyed audit id | idempotent on the keyed id: one row per command |
| `apply/ingestion_cmds.rs` `INSERT OR IGNORE` | `transaction_categorizations` | transaction (1:1) | idempotent on the key; no id minted |
| `apply/transactions.rs` `INSERT OR REPLACE` | `transaction_reviews` | transaction | idempotent on the key |
| `apply/transactions.rs` `INSERT OR IGNORE` | `transaction_tags` | (transaction, tag) | idempotent on the composite key |
| `apply/transactions.rs` `ON CONFLICT ... DO UPDATE` | `transaction_notes` | transaction | idempotent on the key |
| `apply/transactions.rs` `INSERT OR IGNORE` | `split_line_tags` | (split line, tag) | idempotent; the split line id is an ordinal id |
| `apply/categorization_cmds.rs` `INSERT OR REPLACE` | `transaction_categorizations` | transaction (1:1) | latest wins on the key; no id |

Helpers reached from apply with the same shape: `debt.rs` `INSERT OR REPLACE` (`debt_terms`, keyed by
account: idempotent), `debt.rs` `ON CONFLICT` (`credit_card_statements`: the id is keyed, above) and
`lib.rs` `INSERT OR REPLACE` (`transaction_details`, keyed by transaction: idempotent).

## The `ORDER BY id` sites that sort by `id` alone

An id sort carries a meaning only where the ids were minted in an order that matters. Ordinal ids
sort in creation order (the ordinal sits in `rand_a`, right after the millisecond timestamp); keyed
ids and singleton ids sort in an arbitrary but replica-independent order. The ten sites that order by
`id` with no other key, and the verdict on each:

| Site | What it orders | Verdict |
| --- | --- | --- |
| `lib.rs` split-line and provenance read (`ORDER BY spl.id LIMIT 1`) | the one `created_from` provenance link of a transaction | at most one such link exists per transaction (its commit); the order is moot |
| `lib.rs` `dependency_edges_for_row` | class-2 forecast edges | stable output only; rebuilt locally |
| `forecast_persist.rs` (three: recurring events, income sources, assumption inputs) | the inputs hashed into a forecast snapshot | stable hashing; ids come from payloads or are derived, so equal across replicas |
| `money_inbox.rs` `low_confidence_category_transaction_ids` | class-2 inbox candidates | stable output only |
| `forecast/events.rs` (two: recurring transfers) | projected events | stable output; ids are payload ids |
| `forecast/loan_link.rs` `read_linked_bills` | linked bills | stable output; ids are payload ids |
| `apply/recurring.rs` "other bill linked to this loan" `ORDER BY id LIMIT 1` | which existing bill to name in the "already linked" error | the choice of which bill to name; the same bill on every replica, and the refusal does not depend on it |

The split-line read orders by `(sort_order, id)`, so the line order never depends on id order at all;
`tests/derived_ids_in_apply.rs` also checks that `ORDER BY id` is creation order for ordinal ids.

## Known gaps

* **`CommitStaged` is not replicable as a command.** It reads the device-local (class 3) staging
  rows to decide what to commit, so its effect depends on state a replica does not have. Its ids
  are keyed (never ordinals, never a count), but the command itself must be reshaped before Sync
  ships by `personal-cfo-0g528` (SYNC-2c, import-family payload materialization), which blocks the
  Sync client engine and genesis; that bead also re-keys these ids on payload-carried identity
  (never a staged id, which does not cross the service boundary) and removes this row.
* **SQLite's own clock and random values** are the only other nondeterminism source in SQL:
  `tests/id_mint_inventory.rs` also fails on `'now'`, `randomblob`, `CURRENT_TIMESTAMP` or
  `datetime(` in any SQL the engine runs, except migration 44's two `strftime('now')` stamps. They
  touch only a pre-44 vault, which predates any genesis; a restored pre-44 backup re-bootstraps (its
  nonce state is lost) rather than rejoining.

## Measured

The largest per-command mint count is a `SetSplits`, one ordinal id per payload line; every other
command mints at most a handful of ordinal ids and a handful of keyed ids (a commit of one staged
transaction mints its transaction, up to one decision row and one provenance row; a scenario apply
mints one keyed id per active scenario event). The cap is 4,096 ordinals per command (`DerivedIds`),
and a command that would exceed it is refused before any row is kept
(`tests/derived_ids_in_apply.rs`).
