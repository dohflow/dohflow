//! Database migration framework (personal-cfo-wkn).
//!
//! Schema changes are expressed as an ordered list of [`Migration`]s, each with
//! a forward `up` and an optional backward `down`. A runner applies pending
//! migrations (each in its own transaction), recording a content hash per
//! applied migration in the `schema_migrations` tracker for drift detection
//! ("signing" via hash; cryptographic signatures are deferred to
//! `personal-cfo-vhv`).
//!
//! **Baseline-at-current:** migration `0001` is the consolidated current schema
//! ([`crate::BASELINE_UP`]); later migrations are real forward changes. We do
//! not reconstruct never-shipped historical versions.
//!
//! The runner completes before [`crate::DbWorker::open`] returns a usable
//! worker, so no domain query ever runs mid-migration. The explicit `Migrating`
//! state-machine transition is `personal-cfo-tg5`.

use chrono::Utc;
use rusqlite::{params, Connection};

use crate::{projection, DbError, BASELINE_UP};

/// A single forward (and optional backward) schema migration.
pub(crate) struct Migration {
    /// Monotonic version; `MIGRATIONS` is ordered by it.
    pub version: i64,
    /// Stable identifier (recorded in the tracker).
    pub name: &'static str,
    /// Forward SQL, run with `execute_batch`.
    pub up: &'static str,
    /// Backward SQL for one-step rollback. `None` = irreversible (the baseline).
    /// Consumed by [`migrate_down`] (tests + the future repair/state-machine
    /// path, `personal-cfo-tg5`); no non-test caller yet.
    #[allow(dead_code)]
    pub down: Option<&'static str>,
    /// Whether applying this migration requires rebuilding the read models.
    pub rebuilds_read_models: bool,
}

/// The ordered migration set. `0001` is the consolidated baseline; later
/// entries are real forward changes, each with a `down` when reversible.
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "baseline",
        up: BASELINE_UP,
        down: None,
        rebuilds_read_models: false,
    },
    Migration {
        version: 2,
        name: "operation_log_correlation_index",
        up: "CREATE INDEX IF NOT EXISTS idx_operation_log_correlation
                ON operation_log (correlation_id);",
        down: Some("DROP INDEX IF EXISTS idx_operation_log_correlation;"),
        rebuilds_read_models: false,
    },
    // Recurring net-pay income sources (personal-cfo-le79). The schedule is stored
    // as a frequency token + anchor calendar DATE; pay dates are generated at read
    // time by the pay-schedule engine. Net amount is integer minor units (no f64).
    Migration {
        version: 3,
        name: "income_sources",
        up: "CREATE TABLE IF NOT EXISTS income_sources (
                id                 BLOB    PRIMARY KEY,
                name               TEXT    NOT NULL,
                net_minor_units    INTEGER NOT NULL,
                currency           TEXT    NOT NULL,
                frequency          TEXT    NOT NULL,
                anchor_date        TEXT    NOT NULL,
                deposit_account_id BLOB,
                active             INTEGER NOT NULL DEFAULT 1,
                created_at         TEXT    NOT NULL
            );",
        down: Some("DROP TABLE IF EXISTS income_sources;"),
        rebuilds_read_models: false,
    },
    // Canonical recurring-obligation model (plan §9.8, personal-cfo-rxw): the
    // rule/schedule (`recurring_events`), its materialised occurrences
    // (`recurring_event_instances`), merchant/contract metadata
    // (`bill_contracts`), and the forecast-facing obligation projection
    // (`commitments`, rebuilt from the first three — never hand-written).
    // Single-household, so no `household_id`; amounts are integer minor units
    // (no f64); dates are ISO `YYYY-MM-DD`; cross-entity refs are BLOB UUIDs
    // without FK constraints (matching the baseline). Forward-looking columns
    // (price history, contract documents, renewal/cancellation) are deferred.
    Migration {
        version: 4,
        name: "recurring_events_and_commitments",
        up: "CREATE TABLE IF NOT EXISTS recurring_events (
                id                       BLOB PRIMARY KEY,
                name                     TEXT NOT NULL,
                source                   TEXT NOT NULL DEFAULT 'manual'
                    CHECK (source IN ('manual', 'auto_detected')),
                account_id               BLOB,
                amount_expected_minor    INTEGER NOT NULL,
                amount_variance_minor    INTEGER,
                currency                 TEXT NOT NULL,
                frequency                TEXT NOT NULL,
                custom_schedule_json     TEXT,
                next_expected_date       TEXT,
                category_id              BLOB,
                detection_confidence_bps INTEGER NOT NULL DEFAULT 0,
                include_in_forecast      INTEGER NOT NULL DEFAULT 1,
                autopay_account_id       BLOB,
                is_active                INTEGER NOT NULL DEFAULT 1,
                notes                    TEXT,
                created_at               TEXT NOT NULL,
                updated_at               TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS recurring_event_instances (
                id                    BLOB PRIMARY KEY,
                recurring_event_id    BLOB NOT NULL,
                scheduled_date        TEXT NOT NULL,
                expected_amount_minor INTEGER NOT NULL,
                currency              TEXT NOT NULL,
                status                TEXT NOT NULL DEFAULT 'scheduled'
                    CHECK (status IN ('scheduled', 'paid', 'skipped', 'overridden')),
                override_amount_minor INTEGER,
                linked_transaction_id BLOB,
                created_at            TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_recurring_event_instances_event
                ON recurring_event_instances (recurring_event_id);
            CREATE TABLE IF NOT EXISTS bill_contracts (
                id                        BLOB PRIMARY KEY,
                name                      TEXT NOT NULL,
                type                      TEXT NOT NULL
                    CHECK (type IN ('utility', 'rent_mortgage', 'insurance',
                        'subscription', 'loan_payment', 'tax', 'membership',
                        'childcare', 'other')),
                merchant_identity_id      TEXT,
                expected_amount_minor     INTEGER,
                amount_variance_minor     INTEGER,
                currency                  TEXT NOT NULL,
                cadence                   TEXT NOT NULL,
                due_rule_json             TEXT,
                autopay_enabled           INTEGER,
                autopay_account_id        BLOB,
                payment_method_account_id BLOB,
                recurring_event_id        BLOB,
                status                    TEXT NOT NULL DEFAULT 'active'
                    CHECK (status IN ('active', 'paused', 'cancelled',
                        'suspected', 'needs_review')),
                include_in_forecast       INTEGER NOT NULL DEFAULT 1,
                created_at                TEXT NOT NULL,
                updated_at                TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS commitments (
                id                        BLOB PRIMARY KEY,
                commitment_type           TEXT NOT NULL
                    CHECK (commitment_type IN ('rent', 'mortgage', 'utility',
                        'subscription', 'insurance', 'loan_payment',
                        'credit_card_payment', 'tax_reserve', 'childcare',
                        'tuition', 'membership', 'transfer', 'other')),
                name                      TEXT NOT NULL,
                merchant_identity_id      TEXT,
                amount_expected_minor     INTEGER,
                amount_confidence_bps     INTEGER NOT NULL DEFAULT 0,
                due_rule_json             TEXT,
                payment_source_account_id BLOB,
                autopay_status            TEXT NOT NULL DEFAULT 'unknown'
                    CHECK (autopay_status IN ('enabled', 'disabled', 'unknown')),
                source_entity_type        TEXT
                    CHECK (source_entity_type IN ('recurring_event', 'bill_contract')),
                source_entity_id          BLOB,
                include_in_forecast       INTEGER NOT NULL DEFAULT 1,
                status                    TEXT NOT NULL DEFAULT 'active'
                    CHECK (status IN ('active', 'paused', 'cancelled',
                        'suspected', 'needs_review')),
                stale_after_date          TEXT,
                created_at                TEXT NOT NULL,
                updated_at                TEXT NOT NULL
            );",
        down: Some(
            "DROP TABLE IF EXISTS commitments;
            DROP TABLE IF EXISTS bill_contracts;
            DROP INDEX IF EXISTS idx_recurring_event_instances_event;
            DROP TABLE IF EXISTS recurring_event_instances;
            DROP TABLE IF EXISTS recurring_events;",
        ),
        rebuilds_read_models: false,
    },
    // Encrypted attachment store (personal-cfo-bcj, ADR 0023). Metadata only —
    // the encrypted bytes live in `<vault>/blobs/`, each file named by its
    // `storage_id`. Every attachment has a per-blob content key wrapped under the
    // DEK (ADR 0002 §5); filenames + metadata are plaintext columns here because
    // the whole DB is SQLCipher-encrypted at rest. `attachment_links` is the
    // many-to-many join from domain entities (transactions, bills, …) to
    // attachments. BLOB UUIDs, no FK constraints (matching the baseline).
    Migration {
        version: 5,
        name: "attachments",
        up: "CREATE TABLE IF NOT EXISTS attachments (
                id                  BLOB    PRIMARY KEY,
                storage_id          TEXT    NOT NULL UNIQUE,
                wrapped_content_key BLOB    NOT NULL,
                content_key_nonce   BLOB    NOT NULL,
                content_nonce       BLOB    NOT NULL,
                content_alg         TEXT    NOT NULL,
                plaintext_size      INTEGER NOT NULL,
                mime_type           TEXT,
                original_filename   TEXT,
                ref_count           INTEGER NOT NULL DEFAULT 0,
                created_at          TEXT    NOT NULL
            );
            CREATE TABLE IF NOT EXISTS attachment_links (
                attachment_id BLOB NOT NULL,
                entity_kind   TEXT NOT NULL,
                entity_id     BLOB NOT NULL,
                created_at    TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_attachment_links_attachment
                ON attachment_links (attachment_id);
            CREATE INDEX IF NOT EXISTS idx_attachment_links_entity
                ON attachment_links (entity_kind, entity_id);",
        down: Some(
            "DROP INDEX IF EXISTS idx_attachment_links_entity;
            DROP INDEX IF EXISTS idx_attachment_links_attachment;
            DROP TABLE IF EXISTS attachment_links;
            DROP TABLE IF EXISTS attachments;",
        ),
        rebuilds_read_models: false,
    },
    // Security/audit events, distinct from the financial `operation_log` (which is
    // entity-scoped with provenance). Non-financial events — e.g. the onboarding
    // no-reset-warning acknowledgement (personal-cfo-n7bo) — are append-only
    // records carrying the command/correlation provenance + a timestamp. Immutable
    // like the operation log (ADR 0011): INSERT only, UPDATE/DELETE abort.
    Migration {
        version: 6,
        name: "audit_events",
        up: "CREATE TABLE IF NOT EXISTS audit_events (
                id             BLOB PRIMARY KEY,
                event_type     TEXT NOT NULL,
                command_id     BLOB NOT NULL,
                correlation_id BLOB NOT NULL,
                created_at     TEXT NOT NULL
            );
            CREATE TRIGGER IF NOT EXISTS audit_events_no_update
            BEFORE UPDATE ON audit_events
            BEGIN
                SELECT RAISE(ABORT, 'audit_events is append-only');
            END;
            CREATE TRIGGER IF NOT EXISTS audit_events_no_delete
            BEFORE DELETE ON audit_events
            BEGIN
                SELECT RAISE(ABORT, 'audit_events is append-only');
            END;",
        down: Some(
            "DROP TRIGGER IF EXISTS audit_events_no_delete;
            DROP TRIGGER IF EXISTS audit_events_no_update;
            DROP TABLE IF EXISTS audit_events;",
        ),
        rebuilds_read_models: false,
    },
    // A free-text description on bill contracts (personal-cfo-zl1l). Optional and
    // nullable; surfaced in the bills UI alongside name/amount/type. Added via
    // ALTER so existing vaults gain it without rebuilding the table; the reverse
    // drops the column (SQLite >= 3.35 supports DROP COLUMN; the engine is pinned
    // at 3.45 by personal-cfo-7igv).
    Migration {
        version: 7,
        name: "bill_contract_description",
        up: "ALTER TABLE bill_contracts ADD COLUMN description TEXT;",
        down: Some("ALTER TABLE bill_contracts DROP COLUMN description;"),
        rebuilds_read_models: false,
    },
    // Encrypted user settings (personal-cfo-p5g): a key/value store for app-level
    // configuration — reporting currency, locale, notification + privacy
    // preferences, etc. `value` holds a JSON or scalar string; `value_schema_version`
    // versions the per-setting value shape (distinct from the DB `user_version`).
    // Written directly (like vault_metadata), not via a ledger command.
    Migration {
        version: 8,
        name: "settings",
        up: "CREATE TABLE IF NOT EXISTS settings (
                key                  TEXT PRIMARY KEY NOT NULL,
                value                TEXT NOT NULL,
                value_schema_version INTEGER NOT NULL DEFAULT 1,
                updated_at           TEXT NOT NULL
            );",
        down: Some("DROP TABLE IF EXISTS settings;"),
        rebuilds_read_models: false,
    },
    // Soft-archive timestamp for recurring bills (personal-cfo-4d8.2). Nullable;
    // set when a bill is archived (alongside recurring_events.is_active = 0, which
    // already excludes it from the commitments projection / forecast), cleared on
    // restore. Added via ALTER so existing vaults gain it without a rebuild.
    Migration {
        version: 9,
        name: "recurring_event_archived_at",
        up: "ALTER TABLE recurring_events ADD COLUMN archived_at TEXT;",
        down: Some("ALTER TABLE recurring_events DROP COLUMN archived_at;"),
        rebuilds_read_models: false,
    },
    // Forecast assumption-event backbone (ADR 0026 §4, personal-cfo-5u2): the
    // versioned input/override/scenario events a reproducible run applies
    // (forecast_assumption_events — run-independent, gated by `status` so user
    // overrides survive a re-run), the per-run event→row attribution edges that
    // power per-row explanation + incremental invalidation
    // (forecast_dependency_edges), and the date-range recompute queue
    // (forecast_dirty_ranges). BLOB UUID PKs, JSON params, ISO `YYYY-MM-DD`
    // dates, cross-entity refs as BLOB UUIDs without FK constraints (matching
    // the baseline). The persistence tables (forecast_runs/_rows/_input_snapshots)
    // already exist in the baseline.
    Migration {
        version: 10,
        name: "forecast_assumption_events",
        up: "CREATE TABLE IF NOT EXISTS forecast_assumption_events (
                id                 BLOB PRIMARY KEY,
                kind               TEXT NOT NULL CHECK (kind IN (
                    'income_amount', 'income_date', 'bill_amount', 'bill_date',
                    'card_payment_behavior', 'minimum_cash_floor',
                    'variable_spend_override', 'one_time_event', 'inflation_rate',
                    'scenario_toggle', 'exclusion')),
                target_entity_type TEXT,
                target_entity_id   BLOB,
                params_json        TEXT NOT NULL,
                source             TEXT NOT NULL CHECK (source IN (
                    'user_override', 'model_default', 'agent_proposal', 'scheduled')),
                scenario_id        BLOB,
                status             TEXT NOT NULL DEFAULT 'active' CHECK (status IN (
                    'active', 'cleared', 'superseded')),
                superseded_by      BLOB,
                origin_run_id      BLOB,
                created_at         TEXT NOT NULL,
                updated_at         TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_assumption_events_active
                ON forecast_assumption_events (status, scenario_id);
            CREATE TABLE IF NOT EXISTS forecast_dependency_edges (
                id              BLOB PRIMARY KEY,
                forecast_run_id BLOB NOT NULL,
                from_event_id   BLOB NOT NULL,
                to_row_id       BLOB NOT NULL,
                edge_type       TEXT NOT NULL CHECK (edge_type IN (
                    'affects', 'derives_from')),
                created_at      TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_dependency_edges_row
                ON forecast_dependency_edges (to_row_id);
            CREATE INDEX IF NOT EXISTS idx_dependency_edges_run
                ON forecast_dependency_edges (forecast_run_id);
            CREATE TABLE IF NOT EXISTS forecast_dirty_ranges (
                id                  BLOB PRIMARY KEY,
                from_date           TEXT NOT NULL,
                to_date             TEXT NOT NULL,
                reason              TEXT NOT NULL CHECK (reason IN (
                    'assumption_changed', 'transaction_added', 'schedule_changed',
                    'manual')),
                triggering_event_id BLOB,
                created_at          TEXT NOT NULL,
                resolved_at         TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_dirty_ranges_open
                ON forecast_dirty_ranges (resolved_at);",
        down: Some(
            "DROP TABLE IF EXISTS forecast_dirty_ranges;
             DROP TABLE IF EXISTS forecast_dependency_edges;
             DROP TABLE IF EXISTS forecast_assumption_events;",
        ),
        rebuilds_read_models: false,
    },
    // Reproducible forecast persistence (ADR 0026 §3, personal-cfo-eqfw PR-1). The
    // baseline forecast_input_snapshots (63t) stored only entity hashes + a cutoff
    // timestamp; re-running a forecast from a snapshot needs the actual inputs, so
    // this adds the content-addressed shape: a content_hash plus the input blobs
    // (recurring events, income sources, manual assumption events, scenario overlay
    // ids) and an op_seq ledger cutoff. model_registry versions forecast models
    // (L1 today; L2-L4 later); forecast_diffs records run-to-run comparisons (moved
    // here from 5u2). All additive + reversible; existing vaults gain the columns
    // without a rebuild. forecast_runs/_rows already exist in the baseline.
    Migration {
        version: 11,
        name: "forecast_run_persistence",
        up: "ALTER TABLE forecast_input_snapshots ADD COLUMN content_hash TEXT;
            ALTER TABLE forecast_input_snapshots ADD COLUMN ledger_cutoff_op_seq INTEGER;
            ALTER TABLE forecast_input_snapshots ADD COLUMN recurring_events_snapshot_json TEXT;
            ALTER TABLE forecast_input_snapshots ADD COLUMN income_sources_snapshot_json TEXT;
            ALTER TABLE forecast_input_snapshots ADD COLUMN manual_assumption_events_json TEXT;
            ALTER TABLE forecast_input_snapshots ADD COLUMN scenario_overlay_ids_json TEXT;
            CREATE INDEX IF NOT EXISTS idx_input_snapshots_content_hash
                ON forecast_input_snapshots (content_hash);
            CREATE TABLE IF NOT EXISTS model_registry (
                model_id        TEXT PRIMARY KEY,
                version         TEXT NOT NULL,
                parameters_json TEXT,
                created_at      TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS forecast_diffs (
                id              BLOB PRIMARY KEY,
                forecast_run_id BLOB NOT NULL,
                prior_run_id    BLOB,
                summary_json    TEXT NOT NULL,
                created_at      TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_forecast_diffs_run
                ON forecast_diffs (forecast_run_id);",
        down: Some(
            "DROP TABLE IF EXISTS forecast_diffs;
             DROP TABLE IF EXISTS model_registry;
             DROP INDEX IF EXISTS idx_input_snapshots_content_hash;
             ALTER TABLE forecast_input_snapshots DROP COLUMN scenario_overlay_ids_json;
             ALTER TABLE forecast_input_snapshots DROP COLUMN manual_assumption_events_json;
             ALTER TABLE forecast_input_snapshots DROP COLUMN income_sources_snapshot_json;
             ALTER TABLE forecast_input_snapshots DROP COLUMN recurring_events_snapshot_json;
             ALTER TABLE forecast_input_snapshots DROP COLUMN ledger_cutoff_op_seq;
             ALTER TABLE forecast_input_snapshots DROP COLUMN content_hash;",
        ),
        rebuilds_read_models: false,
    },
    // Forecast-state schema (ADR 0026, personal-cfo-0mg — the F1 finale): scenario
    // definitions plus the actualization / quality / backtest / risk tables the
    // F2-F3 features build on. `scenarios` holds only the named definition +
    // lifecycle; a scenario's events are scenario-scoped assumption_events (5u2),
    // so there is NO scenario_events table (ADR 0026 §5). `model_registry` already
    // exists (eqfw, v11). Integer minor units + bps metrics (no f64); CHECK-
    // constrained tokens; BLOB UUID refs without FK constraints (baseline).
    Migration {
        version: 12,
        name: "forecast_state_schema",
        up: "CREATE TABLE IF NOT EXISTS scenarios (
                id          BLOB PRIMARY KEY,
                name        TEXT NOT NULL,
                description TEXT,
                status      TEXT NOT NULL DEFAULT 'draft'
                    CHECK (status IN ('draft', 'active', 'archived')),
                base_run_id BLOB,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS forecast_actuals (
                id                     BLOB PRIMARY KEY,
                forecast_run_id        BLOB NOT NULL,
                forecast_row_id        BLOB,
                realized_date          TEXT NOT NULL,
                realized_amount_minor  INTEGER NOT NULL,
                currency               TEXT NOT NULL,
                match_status           TEXT NOT NULL CHECK (match_status IN (
                    'exact', 'matched', 'missed', 'superseded')),
                matched_transaction_id BLOB,
                created_at             TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_forecast_actuals_run
                ON forecast_actuals (forecast_run_id);
            CREATE TABLE IF NOT EXISTS forecast_quality_scores (
                id              BLOB PRIMARY KEY,
                forecast_run_id BLOB NOT NULL,
                metric_type     TEXT NOT NULL CHECK (metric_type IN (
                    'mape', 'smape', 'mae', 'coverage', 'bias')),
                score_bps       INTEGER NOT NULL,
                sample_size     INTEGER NOT NULL,
                horizon_days    INTEGER,
                computed_at     TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_quality_scores_run
                ON forecast_quality_scores (forecast_run_id);
            CREATE TABLE IF NOT EXISTS forecast_backtest_results (
                id           BLOB PRIMARY KEY,
                model_id     TEXT NOT NULL,
                as_of_date   TEXT NOT NULL,
                horizon_days INTEGER NOT NULL,
                metric_type  TEXT NOT NULL CHECK (metric_type IN (
                    'mape', 'smape', 'mae', 'coverage', 'bias')),
                score_bps    INTEGER NOT NULL,
                sample_size  INTEGER NOT NULL,
                created_at   TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_backtest_model
                ON forecast_backtest_results (model_id);
            CREATE TABLE IF NOT EXISTS risk_flags (
                id                     BLOB PRIMARY KEY,
                forecast_run_id        BLOB,
                flag_type              TEXT NOT NULL CHECK (flag_type IN (
                    'low_balance', 'overdraft_risk', 'large_outflow', 'income_gap',
                    'volatility', 'data_quality')),
                severity               TEXT NOT NULL CHECK (severity IN (
                    'info', 'warning', 'critical')),
                projected_impact_minor INTEGER,
                evidence_json          TEXT,
                suggested_actions_json TEXT,
                readiness_required_bps INTEGER NOT NULL DEFAULT 0,
                created_at             TEXT NOT NULL,
                resolved_at            TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_risk_flags_run
                ON risk_flags (forecast_run_id);",
        down: Some(
            "DROP TABLE IF EXISTS risk_flags;
             DROP TABLE IF EXISTS forecast_backtest_results;
             DROP TABLE IF EXISTS forecast_quality_scores;
             DROP TABLE IF EXISTS forecast_actuals;
             DROP TABLE IF EXISTS scenarios;",
        ),
        rebuilds_read_models: false,
    },
    // Balance observations (ADR 0027, personal-cfo-xmc): evidence of an account's
    // balance at a point in time — NOT a ledger posting (an observation never
    // mutates the ledger). A `source = manual` row is a user BALANCE ASSERTION (the
    // additive-balance primitive); the assertion-anchored balance + the derived
    // auto-reconciling adjustment ("plug") live in the worker (personal-cfo-ueg6),
    // so this is schema only. Integer minor units (no f64); CHECK-constrained
    // source; BLOB UUID refs without FK (baseline convention). Indexed by
    // (account_id, observed_at) for latest-observation lookups.
    Migration {
        version: 13,
        name: "balance_observations",
        up: "CREATE TABLE IF NOT EXISTS balance_observations (
                id                        BLOB PRIMARY KEY,
                account_id                BLOB NOT NULL,
                observed_at               TEXT NOT NULL,
                balance_amount_minor      INTEGER NOT NULL,
                balance_currency          TEXT NOT NULL,
                source                    TEXT NOT NULL CHECK (source IN (
                    'manual', 'csv_import', 'ofx_import', 'connector_sync',
                    'reconciliation_session')),
                source_record_id          BLOB,
                reconciliation_session_id BLOB,
                created_at                TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_balance_observations_account
                ON balance_observations (account_id, observed_at);",
        down: Some("DROP TABLE IF EXISTS balance_observations;"),
        rebuilds_read_models: false,
    },
    // Account subtype (ADR 0028, personal-cfo-9dgg): an optional finer
    // classification within an account's cashflow_role (checking vs savings, credit
    // card vs line of credit, etc.). Nullable — NULL is "unspecified" and passes the
    // CHECK. The token set mirrors core-ledger's `AccountSubtype` (its single source
    // of truth); the role↔subtype match is validated in the kernel, because a column
    // CHECK cannot reference `cashflow_role` under SQLite's `ALTER ... ADD COLUMN`.
    // Only the liquid subtypes drive cash-tier rollups; the rest are foundation for
    // later grouping. Down drops the column (SQLite >= 3.35; pinned at 3.45.3).
    Migration {
        version: 14,
        name: "account_subtype",
        up: "ALTER TABLE accounts ADD COLUMN subtype TEXT
                CHECK (subtype IN (
                    'checking', 'savings', 'money_market', 'cash',
                    'credit_card', 'line_of_credit',
                    'mortgage', 'auto_loan', 'student_loan',
                    'brokerage', 'retirement'));",
        down: Some("ALTER TABLE accounts DROP COLUMN subtype;"),
        rebuilds_read_models: false,
    },
    // Income-source archival (personal-cfo-tch0): an `archived_at` timestamp to
    // mirror the recurring-bill archive-with-history model (4d8.2). `income_sources`
    // already carries `active` (the forecast filters `active = 1`); this records
    // *when* it was archived so the UI can show it. Nullable; down drops the column.
    Migration {
        version: 15,
        name: "income_source_archived_at",
        up: "ALTER TABLE income_sources ADD COLUMN archived_at TEXT;",
        down: Some("ALTER TABLE income_sources DROP COLUMN archived_at;"),
        rebuilds_read_models: false,
    },
    // R2 ingestion staging substrate (ADR 0008, personal-cfo-ihe). The eight tables
    // every importer/extractor/connector commits *through* — no source writes the
    // canonical ledger directly (ADR 0014 pipeline). `staged_*` rows are scratch
    // space: TRUNCATE-safe, discardable without touching accounts/ledger/balances.
    //
    // Two reconciliations vs the plan's §9.12 sketch, recorded in ADR 0008:
    //   * shred-after-parse (ADR 0014 §4): we keep `source_hash` (fingerprint) +
    //     `normalized_json` (extracted fields), NEVER the raw bytes — so there is no
    //     `raw_payload_ref`/`raw_payload_encrypted` column.
    //   * the `source_batches.status` lifecycle is recast around auto-commit-clean:
    //     parsing -> staged -> committed | partially_committed | discarded | failed,
    //     plus superseded. There is NO `reviewing` batch state — review is per-record
    //     (Money-Inbox items), so a clean import flows straight to `committed`.
    //
    // Single-household (no `household_id`); BLOB UUID PKs; integer minor units; ISO
    // TEXT dates; CHECK-constrained enums. Unlike the baseline's "BLOB refs without
    // FK" convention, the staging substrate DECLARES its intra-staging foreign keys
    // (the audit chain must be FK-strict per z2a) — they document intended integrity
    // and would enforce if `PRAGMA foreign_keys` is ever enabled. Cross-boundary refs
    // to canonical rows (proposed/matched account, committed txn, provenance entity)
    // stay plain BLOBs. Runtime strictness of the provenance invariant is enforced in
    // the `ingestion` module (`link_provenance` validates the source_record exists),
    // since `configure_conn` leaves `foreign_keys` at SQLite's per-connection default.
    //
    // NB: import provenance lives in `source_provenance_links` (entity -> source_record).
    // The baseline's `provenance_links` is a DIFFERENT, pre-existing table (op_seq ->
    // entity, the op-log per-entity index, ADR 0011); the plan's §9.12 `provenance_links`
    // name is taken, so we disambiguate rather than clobber it.
    Migration {
        version: 16,
        name: "ingestion_staging",
        up: "CREATE TABLE IF NOT EXISTS source_batches (
                id              BLOB    PRIMARY KEY,
                source_type     TEXT    NOT NULL CHECK (source_type IN (
                    'manual', 'csv', 'ofx', 'qfx', 'qif', 'pdf', 'image',
                    'simplefin', 'teller', 'plaid', 'relay', 'other')),
                source_name     TEXT,
                file_fingerprint TEXT,
                parser_version  TEXT,
                status          TEXT    NOT NULL DEFAULT 'parsing' CHECK (status IN (
                    'parsing', 'staged', 'committed', 'partially_committed',
                    'discarded', 'failed', 'superseded')),
                staged_count    INTEGER NOT NULL DEFAULT 0,
                committed_count INTEGER NOT NULL DEFAULT 0,
                skipped_count   INTEGER NOT NULL DEFAULT 0,
                summary_json    TEXT,
                imported_at     TEXT,
                created_at      TEXT    NOT NULL,
                updated_at      TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_source_batches_fingerprint
                ON source_batches (file_fingerprint);
            CREATE TABLE IF NOT EXISTS source_records (
                id                   BLOB    PRIMARY KEY,
                source_batch_id      BLOB    NOT NULL REFERENCES source_batches(id),
                external_id          TEXT,
                source_hash          TEXT    NOT NULL,
                normalized_json      TEXT    NOT NULL,
                parse_confidence_bps INTEGER,
                created_at           TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_source_records_batch
                ON source_records (source_batch_id);
            CREATE TABLE IF NOT EXISTS parser_runs (
                id              BLOB    PRIMARY KEY,
                source_batch_id BLOB    NOT NULL REFERENCES source_batches(id),
                parser_name     TEXT    NOT NULL,
                parser_version  TEXT    NOT NULL,
                bytes_in        INTEGER NOT NULL DEFAULT 0,
                records_out     INTEGER NOT NULL DEFAULT 0,
                status          TEXT    NOT NULL CHECK (status IN (
                    'ok', 'limit_exceeded', 'parse_error', 'timeout')),
                limit_hit       TEXT,
                started_at      TEXT    NOT NULL,
                finished_at     TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_parser_runs_batch
                ON parser_runs (source_batch_id);
            CREATE TABLE IF NOT EXISTS staged_transactions (
                id                       BLOB    PRIMARY KEY,
                source_record_id         BLOB    NOT NULL REFERENCES source_records(id),
                proposed_account_id      BLOB,
                posted_at                TEXT    NOT NULL,
                amount_minor             INTEGER NOT NULL,
                currency                 TEXT    NOT NULL,
                normalized_merchant      TEXT,
                description              TEXT,
                txn_fingerprint          TEXT    NOT NULL,
                dedupe_status            TEXT    NOT NULL DEFAULT 'pending' CHECK (
                    dedupe_status IN ('pending', 'unique', 'suspected_duplicate')),
                commit_status            TEXT    NOT NULL DEFAULT 'staged' CHECK (
                    commit_status IN ('staged', 'committed', 'skipped', 'flagged')),
                committed_transaction_id BLOB,
                created_at               TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_staged_transactions_record
                ON staged_transactions (source_record_id);
            CREATE INDEX IF NOT EXISTS idx_staged_transactions_fingerprint
                ON staged_transactions (txn_fingerprint);
            CREATE TABLE IF NOT EXISTS staged_accounts (
                id                   BLOB    PRIMARY KEY,
                source_batch_id      BLOB    NOT NULL REFERENCES source_batches(id),
                external_name        TEXT,
                external_number_hash TEXT,
                proposed_subtype     TEXT,
                matched_account_id   BLOB,
                created_at           TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_staged_accounts_batch
                ON staged_accounts (source_batch_id);
            CREATE TABLE IF NOT EXISTS staged_balances (
                id               BLOB    PRIMARY KEY,
                source_record_id BLOB    NOT NULL REFERENCES source_records(id),
                account_ref      BLOB,
                observed_at      TEXT    NOT NULL,
                balance_minor    INTEGER NOT NULL,
                currency         TEXT    NOT NULL,
                created_at       TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_staged_balances_record
                ON staged_balances (source_record_id);
            CREATE TABLE IF NOT EXISTS dedupe_decisions (
                id                    BLOB    PRIMARY KEY,
                source_batch_id       BLOB    NOT NULL REFERENCES source_batches(id),
                layer                 TEXT    NOT NULL CHECK (layer IN (
                    'file', 'transaction')),
                staged_transaction_id BLOB    REFERENCES staged_transactions(id),
                matched_entity_type   TEXT,
                matched_entity_id     BLOB,
                decision              TEXT    NOT NULL CHECK (decision IN (
                    'committed', 'skipped', 'merged', 'flagged')),
                reason                TEXT    NOT NULL,
                decided_at            TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_dedupe_decisions_batch
                ON dedupe_decisions (source_batch_id);
            CREATE INDEX IF NOT EXISTS idx_dedupe_decisions_staged
                ON dedupe_decisions (staged_transaction_id);
            CREATE TABLE IF NOT EXISTS source_provenance_links (
                id               BLOB    PRIMARY KEY,
                entity_type      TEXT    NOT NULL CHECK (entity_type IN (
                    'ledger_transaction', 'ledger_posting', 'account',
                    'balance_observation')),
                entity_id        BLOB    NOT NULL,
                source_record_id BLOB    NOT NULL REFERENCES source_records(id),
                relationship     TEXT    NOT NULL CHECK (relationship IN (
                    'created_from', 'amended_by', 'inferred_from', 'confirmed_by',
                    'contradicted_by', 'superseded_by')),
                created_at       TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_source_provenance_record
                ON source_provenance_links (source_record_id);
            CREATE INDEX IF NOT EXISTS idx_source_provenance_entity
                ON source_provenance_links (entity_type, entity_id);",
        down: Some(
            "DROP TABLE IF EXISTS source_provenance_links;
             DROP TABLE IF EXISTS dedupe_decisions;
             DROP TABLE IF EXISTS staged_balances;
             DROP TABLE IF EXISTS staged_accounts;
             DROP TABLE IF EXISTS staged_transactions;
             DROP TABLE IF EXISTS parser_runs;
             DROP TABLE IF EXISTS source_records;
             DROP TABLE IF EXISTS source_batches;",
        ),
        rebuilds_read_models: false,
    },
    // The Money Inbox triage read model (ADR 0014 §7, bead dsq). One materialized
    // store for items across all kinds; a deterministic `money_inbox::rebuild_in`
    // projects it from canonical state (cursor + checksum like other read models).
    // `item_kind` is CHECK'd against all nine planned kinds so future generators
    // need no migration; only `imported_waiting_commit` is wired in this slice.
    Migration {
        version: 17,
        name: "money_inbox_read_model",
        up: "CREATE TABLE IF NOT EXISTS money_inbox_read_model (
                item_id       BLOB    PRIMARY KEY,
                item_kind     TEXT    NOT NULL CHECK (item_kind IN (
                    'imported_waiting_commit', 'low_confidence_category',
                    'possible_transfer', 'possible_recurring_bill', 'stale_balance',
                    'document_extraction_unconfirmed', 'forecast_assumption_attention',
                    'connector_error', 'reconciliation_discrepancy')),
                target_table  TEXT    NOT NULL,
                target_id     BLOB    NOT NULL,
                priority      INTEGER NOT NULL DEFAULT 50,
                surfaced_at   TEXT    NOT NULL,
                snoozed_until TEXT,
                dismissed_at  TEXT,
                resolved_at   TEXT,
                payload_json  TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_money_inbox_default_sort
                ON money_inbox_read_model (priority, surfaced_at, snoozed_until);",
        down: Some("DROP TABLE IF EXISTS money_inbox_read_model;"),
        rebuilds_read_models: false,
    },
    // Per-transaction display detail (personal-cfo-byxe): a free-text memo +
    // counterparty kept beside the pure double-entry ledger (ADR 0007 stays clean).
    // Written when a transaction is committed (imports carry the source merchant /
    // description); read by the transactions list so imported rows aren't bare.
    Migration {
        version: 18,
        name: "transaction_details",
        up: "CREATE TABLE IF NOT EXISTS transaction_details (
                transaction_id BLOB    PRIMARY KEY REFERENCES ledger_transactions(id),
                memo           TEXT,
                counterparty   TEXT,
                created_at     TEXT    NOT NULL
            );",
        down: Some("DROP TABLE IF EXISTS transaction_details;"),
        rebuilds_read_models: false,
    },
    // Soft-delete for categories (ADR 0030, personal-cfo-bac): "delete" is archive
    // (non-destructive). An archived category hides from pickers but keeps its
    // historical assignments valid. System defaults can be archived (hidden) too.
    Migration {
        version: 19,
        name: "category_archived_at",
        up: "ALTER TABLE categories ADD COLUMN archived_at TEXT;",
        down: Some("ALTER TABLE categories DROP COLUMN archived_at;"),
        rebuilds_read_models: false,
    },
    // A transaction's category assignment (ADR 0030, personal-cfo-bac): one row per
    // categorized transaction (1:1), latest-wins. `source`/`confidence_bps` let a
    // later rule/model path write the same store without a schema change; a manual
    // assignment is source = user, confidence = 100% (10000 bps). Read by the
    // transactions list (LEFT JOIN); the category is metadata, not a ledger posting
    // (ADR 0007 untouched).
    Migration {
        version: 20,
        name: "transaction_categorizations",
        up: "CREATE TABLE IF NOT EXISTS transaction_categorizations (
                transaction_id BLOB    PRIMARY KEY REFERENCES ledger_transactions(id),
                category_id    BLOB    NOT NULL REFERENCES categories(id),
                source         TEXT    NOT NULL
                    CHECK (source IN ('user', 'rule', 'model', 'import_alias')),
                confidence_bps INTEGER NOT NULL,
                assigned_at    TEXT    NOT NULL
            );",
        down: Some("DROP TABLE IF EXISTS transaction_categorizations;"),
        rebuilds_read_models: false,
    },
    // Scheduled account-to-account transfers (ADR 0026 §14, personal-cfo-npoe): a
    // recurring definition that projects on the pay-schedule machinery. v1 moves
    // cash between two liquid accounts; each occurrence projects two legs in the
    // per-account forecast (source −, destination +), aggregate net cash unchanged.
    Migration {
        version: 21,
        name: "recurring_transfers",
        up: "CREATE TABLE IF NOT EXISTS recurring_transfers (
                id                BLOB    PRIMARY KEY,
                source_account_id BLOB    NOT NULL REFERENCES accounts(id),
                dest_account_id   BLOB    NOT NULL REFERENCES accounts(id),
                amount_minor      INTEGER NOT NULL,
                currency          TEXT    NOT NULL,
                frequency         TEXT    NOT NULL,
                anchor_date       TEXT    NOT NULL,
                created_at        TEXT    NOT NULL,
                CHECK (source_account_id <> dest_account_id)
            );",
        down: Some("DROP TABLE IF EXISTS recurring_transfers;"),
        rebuilds_read_models: false,
    },
    // Money Inbox action audit log (ADR 0014 §7, personal-cfo-3d3 / -ci71): one row
    // per user action on an inbox item (snooze / dismiss / resolve / import-anyway /
    // skip), with the kind-specific detail in payload_json. The soft actions
    // (snooze/dismiss) are re-applied to the rebuilt read model by `item_id`, so the
    // user's triage state survives a projection rebuild. No FK to the rebuilt
    // `money_inbox_read_model` — its rows churn on every rebuild, so an enforced FK
    // would break it; `item_id` is a logical link.
    Migration {
        version: 22,
        name: "change_journal_entries",
        up: "CREATE TABLE IF NOT EXISTS change_journal_entries (
                id           BLOB    PRIMARY KEY,
                item_id      BLOB    NOT NULL,
                entry_kind   TEXT    NOT NULL
                    CHECK (entry_kind IN (
                        'snooze', 'dismiss', 'resolve', 'import_anyway', 'skip')),
                actor        TEXT    NOT NULL,
                payload_json TEXT    NOT NULL,
                created_at   TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_change_journal_item
                ON change_journal_entries (item_id, created_at);",
        down: Some("DROP TABLE IF EXISTS change_journal_entries;"),
        rebuilds_read_models: false,
    },
    // Soft-delete marker for transactions (personal-cfo-4d8.11, ADR 0007 §9): a voided
    // transaction (and its reversal) stay in the append-only ledger but are hidden from
    // every list/projection. Added empty, so no read-model rebuild is needed here.
    Migration {
        version: 23,
        name: "ledger_transaction_voided_at",
        up: "ALTER TABLE ledger_transactions ADD COLUMN voided_at TEXT;",
        down: Some("ALTER TABLE ledger_transactions DROP COLUMN voided_at;"),
        rebuilds_read_models: false,
    },
    // Reviewed-state override store (personal-cfo-4d8.7, ADR 0032 §2). Canonical state
    // (like transaction_categorizations), not a read model: a row is the user's explicit
    // mark-reviewed/unreviewed for a transaction. The DEFAULT (no row) is derived at read
    // time from import provenance — imported = unreviewed, manual = reviewed — so existing
    // imports are flagged without a backfill.
    Migration {
        version: 24,
        name: "transaction_reviews",
        up: "CREATE TABLE IF NOT EXISTS transaction_reviews (
                transaction_id BLOB    PRIMARY KEY,
                reviewed       INTEGER NOT NULL CHECK (reviewed IN (0, 1)),
                reviewed_at    TEXT    NOT NULL
            );",
        down: Some("DROP TABLE IF EXISTS transaction_reviews;"),
        rebuilds_read_models: false,
    },
    // Tags + notes (personal-cfo-2ryf / -hmt, ADR 0033). Tags = many-to-many labels
    // orthogonal to the 1:1 category; canonical state, soft-delete via archived_at
    // (names unique among non-archived). transaction_details gains a free-text note
    // beside the byxe memo/counterparty.
    Migration {
        version: 25,
        name: "tags_and_notes",
        up: "CREATE TABLE IF NOT EXISTS tags (
                id          BLOB PRIMARY KEY,
                name        TEXT NOT NULL,
                color       TEXT,
                archived_at TEXT,
                created_at  TEXT NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_tags_name_active
                ON tags (name) WHERE archived_at IS NULL;
            CREATE TABLE IF NOT EXISTS transaction_tags (
                transaction_id BLOB NOT NULL,
                tag_id         BLOB NOT NULL,
                PRIMARY KEY (transaction_id, tag_id)
            );
            CREATE INDEX IF NOT EXISTS idx_transaction_tags_tag
                ON transaction_tags (tag_id);
            ALTER TABLE transaction_details ADD COLUMN note TEXT;",
        down: Some(
            "ALTER TABLE transaction_details DROP COLUMN note;
             DROP TABLE IF EXISTS transaction_tags;
             DROP TABLE IF EXISTS tags;",
        ),
        rebuilds_read_models: false,
    },
    // Transaction splits (personal-cfo-kr9, ADR 0034). A side-table decomposition of
    // ONE unchanged ledger transaction: split_lines carry per-line amount + category +
    // note, split_line_tags carry per-line tags (ADR 0033 §7). Not multi-posting — the
    // ledger postings are untouched. The lines' amounts sum to the transaction amount
    // (command-enforced by SetSplits, e7i).
    Migration {
        version: 26,
        name: "transaction_splits",
        up: "CREATE TABLE IF NOT EXISTS split_lines (
                id             BLOB PRIMARY KEY,
                transaction_id BLOB    NOT NULL,
                amount_minor   INTEGER NOT NULL,
                currency       TEXT    NOT NULL,
                category_id    BLOB,
                note           TEXT,
                sort_order     INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_split_lines_txn
                ON split_lines (transaction_id);
            CREATE TABLE IF NOT EXISTS split_line_tags (
                split_line_id BLOB NOT NULL,
                tag_id        BLOB NOT NULL,
                PRIMARY KEY (split_line_id, tag_id)
            );
            CREATE INDEX IF NOT EXISTS idx_split_line_tags_tag
                ON split_line_tags (tag_id);",
        down: Some(
            "DROP TABLE IF EXISTS split_line_tags;
             DROP TABLE IF EXISTS split_lines;",
        ),
        rebuilds_read_models: false,
    },
    // Canonical merchant-identity entity layer (ADR 0030 addendum, personal-cfo-zrpg).
    // merchant_identities is the real-world merchant; merchant_aliases map each
    // normalize_merchant key (7yh0) onto one identity, so "AMZN MKTP US" and "AMAZON COM"
    // resolve together. is_ambiguous flags merchants that span categories (Amazon, Costco)
    // so auto-categorization knows to defer to disambiguation rather than apply one category.
    // Schema only — no read-model wiring / seeding / fuzzy matching yet (downstream beads).
    Migration {
        version: 27,
        name: "merchant_identity_layer",
        up: "CREATE TABLE IF NOT EXISTS merchant_identities (
                id                  BLOB    PRIMARY KEY,
                display_name        TEXT    NOT NULL,
                default_category_id BLOB    REFERENCES categories(id),
                is_ambiguous        INTEGER NOT NULL DEFAULT 0,
                source              TEXT    NOT NULL
                    CHECK (source IN ('seed', 'user', 'auto')),
                created_at          TEXT    NOT NULL
            );
            CREATE TABLE IF NOT EXISTS merchant_aliases (
                normalized_key       TEXT    PRIMARY KEY,
                merchant_identity_id BLOB    NOT NULL REFERENCES merchant_identities(id),
                source               TEXT    NOT NULL
                    CHECK (source IN ('seed', 'user', 'auto')),
                confidence_bps       INTEGER NOT NULL,
                created_at           TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_merchant_aliases_identity
                ON merchant_aliases (merchant_identity_id);",
        down: Some(
            "DROP TABLE IF EXISTS merchant_aliases;
             DROP TABLE IF EXISTS merchant_identities;",
        ),
        rebuilds_read_models: false,
    },
    // Prefix aliases for multi-location merchants (ADR 0030 addendum, personal-cfo-2a6r).
    // An 'exact' alias maps one normalized key; a 'prefix' alias matches any key that begins
    // with it on a word boundary — so a physical chain (COSTCO, TARGET, WALMART) whose
    // normalized key keeps the city ("COSTCO WHSE SEATTLE") resolves to one identity without
    // enumerating every location. Token set enforced in code (the seed only writes the two).
    Migration {
        version: 28,
        name: "merchant_alias_match_type",
        up: "ALTER TABLE merchant_aliases ADD COLUMN match_type TEXT NOT NULL DEFAULT 'exact';",
        down: Some("ALTER TABLE merchant_aliases DROP COLUMN match_type;"),
        rebuilds_read_models: false,
    },
    // Per-account debt terms (ADR 0035 §5, personal-cfo-6wk.6). A side-table keyed by
    // account_id — the shared attributes both cards (xcq) and loans (40f) extend, kept off
    // the accounts table so liquid accounts don't carry NULL debt columns. repayment_philosophy
    // is the ADR 0035 §1 enum (one model, not a credit-card-only pay_behavior);
    // paying_source_account_id is the single authoritative debt→liquid mapping (§2), validated
    // in the kernel/worker (target liability, source liquid) since a column CHECK cannot read
    // another row's role. Day-of-month 1–31 with a month-end clamp applied at projection time.
    Migration {
        version: 29,
        name: "debt_terms",
        up: "CREATE TABLE IF NOT EXISTS debt_terms (
                account_id               BLOB    PRIMARY KEY REFERENCES accounts(id),
                apr_bps                  INTEGER,
                statement_close_day      INTEGER,
                payment_due_day          INTEGER,
                grace_period_days        INTEGER,
                credit_limit_minor       INTEGER,
                repayment_philosophy     TEXT    NOT NULL DEFAULT 'unknown'
                    CHECK (repayment_philosophy IN (
                        'pay_in_full', 'pay_statement_balance', 'pay_current_balance',
                        'pay_minimum', 'pay_fixed_amount', 'unknown')),
                fixed_amount_minor       INTEGER,
                min_payment_percent_bps  INTEGER,
                min_payment_floor_minor  INTEGER,
                paying_source_account_id BLOB    REFERENCES accounts(id),
                updated_at               TEXT    NOT NULL
            );",
        down: Some("DROP TABLE IF EXISTS debt_terms;"),
        rebuilds_read_models: false,
    },
    // Credit-card cycle + statement schema (ADR 0039, personal-cfo-xcq). The per-account debt
    // ATTRIBUTES (apr, days, philosophy, paying source) live in `debt_terms` (v29); these two
    // tables hold the per-cycle working + posted state the cycle model (kqez), statement forecast
    // (4lhm), and interest (llx5) populate. Money is integer minor units; rates (bps) stay on
    // `debt_terms`. Schema only — no derivation logic here. `cycle_close` / `payment_due` are ISO
    // dates; `status` tracks a cycle from open → statemented → paid/partial.
    Migration {
        version: 30,
        name: "credit_card_cycles",
        up: "CREATE TABLE IF NOT EXISTS credit_card_cycles (
                account_id                       BLOB    NOT NULL REFERENCES accounts(id),
                cycle_close                      TEXT    NOT NULL,
                payment_due                      TEXT,
                currency                         TEXT    NOT NULL,
                carried_opening_balance_minor    INTEGER NOT NULL DEFAULT 0,
                new_charges_minor                INTEGER NOT NULL DEFAULT 0,
                accrued_interest_minor           INTEGER NOT NULL DEFAULT 0,
                forecast_statement_balance_minor INTEGER,
                statement_balance_minor          INTEGER,
                minimum_due_minor                INTEGER,
                status                           TEXT    NOT NULL DEFAULT 'open'
                    CHECK (status IN ('open', 'statemented', 'paid', 'partial')),
                created_at                       TEXT    NOT NULL,
                updated_at                       TEXT    NOT NULL,
                PRIMARY KEY (account_id, cycle_close)
            );
            CREATE TABLE IF NOT EXISTS credit_card_statements (
                id                      BLOB    PRIMARY KEY,
                account_id              BLOB    NOT NULL REFERENCES accounts(id),
                cycle_close             TEXT    NOT NULL,
                statement_balance_minor INTEGER NOT NULL,
                minimum_due_minor       INTEGER,
                payment_due             TEXT    NOT NULL,
                currency                TEXT    NOT NULL,
                created_at              TEXT    NOT NULL,
                UNIQUE (account_id, cycle_close)
            );
            CREATE INDEX IF NOT EXISTS idx_credit_card_cycles_account
                ON credit_card_cycles (account_id);",
        down: Some(
            "DROP INDEX IF EXISTS idx_credit_card_cycles_account;
             DROP TABLE IF EXISTS credit_card_statements;
             DROP TABLE IF EXISTS credit_card_cycles;",
        ),
        rebuilds_read_models: false,
    },
    // Widen the forecast_assumption_events `kind` CHECK to admit 'recurring_debt_payment' — the
    // ADR 0036 "extra $X/mo against debt" overlay (personal-cfo-6wk.19). SQLite can't ALTER a
    // CHECK, and v10 is a shipped, content-hashed migration that must not be edited, so rebuild
    // the table (create-copy-drop-rename) and recreate its index. Columns/order are unchanged, so
    // `INSERT .. SELECT *` round-trips. `down` restores the original CHECK, dropping any rows of
    // the new kind (they'd violate it).
    Migration {
        version: 31,
        name: "forecast_assumption_events_recurring_debt_kind",
        up: "CREATE TABLE forecast_assumption_events_new (
                id                 BLOB PRIMARY KEY,
                kind               TEXT NOT NULL CHECK (kind IN (
                    'income_amount', 'income_date', 'bill_amount', 'bill_date',
                    'card_payment_behavior', 'minimum_cash_floor',
                    'variable_spend_override', 'one_time_event', 'inflation_rate',
                    'scenario_toggle', 'exclusion', 'recurring_debt_payment')),
                target_entity_type TEXT,
                target_entity_id   BLOB,
                params_json        TEXT NOT NULL,
                source             TEXT NOT NULL CHECK (source IN (
                    'user_override', 'model_default', 'agent_proposal', 'scheduled')),
                scenario_id        BLOB,
                status             TEXT NOT NULL DEFAULT 'active' CHECK (status IN (
                    'active', 'cleared', 'superseded')),
                superseded_by      BLOB,
                origin_run_id      BLOB,
                created_at         TEXT NOT NULL,
                updated_at         TEXT NOT NULL
            );
            INSERT INTO forecast_assumption_events_new SELECT * FROM forecast_assumption_events;
            DROP TABLE forecast_assumption_events;
            ALTER TABLE forecast_assumption_events_new RENAME TO forecast_assumption_events;
            CREATE INDEX IF NOT EXISTS idx_assumption_events_active
                ON forecast_assumption_events (status, scenario_id);",
        down: Some(
            "CREATE TABLE forecast_assumption_events_old (
                id                 BLOB PRIMARY KEY,
                kind               TEXT NOT NULL CHECK (kind IN (
                    'income_amount', 'income_date', 'bill_amount', 'bill_date',
                    'card_payment_behavior', 'minimum_cash_floor',
                    'variable_spend_override', 'one_time_event', 'inflation_rate',
                    'scenario_toggle', 'exclusion')),
                target_entity_type TEXT,
                target_entity_id   BLOB,
                params_json        TEXT NOT NULL,
                source             TEXT NOT NULL CHECK (source IN (
                    'user_override', 'model_default', 'agent_proposal', 'scheduled')),
                scenario_id        BLOB,
                status             TEXT NOT NULL DEFAULT 'active' CHECK (status IN (
                    'active', 'cleared', 'superseded')),
                superseded_by      BLOB,
                origin_run_id      BLOB,
                created_at         TEXT NOT NULL,
                updated_at         TEXT NOT NULL
            );
            INSERT INTO forecast_assumption_events_old
                SELECT * FROM forecast_assumption_events WHERE kind != 'recurring_debt_payment';
            DROP TABLE forecast_assumption_events;
            ALTER TABLE forecast_assumption_events_old RENAME TO forecast_assumption_events;
            CREATE INDEX IF NOT EXISTS idx_assumption_events_active
                ON forecast_assumption_events (status, scenario_id);",
        ),
        rebuilds_read_models: false,
    },
    // Early-confirmed obligations (ADR 5ie.7, personal-cfo-5ie.9). One durable row per
    // recurring-bill occurrence the user has marked paid ahead of (or on) its scheduled date:
    // it links the occurrence to the real ledger transaction that satisfied it and is the
    // DETERMINISTIC source the forecast reads to suppress that occurrence -- exact on
    // (recurring_event_id, scheduled_date), so a confirm made further ahead than the ±7-day
    // instance-linking tolerance still suppresses correctly (no double-count). UNIQUE enforces
    // idempotency; unconfirm deletes the row (and voids the transaction).
    Migration {
        version: 32,
        name: "confirmed_obligations",
        up: "CREATE TABLE IF NOT EXISTS confirmed_obligations (
                id                  BLOB    PRIMARY KEY,
                recurring_event_id  BLOB    NOT NULL REFERENCES recurring_events(id),
                scheduled_date      TEXT    NOT NULL,
                transaction_id      BLOB    NOT NULL,
                paying_account_id   BLOB    NOT NULL REFERENCES accounts(id),
                actual_date         TEXT    NOT NULL,
                actual_amount_minor INTEGER NOT NULL,
                currency            TEXT    NOT NULL,
                created_at          TEXT    NOT NULL,
                UNIQUE (recurring_event_id, scheduled_date)
            );",
        down: Some("DROP TABLE IF EXISTS confirmed_obligations;"),
        rebuilds_read_models: false,
    },
    // Authoritative autopay intent on the bill the forecast reads (ADR 0041, personal-cfo-mc7f).
    // Nullable: 1 = autopay, 0 = manual, NULL = legacy/unknown. Distinct from autopay_account_id
    // ("which account"). Rebuild so the commitments projection re-derives autopay_status from this
    // (intent) rather than from account presence.
    Migration {
        version: 33,
        name: "recurring_events_autopay_enabled",
        up: "ALTER TABLE recurring_events ADD COLUMN autopay_enabled INTEGER;",
        down: Some("ALTER TABLE recurring_events DROP COLUMN autopay_enabled;"),
        rebuilds_read_models: true,
    },
    // Prepare the risk_flags model for the descriptive band-drift signal (personal-cfo-5ie.8): add
    // the `cash_band_breach` flag_type, and reconcile the ADR 0018 §915.1 decision by renaming
    // `suggested_actions_json` → `contributing_factors_json` (descriptive evidence only, no
    // directives). SQLite can't ALTER a CHECK and the column is renamed, so rebuild the table
    // (create-copy-drop-rename), mapping the renamed column explicitly. The table is latent (no
    // production emitter yet), so no rows move in practice. `down` restores the old schema, dropping
    // any cash_band_breach rows (they'd violate the old CHECK).
    Migration {
        version: 34,
        name: "risk_flags_cash_band_breach_and_915_1_rename",
        up: "CREATE TABLE risk_flags_new (
                id                        BLOB PRIMARY KEY,
                forecast_run_id           BLOB,
                flag_type                 TEXT NOT NULL CHECK (flag_type IN (
                    'low_balance', 'overdraft_risk', 'large_outflow', 'income_gap',
                    'volatility', 'data_quality', 'cash_band_breach')),
                severity                  TEXT NOT NULL CHECK (severity IN (
                    'info', 'warning', 'critical')),
                projected_impact_minor    INTEGER,
                evidence_json             TEXT,
                contributing_factors_json TEXT,
                readiness_required_bps    INTEGER NOT NULL DEFAULT 0,
                created_at                TEXT NOT NULL,
                resolved_at               TEXT
            );
            INSERT INTO risk_flags_new
                (id, forecast_run_id, flag_type, severity, projected_impact_minor,
                 evidence_json, contributing_factors_json, readiness_required_bps,
                 created_at, resolved_at)
                SELECT id, forecast_run_id, flag_type, severity, projected_impact_minor,
                       evidence_json, suggested_actions_json, readiness_required_bps,
                       created_at, resolved_at
                FROM risk_flags;
            DROP TABLE risk_flags;
            ALTER TABLE risk_flags_new RENAME TO risk_flags;
            CREATE INDEX IF NOT EXISTS idx_risk_flags_run ON risk_flags (forecast_run_id);",
        down: Some(
            "CREATE TABLE risk_flags_old (
                id                     BLOB PRIMARY KEY,
                forecast_run_id        BLOB,
                flag_type              TEXT NOT NULL CHECK (flag_type IN (
                    'low_balance', 'overdraft_risk', 'large_outflow', 'income_gap',
                    'volatility', 'data_quality')),
                severity               TEXT NOT NULL CHECK (severity IN (
                    'info', 'warning', 'critical')),
                projected_impact_minor INTEGER,
                evidence_json          TEXT,
                suggested_actions_json TEXT,
                readiness_required_bps INTEGER NOT NULL DEFAULT 0,
                created_at             TEXT NOT NULL,
                resolved_at            TEXT
            );
            INSERT INTO risk_flags_old
                (id, forecast_run_id, flag_type, severity, projected_impact_minor,
                 evidence_json, suggested_actions_json, readiness_required_bps,
                 created_at, resolved_at)
                SELECT id, forecast_run_id, flag_type, severity, projected_impact_minor,
                       evidence_json, contributing_factors_json, readiness_required_bps,
                       created_at, resolved_at
                FROM risk_flags WHERE flag_type != 'cash_band_breach';
            DROP TABLE risk_flags;
            ALTER TABLE risk_flags_old RENAME TO risk_flags;
            CREATE INDEX IF NOT EXISTS idx_risk_flags_run ON risk_flags (forecast_run_id);",
        ),
        rebuilds_read_models: false,
    },
    // Read-path index for the assertion-anchored balance reads (personal-cfo-3fdd.3b).
    // The latest-assertion lookup (`assertion_anchored_balance`, the batched cash-tier
    // rollups, forecast anchors) orders by observed_at DESC, created_at DESC per
    // account; the v13 (account_id, observed_at) index cannot serve the created_at
    // tiebreak, so ship the full covering order. The paged transaction read needs no
    // new index: it rides the baseline idx_ledger_transactions_date /
    // idx_ledger_postings_transaction (verified via EXPLAIN QUERY PLAN in
    // `txn_row_reads_have_no_correlated_subqueries`).
    Migration {
        version: 35,
        name: "balance_observations_account_date_index",
        up: "CREATE INDEX IF NOT EXISTS idx_balance_observations_account_date
                ON balance_observations (account_id, observed_at DESC, created_at DESC);",
        down: Some("DROP INDEX IF EXISTS idx_balance_observations_account_date;"),
        rebuilds_read_models: false,
    },
    // Account-model extensions (ADR 0044, personal-cfo-4d8.22): real-asset subtypes,
    // account notes, and the loan's original principal.
    //
    // Broadening the `accounts.subtype` CHECK to admit the real-asset tokens needs a
    // COLUMN SWAP — SQLite can't ALTER a column CHECK. We add a new column carrying the
    // extended CHECK, copy across (every existing token is in the new superset, so all
    // rows pass), drop the old column, and rename. FK enforcement is off (per
    // configure_conn), and `subtype` is not indexed (v14's `down` already DROP COLUMNs
    // it), so the swap touches nothing else.
    Migration {
        version: 36,
        name: "account_model_extensions",
        up: "ALTER TABLE accounts ADD COLUMN subtype_v36 TEXT
                CHECK (subtype_v36 IN (
                    'checking', 'savings', 'money_market', 'cash',
                    'credit_card', 'line_of_credit',
                    'mortgage', 'auto_loan', 'student_loan',
                    'brokerage', 'retirement',
                    'property', 'vehicle', 'other_real_asset'));
             UPDATE accounts SET subtype_v36 = subtype;
             ALTER TABLE accounts DROP COLUMN subtype;
             ALTER TABLE accounts RENAME COLUMN subtype_v36 TO subtype;
             ALTER TABLE accounts ADD COLUMN notes TEXT;
             ALTER TABLE debt_terms ADD COLUMN original_principal_minor INTEGER;",
        // Reverse: drop the additions, then swap `subtype` back to the narrower CHECK.
        // (Down runs only on rollback/tests; fixture vaults carry no real-asset subtype,
        // so the narrower CHECK's UPDATE never rejects a row.)
        down: Some(
            "ALTER TABLE debt_terms DROP COLUMN original_principal_minor;
             ALTER TABLE accounts DROP COLUMN notes;
             ALTER TABLE accounts ADD COLUMN subtype_v14 TEXT
                 CHECK (subtype_v14 IN (
                     'checking', 'savings', 'money_market', 'cash',
                     'credit_card', 'line_of_credit',
                     'mortgage', 'auto_loan', 'student_loan',
                     'brokerage', 'retirement'));
             UPDATE accounts SET subtype_v14 = subtype;
             ALTER TABLE accounts DROP COLUMN subtype;
             ALTER TABLE accounts RENAME COLUMN subtype_v14 TO subtype;",
        ),
        rebuilds_read_models: false,
    },
    // The display-only real-asset -> financing-liability link (ADR 0044 §5,
    // personal-cfo-4d8.22.3): a nullable one-to-one pointer stored on the asset row
    // (a property -> its mortgage, a vehicle -> its auto-loan). FK enforcement is off
    // (SQLite default here), so `REFERENCES` is documentation, not a runtime constraint;
    // the apply layer validates the asset/liability roles. Additive + nullable, so no
    // read-model rebuild.
    Migration {
        version: 37,
        name: "account_linking",
        up: "ALTER TABLE accounts ADD COLUMN linked_account_id BLOB REFERENCES accounts(id);",
        down: Some("ALTER TABLE accounts DROP COLUMN linked_account_id;"),
        rebuilds_read_models: false,
    },
    // The secondary transaction / authorization date (ADR 0045, personal-cfo-4d8.24.1):
    // the posted date stays the primary date (`posted_at` / `occurred_at`); this nullable
    // column keeps the transaction date a source carries alongside it (e.g. a CapitalOne
    // "Transaction Date" vs "Posted Date") on both the staging and committed sides.
    // Additive + nullable; existing rows read NULL; no read-model rebuild.
    Migration {
        version: 38,
        name: "transaction_authorization_date",
        up: "ALTER TABLE staged_transactions ADD COLUMN transaction_date TEXT;
             ALTER TABLE transaction_details ADD COLUMN transaction_date TEXT;",
        down: Some(
            "ALTER TABLE staged_transactions DROP COLUMN transaction_date;
             ALTER TABLE transaction_details DROP COLUMN transaction_date;",
        ),
        rebuilds_read_models: false,
    },
    // The source's own category string carried on a staged transaction (ADR 0045 §3,
    // personal-cfo-4d8.24.1.1). At commit it is matched (by name) to a real category
    // and recorded as an `import_alias` categorization at reduced confidence — a
    // prefill the user confirms or corrects. Additive + nullable; existing rows NULL.
    Migration {
        version: 39,
        name: "staged_imported_category",
        up: "ALTER TABLE staged_transactions ADD COLUMN imported_category TEXT;",
        down: Some("ALTER TABLE staged_transactions DROP COLUMN imported_category;"),
        rebuilds_read_models: false,
    },
    // The normalized merchant key a recurring bill was promoted from (personal-cfo-5n4.8):
    // a durable link so the recurring-suggestion exclusion keys on this, not the bill's
    // (mutable) name — renaming a promoted bill no longer re-surfaces the suggestion.
    // Additive + nullable; existing/manually-created bills read NULL.
    Migration {
        version: 40,
        name: "recurring_event_source_merchant_key",
        up: "ALTER TABLE recurring_events ADD COLUMN source_merchant_key TEXT;",
        down: Some("ALTER TABLE recurring_events DROP COLUMN source_merchant_key;"),
        rebuilds_read_models: false,
    },
    // User dismissals of recurring-bill suggestions (ADR 0046, personal-cfo-4d8.24.6):
    // a suppression keyed on (merchant_key, currency) recording the dismissed amount +
    // cadence so detection can drop the suggestion until the pattern MATERIALLY changes
    // (amount outside the detector's band, or a different cadence). Latest-dismiss-wins
    // (upsert on the PK). Additive; no read-model rebuild.
    Migration {
        version: 41,
        name: "recurring_suggestion_suppressions",
        up: "CREATE TABLE IF NOT EXISTS recurring_suggestion_suppressions (
                merchant_key TEXT    NOT NULL,
                currency     TEXT    NOT NULL,
                amount_minor INTEGER NOT NULL,
                frequency    TEXT    NOT NULL,
                reason       TEXT,
                dismissed_at TEXT    NOT NULL,
                PRIMARY KEY (merchant_key, currency)
            );",
        down: Some("DROP TABLE IF EXISTS recurring_suggestion_suppressions;"),
        rebuilds_read_models: false,
    },
    // Tags on recurring bills (ADR 0033 addendum, personal-cfo-4d8.24.5.1): the shared
    // `tags` vocabulary joined to a recurring event, mirroring `transaction_tags`. Set
    // when promoting a suggestion so the bill's occurrences carry the tags. Additive.
    Migration {
        version: 42,
        name: "recurring_event_tags",
        up: "CREATE TABLE IF NOT EXISTS recurring_event_tags (
                recurring_event_id BLOB NOT NULL,
                tag_id             BLOB NOT NULL,
                PRIMARY KEY (recurring_event_id, tag_id)
            );
            CREATE INDEX IF NOT EXISTS idx_recurring_event_tags_tag
                ON recurring_event_tags (tag_id);",
        down: Some("DROP TABLE IF EXISTS recurring_event_tags;"),
        rebuilds_read_models: false,
    },
    // HSA + crypto investment subtypes (ADR 0028 addendum 2026-07-11, personal-cfo-4d8.25.23):
    // widen the `accounts.subtype` CHECK via the same COLUMN SWAP as v36 (SQLite can't ALTER a
    // column CHECK). Every existing token is in the new superset, so all rows pass.
    Migration {
        version: 43,
        name: "investment_subtypes_hsa_crypto",
        up: "ALTER TABLE accounts ADD COLUMN subtype_v43 TEXT
                CHECK (subtype_v43 IN (
                    'checking', 'savings', 'money_market', 'cash',
                    'credit_card', 'line_of_credit',
                    'mortgage', 'auto_loan', 'student_loan',
                    'brokerage', 'retirement', 'hsa', 'crypto',
                    'property', 'vehicle', 'other_real_asset'));
             UPDATE accounts SET subtype_v43 = subtype;
             ALTER TABLE accounts DROP COLUMN subtype;
             ALTER TABLE accounts RENAME COLUMN subtype_v43 TO subtype;",
        // Reverse to the v36 superset. Fixture/rollback vaults carry no hsa/crypto subtype,
        // so the narrower CHECK's UPDATE never rejects a row.
        down: Some(
            "ALTER TABLE accounts ADD COLUMN subtype_v36 TEXT
                 CHECK (subtype_v36 IN (
                     'checking', 'savings', 'money_market', 'cash',
                     'credit_card', 'line_of_credit',
                     'mortgage', 'auto_loan', 'student_loan',
                     'brokerage', 'retirement',
                     'property', 'vehicle', 'other_real_asset'));
             UPDATE accounts SET subtype_v36 = subtype;
             ALTER TABLE accounts DROP COLUMN subtype;
             ALTER TABLE accounts RENAME COLUMN subtype_v36 TO subtype;",
        ),
        rebuilds_read_models: false,
    },
    // Re-type the seeded "Credit Card Payment" category from expense to transfer and move it
    // under the "Transfers" group (ADR 0030 addendum 2026-07-11, personal-cfo-4d8.25.21): a
    // card payment is money movement (checking -> card), not new spend. Category IDs are
    // stable, so existing categorizations are preserved; only the type/parent/behavior change.
    // Guarded so it only fires on a vault that still has the seeded expense-typed leaf under
    // Debt AND has the Transfers group (never turns it into an orphan). A fresh vault seeds it
    // correctly from DEFAULT_TAXONOMY, so this UPDATE is a no-op there (empty categories table).
    // Rebuilds read models: the type + forecast_behavior change affects spend rollups and the
    // forecast's spend classification.
    Migration {
        version: 44,
        name: "credit_card_payment_is_a_transfer",
        up: "UPDATE categories
                SET parent_id = (SELECT id FROM categories
                                  WHERE is_system = 1 AND parent_id IS NULL AND name = 'Transfers'),
                    type = 'transfer',
                    forecast_behavior = 'ignore_cashflow',
                    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
              WHERE is_system = 1 AND name = 'Credit Card Payment' AND type = 'expense'
                AND EXISTS (SELECT 1 FROM categories
                             WHERE is_system = 1 AND parent_id IS NULL AND name = 'Transfers');",
        down: Some(
            "UPDATE categories
                SET parent_id = (SELECT id FROM categories
                                  WHERE is_system = 1 AND parent_id IS NULL AND name = 'Debt'),
                    type = 'expense',
                    forecast_behavior = 'deterministic',
                    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
              WHERE is_system = 1 AND name = 'Credit Card Payment' AND type = 'transfer'
                AND EXISTS (SELECT 1 FROM categories
                             WHERE is_system = 1 AND parent_id IS NULL AND name = 'Debt');",
        ),
        rebuilds_read_models: true,
    },
    // A scenario can carry a user-set expiry (ADR 0051 §3). It is filtered at READ
    // time, never flipped by a job, so rebuilds stay clock-independent (ADR 0014 §7).
    Migration {
        version: 45,
        name: "scenario_expiry",
        up: "ALTER TABLE scenarios ADD COLUMN expires_on TEXT;",
        down: Some("ALTER TABLE scenarios DROP COLUMN expires_on;"),
        rebuilds_read_models: false,
    },
    // The category filter and the spend aggregate both look split lines up BY CATEGORY
    // (ADR 0052), which the transaction-keyed index cannot serve.
    Migration {
        version: 46,
        name: "split_lines_category_index",
        up: "CREATE INDEX IF NOT EXISTS idx_split_lines_category
                ON split_lines (category_id);",
        down: Some("DROP INDEX IF EXISTS idx_split_lines_category;"),
        rebuilds_read_models: false,
    },
    // Applying a scenario (ADR 0055): `applied_at` + `applied_op_id` mark a scenario as
    // applied and carry the reversal handle, and `promoted_from_scenario_id` is both the
    // provenance of a promoted base event and what revert selects on.
    //
    // Applied-ness is deliberately NOT a fourth `status` token: status is a lifecycle
    // (draft/active/archived) and applied-ness is orthogonal to it — an applied scenario
    // can still be archived, and archiving must not un-apply anything (ADR 0055 §4).
    //
    // Three nullable adds, no backfill and no data rewrite. This lands on the owner's real
    // encrypted vault on next open, so additive-nullable is the only shape worth taking.
    Migration {
        version: 47,
        name: "scenario_apply",
        up: "ALTER TABLE scenarios ADD COLUMN applied_at TEXT;
             ALTER TABLE scenarios ADD COLUMN applied_op_id BLOB;
             ALTER TABLE forecast_assumption_events
                 ADD COLUMN promoted_from_scenario_id BLOB;
             CREATE INDEX IF NOT EXISTS idx_assumption_events_promoted_from
                 ON forecast_assumption_events (promoted_from_scenario_id);",
        down: Some(
            "DROP INDEX IF EXISTS idx_assumption_events_promoted_from;
             ALTER TABLE forecast_assumption_events
                 DROP COLUMN promoted_from_scenario_id;
             ALTER TABLE scenarios DROP COLUMN applied_op_id;
             ALTER TABLE scenarios DROP COLUMN applied_at;",
        ),
        rebuilds_read_models: false,
    },
    // Connector connections (personal-cfo-gglk, ADR 0060 §1): one row per
    // NOTE: the REFERENCES clauses are documentation — PRAGMA foreign_keys
    // stays OFF repo-wide (see ingestion.rs), so referential integrity is
    // app-enforced (delete_connector_connection removes links explicitly).
    // linked aggregator connection. `credential` is the provider access
    // credential (for SimpleFIN, the access URL) — encrypted at rest by
    // SQLCipher like every other column, stored in the vault deliberately so
    // backup/restore round-trips the connection (ADR 0060 §1; ADR 0004 §2).
    // Configuration, not ledger data: written directly like `settings`, no
    // op-log. `connector_account_links` maps the provider's external accounts
    // onto real accounts and carries the per-account sync watermark
    // (`last_synced_on`, a calendar date) that retry-required provider errors
    // hold back.
    Migration {
        version: 48,
        name: "connector_connections",
        up: "CREATE TABLE IF NOT EXISTS connector_connections (
                 id BLOB PRIMARY KEY,
                 adapter_id TEXT NOT NULL,
                 credential TEXT NOT NULL,
                 display_hint TEXT,
                 last_synced_at TEXT,
                 last_error TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS connector_account_links (
                 connection_id BLOB NOT NULL REFERENCES connector_connections(id),
                 external_id TEXT NOT NULL,
                 external_name TEXT,
                 account_id BLOB REFERENCES accounts(id),
                 last_synced_on TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 PRIMARY KEY (connection_id, external_id)
             );",
        down: Some(
            "DROP TABLE IF EXISTS connector_account_links;
             DROP TABLE IF EXISTS connector_connections;",
        ),
        rebuilds_read_models: false,
    },
    // The cross-source dedupe layer (personal-cfo-tevp, ADR 0014 §3 addendum)
    // probes staged_transactions by account + amount per committed row; the
    // staged table doubles as the permanent fingerprint index, so without
    // this the probe is a full scan that grows with vault history.
    Migration {
        version: 49,
        name: "staged_account_amount_index",
        up: "CREATE INDEX IF NOT EXISTS idx_staged_transactions_account_amount
                 ON staged_transactions (proposed_account_id, amount_minor);",
        down: Some("DROP INDEX IF EXISTS idx_staged_transactions_account_amount;"),
        rebuilds_read_models: false,
    },
    // Deterministic matcher projection for one-off manual future entries
    // (personal-cfo-xtz5, ADR 0026 addendum 2026-09-02): rebuilt alongside
    // the recurring-instance seam; contents are derived, never authored.
    Migration {
        version: 50,
        name: "manual_entry_links",
        up: "CREATE TABLE IF NOT EXISTS manual_entry_links (
                 assumption_event_id BLOB PRIMARY KEY,
                 linked_transaction_id BLOB NOT NULL,
                 matched_on TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );",
        down: Some("DROP TABLE IF EXISTS manual_entry_links;"),
        rebuilds_read_models: false,
    },
    // Local durable job state (personal-cfo-ati).  The table is deliberately
    // class 3: schedules, retry state, payload configuration, and unlock-window
    // claims are device-local and never enter the Sync snapshot/tail.
    Migration {
        version: 51,
        name: "durable_jobs",
        up: "CREATE TABLE IF NOT EXISTS durable_jobs (
                 id                         BLOB PRIMARY KEY,
                 kind                       TEXT NOT NULL,
                 cadence                    TEXT NOT NULL,
                 next_due_at                TEXT,
                 state                      TEXT NOT NULL DEFAULT 'queued'
                     CHECK (state IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
                 attempt_count              INTEGER NOT NULL DEFAULT 0
                     CHECK (attempt_count >= 0),
                 max_attempts               INTEGER NOT NULL
                     CHECK (max_attempts > 0),
                 backoff_initial_seconds   INTEGER NOT NULL DEFAULT 60
                     CHECK (backoff_initial_seconds >= 0),
                 backoff_multiplier_bps     INTEGER NOT NULL DEFAULT 20000
                     CHECK (backoff_multiplier_bps >= 10000),
                 enabled                    INTEGER NOT NULL DEFAULT 1
                     CHECK (enabled IN (0, 1)),
                 requires_explicit_opt_in  INTEGER NOT NULL DEFAULT 0
                     CHECK (requires_explicit_opt_in IN (0, 1)),
                 payload_json               TEXT,
                 last_run_at                TEXT,
                 last_outcome               TEXT
                     CHECK (last_outcome IS NULL OR last_outcome IN (
                         'succeeded', 'failed', 'cancelled', 'skipped')),
                 last_error                 TEXT,
                 last_unlock_window         TEXT,
                 cancel_requested           INTEGER NOT NULL DEFAULT 0
                     CHECK (cancel_requested IN (0, 1)),
                 created_at                 TEXT NOT NULL,
                 updated_at                 TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_durable_jobs_due
                 ON durable_jobs (enabled, state, next_due_at);",
        down: Some(
            "DROP INDEX IF EXISTS idx_durable_jobs_due;
             DROP TABLE IF EXISTS durable_jobs;",
        ),
        rebuilds_read_models: false,
    },
    // Verified local backup receipts and retention evidence (personal-cfo-8qh).
    // The destination is device-local and remains inside the SQLCipher vault;
    // retention separately verifies the package manifest before deleting it.
    Migration {
        version: 52,
        name: "backup_history",
        up: "CREATE TABLE IF NOT EXISTS backup_history (
                 backup_id       BLOB PRIMARY KEY CHECK (length(backup_id) = 16),
                 vault_id        BLOB NOT NULL CHECK (length(vault_id) = 16),
                 created_at      TEXT NOT NULL,
                 kind            TEXT NOT NULL CHECK (kind IN ('manual', 'scheduled')),
                 destination     TEXT NOT NULL CHECK (length(destination) > 0),
                 format_version  INTEGER NOT NULL CHECK (format_version = 2),
                 size_bytes      INTEGER NOT NULL CHECK (size_bytes >= 0),
                 verified        INTEGER NOT NULL CHECK (verified IN (0, 1)),
                 error           TEXT,
                 CHECK (
                     (verified = 1 AND error IS NULL)
                     OR (verified = 0 AND error IS NOT NULL)
                 )
             );
             CREATE INDEX IF NOT EXISTS idx_backup_history_vault_created
                 ON backup_history (vault_id, created_at DESC, backup_id DESC);",
        down: Some(
            "DROP INDEX IF EXISTS idx_backup_history_vault_created;
             DROP TABLE IF EXISTS backup_history;",
        ),
        rebuilds_read_models: false,
    },
];

const fn max_version() -> i64 {
    let mut max = 0;
    let mut i = 0;
    while i < MIGRATIONS.len() {
        let v = MIGRATIONS[i].version;
        if v > max {
            max = v;
        }
        i += 1;
    }
    max
}

/// The schema version a fully-migrated vault is at (the highest migration).
pub(crate) const CURRENT_VERSION: i64 = max_version();

/// Apply all pending migrations to `conn` using the canonical [`MIGRATIONS`] set.
/// Returns the number applied. Runs at vault open, before the worker is usable.
pub(crate) fn run_migrations(conn: &mut Connection) -> Result<u64, DbError> {
    apply(conn, MIGRATIONS)
}

/// Apply pending migrations from `migrations` (split out so tests can drive a
/// synthetic set). Each migration commits in its own transaction; a tracker row
/// records its content hash. If any applied migration rebuilds read models, the
/// transaction-display projection is rebuilt afterward.
pub(crate) fn apply(conn: &mut Connection, migrations: &[Migration]) -> Result<u64, DbError> {
    ensure_tracker(conn)?;
    let applied = applied_versions(conn)?;

    // Drift check: a migration that was already applied must not have had its
    // `up` SQL edited since (that would silently diverge vaults).
    for m in migrations {
        if let Some((_, recorded)) = applied.iter().find(|(v, _)| *v == m.version) {
            if recorded != &content_hash(m.up) {
                return Err(DbError::SelfTestFailed(format!(
                    "migration {} ({}) content hash changed after it was applied",
                    m.version, m.name
                )));
            }
        }
    }

    let mut count = 0u64;
    let mut needs_rebuild = false;
    for m in migrations {
        if applied.iter().any(|(v, _)| *v == m.version) {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(m.up)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, content_hash, applied_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                m.version,
                m.name,
                content_hash(m.up),
                Utc::now().to_rfc3339()
            ],
        )?;
        tx.commit()?;
        count += 1;
        needs_rebuild |= m.rebuilds_read_models;
    }

    conn.pragma_update(None, "user_version", CURRENT_VERSION)?;
    if needs_rebuild {
        projection::rebuild(conn)?;
    }
    Ok(count)
}

/// Roll back applied migrations down to (and including) `to_version`'s state —
/// i.e. revert everything with `version > to_version`, newest first. Errors if a
/// migration in that range is irreversible (`down == None`). Exercised by the
/// round-trip tests; the production repair path is `personal-cfo-tg5`.
#[allow(dead_code)]
pub(crate) fn migrate_down(conn: &mut Connection, to_version: i64) -> Result<u64, DbError> {
    let applied = applied_versions(conn)?;
    let mut to_revert: Vec<&Migration> = MIGRATIONS
        .iter()
        .filter(|m| m.version > to_version && applied.iter().any(|(v, _)| *v == m.version))
        .collect();
    to_revert.sort_by_key(|m| std::cmp::Reverse(m.version));

    let mut count = 0u64;
    for m in to_revert {
        let down = m.down.ok_or_else(|| {
            DbError::SelfTestFailed(format!(
                "migration {} ({}) is irreversible (no down)",
                m.version, m.name
            ))
        })?;
        let tx = conn.transaction()?;
        tx.execute_batch(down)?;
        tx.execute(
            "DELETE FROM schema_migrations WHERE version = ?1",
            [m.version],
        )?;
        tx.commit()?;
        count += 1;
    }
    conn.pragma_update(None, "user_version", to_version.max(0))?;
    Ok(count)
}

fn ensure_tracker(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version      INTEGER PRIMARY KEY,
            name         TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            applied_at   TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn applied_versions(conn: &Connection) -> Result<Vec<(i64, String)>, DbError> {
    let mut stmt = conn.prepare("SELECT version, content_hash FROM schema_migrations")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// FNV-1a (64-bit) hex digest over a migration's SQL — tamper/drift evidence.
/// Cryptographic signing of migration files is deferred to `personal-cfo-vhv`.
pub(crate) fn content_hash(sql: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in sql.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Migration upgrade/downgrade safety (personal-cfo-c545, §20.1).
///
/// Every prior schema version must migrate forward to current without losing
/// data or integrity, and every reversible migration must roll back the same
/// way. Each "fixture vault" is generated on demand by applying a prefix of
/// [`MIGRATIONS`] (per `tests/fixtures/README.md`, no binaries are committed),
/// then seeded with core baseline data and driven up and back down.
///
/// Migrations are SQL over the schema and are key-agnostic, so these run on a
/// plain in-memory connection — SQLCipher encryption is transparent to them and
/// is covered separately by the cross-version (personal-cfo-7igv) and side-file
/// leak (personal-cfo-zxvl) suites.
#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{apply, migrate_down, CURRENT_VERSION, MIGRATIONS};
    use crate::projection;

    /// Seed `n` rows into `accounts` — baseline core data present at every schema
    /// version — via raw SQL, so up/down migrations can be checked for loss.
    fn seed_accounts(conn: &Connection, n: u8) {
        for i in 0..n {
            let mut id = [0u8; 16];
            id[15] = i + 1;
            let mut ledger = [0u8; 16];
            ledger[0] = 0xAA;
            ledger[15] = i + 1;
            conn.execute(
                "INSERT INTO accounts
                    (id, ledger_account_id, name, cashflow_role, normal_balance, currency)
                 VALUES (?1, ?2, ?3, 'liquid_cash', 'debit', 'USD')",
                rusqlite::params![&id[..], &ledger[..], format!("Account {i}")],
            )
            .unwrap();
        }
    }

    fn account_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))
            .unwrap()
    }

    fn integrity_ok(conn: &Connection) -> bool {
        let report: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        report == "ok"
    }

    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    /// The set is contiguous `1..=CURRENT_VERSION` and every non-baseline
    /// migration ships a reversible `down`. This is what lets the up/down loops
    /// below cover *every* migration: adding one with a version gap or no `down`
    /// fails CI (the c545 "every migration ships a fixture" requirement).
    #[test]
    fn migration_set_is_contiguous_and_reversible() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                m.version,
                i as i64 + 1,
                "migration {} has a version gap / is out of order",
                m.name
            );
            if m.version > 1 {
                assert!(
                    m.down.is_some(),
                    "migration {} ({}) ships no down — c545 requires reversibility",
                    m.version,
                    m.name
                );
            }
        }
        assert_eq!(
            CURRENT_VERSION,
            MIGRATIONS.len() as i64,
            "CURRENT_VERSION must equal the migration count"
        );
    }

    /// Migration v44 moves an EXISTING seeded "Credit Card Payment" (expense, under
    /// Debt) to the Transfers group as a transfer, preserving its id
    /// (personal-cfo-4d8.25.21). The round-trip loops above only exercise v44 against
    /// an empty categories table; this proves the UPDATE on real data.
    #[test]
    fn v44_moves_an_existing_credit_card_payment_to_transfers() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..43]).unwrap(); // schema through v43, pre-move
        let debt = [0x0Au8; 16];
        let transfers = [0x0Bu8; 16];
        let ccp = [0x0Cu8; 16];
        let ins = |conn: &Connection,
                   id: &[u8],
                   parent: Option<&[u8]>,
                   name: &str,
                   ty: &str,
                   fb: &str| {
            conn.execute(
                "INSERT INTO categories
                    (id, household_id, parent_id, name, type, icon, color, is_system,
                     budget_default_minor, forecast_behavior, created_at, updated_at)
                 VALUES (?1, NULL, ?2, ?3, ?4, NULL, NULL, 1, NULL, ?5, '2026-01-01T00:00:00Z',
                         '2026-01-01T00:00:00Z')",
                rusqlite::params![id, parent, name, ty, fb],
            )
            .unwrap();
        };
        ins(&conn, &debt, None, "Debt", "expense", "deterministic");
        ins(
            &conn,
            &transfers,
            None,
            "Transfers",
            "transfer",
            "ignore_cashflow",
        );
        ins(
            &conn,
            &ccp,
            Some(&debt[..]),
            "Credit Card Payment",
            "expense",
            "deterministic",
        );

        apply(&mut conn, &MIGRATIONS[43..44]).unwrap(); // v44

        let (parent, ty, fb): (Vec<u8>, String, String) = conn
            .query_row(
                "SELECT parent_id, type, forecast_behavior FROM categories WHERE id = ?1",
                rusqlite::params![&ccp[..]],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(ty, "transfer");
        assert_eq!(fb, "ignore_cashflow");
        assert_eq!(parent, transfers.to_vec(), "reparented under Transfers");
        assert!(integrity_ok(&conn));
    }

    /// Every prior version → CURRENT up-migration preserves data, leaves the DB
    /// integral, and yields a deterministic read-model checksum.
    #[test]
    fn up_migration_from_every_prior_version_is_safe() {
        for v in 1..CURRENT_VERSION {
            let mut conn = Connection::open_in_memory().unwrap();
            // A golden vault at version `v`, seeded with core data.
            apply(&mut conn, &MIGRATIONS[..v as usize]).unwrap();
            seed_accounts(&conn, 3);
            let before = account_count(&conn);

            // A newer app opens it and migrates forward to CURRENT.
            apply(&mut conn, MIGRATIONS).unwrap();

            assert!(
                integrity_ok(&conn),
                "integrity_check failed after up-migration from v{v}"
            );
            assert_eq!(user_version(&conn), CURRENT_VERSION);
            assert_eq!(
                account_count(&conn),
                before,
                "accounts lost migrating up from v{v} to v{CURRENT_VERSION}"
            );
            // The read-model rebuild is deterministic: two rebuilds agree.
            projection::rebuild(&mut conn).unwrap();
            let first = projection::checksum(&conn).unwrap();
            projection::rebuild(&mut conn).unwrap();
            let second = projection::checksum(&conn).unwrap();
            assert_eq!(
                first, second,
                "read-model checksum not stable after up-migration from v{v}"
            );
        }
    }

    /// CURRENT → every prior version down-migration preserves core data and
    /// leaves the DB integral (dropping feature tables orphans nothing).
    #[test]
    fn down_migration_to_every_prior_version_is_safe() {
        for to in (1..CURRENT_VERSION).rev() {
            let mut conn = Connection::open_in_memory().unwrap();
            apply(&mut conn, MIGRATIONS).unwrap(); // a CURRENT vault
            seed_accounts(&conn, 3);
            let before = account_count(&conn);

            let reverted = migrate_down(&mut conn, to).unwrap();

            assert_eq!(
                reverted,
                (CURRENT_VERSION - to) as u64,
                "expected to revert {} migration(s) to reach v{to}",
                CURRENT_VERSION - to
            );
            assert!(
                integrity_ok(&conn),
                "integrity_check failed after down-migration to v{to}"
            );
            assert_eq!(user_version(&conn), to);
            assert_eq!(
                account_count(&conn),
                before,
                "core accounts lost migrating down to v{to}"
            );
        }
    }

    /// A full round-trip — CURRENT → baseline → CURRENT — restores the schema and
    /// preserves data, proving down and up compose without drift.
    #[test]
    fn full_down_up_round_trip_preserves_data() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();
        seed_accounts(&conn, 5);
        let before = account_count(&conn);

        migrate_down(&mut conn, 1).unwrap();
        assert_eq!(user_version(&conn), 1);
        apply(&mut conn, MIGRATIONS).unwrap();

        assert_eq!(user_version(&conn), CURRENT_VERSION);
        assert!(
            integrity_ok(&conn),
            "integrity_check failed after round-trip"
        );
        assert_eq!(
            account_count(&conn),
            before,
            "accounts lost across a down→up round-trip"
        );
    }
}
