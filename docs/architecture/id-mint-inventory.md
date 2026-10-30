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
| `lib.rs` | `stage_sync_batch` | 2 | keep-class-3 | Device-local import staging. |
| `lib.rs` | `clone_scenario` | 1 | keep-pending | A direct writer outside the bus; `personal-cfo-5ymg8.5` routes it as a command with a derived id. |
| `lib.rs` | `ensure_node_id` | 1 | keep-class-3 | The device's own node id. |
| `lib.rs` | `ensure_vault_metadata` | 1 | keep-seeder | The vault id, minted once at creation (genesis). |
| `lib.rs` | `ensure_default_categories` | 2 | keep-seeder | Genesis seeder; `personal-cfo-32fmp.3` makes it genesis-only with content-derived ids. |
| `lib.rs` | `ensure_system_ledger_account` | 1 | keep-singleton | The create-if-missing system account, reached from `apply`; `personal-cfo-32fmp.3` derives it from its content and it consumes no ordinal. The one allowed random mint reachable from apply, until then. |
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
| `apply/ingestion_cmds.rs` provenance link | keyed | `provenance-link` | staged transaction id, relationship |
| `apply/household.rs` `SetHouseholdTimezone` audit row (class 3, written once per command: `INSERT OR IGNORE`) | keyed | `audit.household-timezone` | none (the command id is in every keyed id) |
| `apply/recurring.rs` `apply_dismiss_recurring_suggestion` | none | | the suppression row has a natural key; the affected entity recorded with the command is the command id |

## Known gaps

* **`CommitStaged` is not replicable as a command.** It reads the device-local (class 3) staging
  rows to decide what to commit, so its effect depends on state a replica does not have. Its ids
  are keyed (never ordinals, never a count), but the command itself must be reshaped by the import
  reshape before Sync ships; that bead blocks Sync shipping.
* **SQLite's own clock and random values** are the only other nondeterminism source in SQL:
  `tests/id_mint_inventory.rs` also fails on `'now'`, `randomblob`, `CURRENT_TIMESTAMP` or
  `datetime(` in any SQL the engine runs, except migration 44's two `strftime('now')` stamps, which
  ran before any genesis and so cannot fork a Sync vault.

## Measured

The largest known per-command mint count is a `SetSplits` with one id per payload line; the cap is
4,096 ordinals per command (`DerivedIds`), and a command that would exceed it is refused before any
row is written (`tests/derived_ids_in_apply.rs`). An import batch commit commits one staged
transaction per command, so it mints a handful of keyed ids.
