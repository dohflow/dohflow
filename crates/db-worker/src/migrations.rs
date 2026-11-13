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
use rusqlite::{params, Connection, OptionalExtension};

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
    /// Run this migration (both directions) with foreign-key enforcement
    /// off — SQLite's documented table-rebuild procedure. The bundled SQLite
    /// enforces foreign keys by default, so dropping a parent table that has
    /// child rows fails even when an identical table replaces it in the same
    /// transaction. The pragma cannot change inside a transaction, so the
    /// runner turns it off before `BEGIN`, requires an empty
    /// `PRAGMA foreign_key_check` before `COMMIT`, then restores it. Only for
    /// a migration that rebuilds a table other tables reference.
    pub foreign_keys_off: bool,
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
        foreign_keys_off: false,
    },
    Migration {
        version: 2,
        name: "operation_log_correlation_index",
        up: "CREATE INDEX IF NOT EXISTS idx_operation_log_correlation
                ON operation_log (correlation_id);",
        down: Some("DROP INDEX IF EXISTS idx_operation_log_correlation;"),
        rebuilds_read_models: false,
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
    },
    // Account subtype (ADR 0028, personal-cfo-9dgg): an optional finer
    // classification within an account's cashflow_role (checking vs savings, credit
    // card vs line of credit, etc.). Nullable — NULL is "unspecified" and passes the
    // CHECK. The token set mirrors core-ledger's `AccountSubtype` (its single source
    // of truth); the role↔subtype match is validated in the kernel, because a column
    // CHECK cannot reference `cashflow_role` under SQLite's `ALTER ... ADD COLUMN`.
    // Only the liquid subtypes drive cash-tier rollups; the rest are foundation for
    // later grouping. Down drops the column (SQLite >= 3.35; pinned at 3.51.3).
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
    // (the audit chain must be FK-strict per z2a) — and they ARE enforced: the bundled
    // SQLite build turns `foreign_keys` on by default (found by migration 53's tests,
    // personal-cfo-r2pow; an earlier version of this note said otherwise). Cross-boundary refs
    // to canonical rows (proposed/matched account, committed txn, provenance entity)
    // stay plain BLOBs. Runtime strictness of the provenance invariant is enforced in
    // the `ingestion` module (`link_provenance` validates the source_record exists),
    // since cross-boundary refs are plain BLOBs no foreign key can check.
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
    },
    // Account-model extensions (ADR 0044, personal-cfo-4d8.22): real-asset subtypes,
    // account notes, and the loan's original principal.
    //
    // Broadening the `accounts.subtype` CHECK to admit the real-asset tokens needs a
    // COLUMN SWAP — SQLite can't ALTER a column CHECK. We add a new column carrying the
    // extended CHECK, copy across (every existing token is in the new superset, so all
    // rows pass), drop the old column, and rename. `subtype` is in no foreign key and
    // not indexed (v14's `down` already DROP COLUMNs it), so the swap touches nothing
    // else. (An earlier version of this note said FK enforcement is off; the bundled
    // SQLite defaults it on — see migration 53.)
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
    },
    // A scenario can carry a user-set expiry (ADR 0051 §3). It is filtered at READ
    // time, never flipped by a job, so rebuilds stay clock-independent (ADR 0014 §7).
    Migration {
        version: 45,
        name: "scenario_expiry",
        up: "ALTER TABLE scenarios ADD COLUMN expires_on TEXT;",
        down: Some("ALTER TABLE scenarios DROP COLUMN expires_on;"),
        rebuilds_read_models: false,
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
    },
    // Connector connections (personal-cfo-gglk, ADR 0060 §1): one row per
    // NOTE: these REFERENCES are enforced — the bundled SQLite defaults
    // `foreign_keys` on (see migration 53) — and delete_connector_connection
    // also removes links explicitly, first, so it never relies on that.
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
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
        foreign_keys_off: false,
    },
    // Local backup receipts (personal-cfo-8qh). The destination is device-local
    // and remains inside the SQLCipher vault; scheduled backups never delete
    // files from the selected folder.
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
        foreign_keys_off: false,
    },
    // The `lunchflow` source_type token (personal-cfo-r2pow, ADR 0076 decision 8:
    // keep the CHECK, widen it with one reviewed migration per adapter). SQLite
    // cannot ALTER a CHECK, and `source_type` is NOT NULL with no default, so
    // migration 36's column swap (ADD COLUMN needs a default for NOT NULL) would
    // quietly add one. Instead this is SQLite's documented table rebuild: create
    // the table exactly as v16 defined it (no later migration altered it) with
    // the wider CHECK, copy every row, drop the old table, rename, and recreate
    // its one index. It runs with `foreign_keys_off` (see [`Migration`]): the
    // four child tables' `REFERENCES source_batches(id)` keep naming the rebuilt
    // table, and the runner's `foreign_key_check` proves every child still
    // resolves before the transaction commits.
    Migration {
        version: 53,
        name: "source_type_lunchflow",
        up: "CREATE TABLE source_batches_v53 (
                id              BLOB    PRIMARY KEY,
                source_type     TEXT    NOT NULL CHECK (source_type IN (
                    'manual', 'csv', 'ofx', 'qfx', 'qif', 'pdf', 'image',
                    'simplefin', 'teller', 'plaid', 'relay', 'other', 'lunchflow')),
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
             INSERT INTO source_batches_v53 (id, source_type, source_name, file_fingerprint, parser_version, status, staged_count, committed_count, skipped_count, summary_json, imported_at, created_at, updated_at)
                 SELECT id, source_type, source_name, file_fingerprint, parser_version, status, staged_count, committed_count, skipped_count, summary_json, imported_at, created_at, updated_at FROM source_batches;
             DROP INDEX IF EXISTS idx_source_batches_fingerprint;
             DROP TABLE source_batches;
             ALTER TABLE source_batches_v53 RENAME TO source_batches;
             CREATE INDEX IF NOT EXISTS idx_source_batches_fingerprint
                 ON source_batches (file_fingerprint);",
        // Reverse: the same rebuild with the v16 CHECK. A vault holding
        // 'lunchflow' batches FAILS this copy (the narrower CHECK rejects the
        // row) and the migration's transaction rolls back — refusing, rather
        // than rewriting provenance to another token.
        down: Some(
            "CREATE TABLE source_batches_v53 (
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
             INSERT INTO source_batches_v53 (id, source_type, source_name, file_fingerprint, parser_version, status, staged_count, committed_count, skipped_count, summary_json, imported_at, created_at, updated_at)
                 SELECT id, source_type, source_name, file_fingerprint, parser_version, status, staged_count, committed_count, skipped_count, summary_json, imported_at, created_at, updated_at FROM source_batches;
             DROP INDEX IF EXISTS idx_source_batches_fingerprint;
             DROP TABLE source_batches;
             ALTER TABLE source_batches_v53 RENAME TO source_batches;
             CREATE INDEX IF NOT EXISTS idx_source_batches_fingerprint
                 ON source_batches (file_fingerprint);",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: true,
    },
    // Provider account currency on each connector link (personal-cfo-049p6,
    // ADR 0076 decision 7). Nullable: links saved before this migration stay
    // NULL until their next refresh records the provider's currency — NULL
    // means "not known", never "the base currency". The CHECK admits only an
    // uppercase ISO 4217-shaped code. Additive, so no rebuild and no
    // read-model impact.
    Migration {
        version: 54,
        name: "connector_link_currency",
        up: "ALTER TABLE connector_account_links ADD COLUMN currency TEXT
                 CHECK (currency IS NULL OR (length(currency) = 3 AND currency = upper(currency)));",
        down: Some("ALTER TABLE connector_account_links DROP COLUMN currency;"),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // The connector connection a sync batch came from (personal-cfo-6evt, ADR
    // 0014 §3 addendum 2026-10-04): a connector's identity is its connection,
    // so dedupe can tell two connections to one provider apart. Nullable:
    // file imports have no connection, and batches synced before this
    // migration stay NULL, which dedupe treats as "the same connection as any
    // later batch of the source type" (today's behavior). No foreign key: the
    // id is provenance and outlives a removed connection. Additive, so no
    // rebuild and no read-model impact.
    Migration {
        version: 55,
        name: "source_batch_connector_connection",
        up: "ALTER TABLE source_batches ADD COLUMN connector_connection_id BLOB;",
        down: Some("ALTER TABLE source_batches DROP COLUMN connector_connection_id;"),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Operation log v2 (personal-cfo-t2s4b part 4; ADR 0074, vault-format.md C.1.2 and the
    // envelope plaintext): the replayable form of a command. `payload` is the canonical
    // JSON of the WriteCommand (`db_worker::payload`), `payload_schema_version` names the
    // payload encoding it was written under, `write_set_hash` binds the class-1 rows the
    // command touched (part 5), and `engine_version` records the build that applied it.
    //
    // The four columns are nullable so that a row written before this migration stays
    // valid, and NULL is meaningful there: it is **audit history only**, never replayed and
    // never sent as a Sync envelope; `payload_schema_version IS NULL` tells the two apart.
    // `operation_log` is append-only (triggers refuse UPDATE and DELETE), so the pre-v2 rows
    // are left exactly as they are: ADD COLUMN rewrites no row and fires no trigger.
    //
    // A v2 row is all-or-nothing, enforced by one CHECK on the last-added column, which
    // can see all four: either all four are NULL, or all four are present and valid (a
    // payload, a version of at least 1, a 32-byte write-set hash, a 1 to 64 character engine
    // version). A half-written row could never be repaired in an append-only table, and
    // SQLite cannot add a CHECK to an existing column later without rebuilding the audit
    // table, so the invariant is enforced now, before anything writes the columns. The
    // consequence is deliberate: the dispatcher may write v2 columns only once it can supply
    // the write-set hash (part 5). The comparisons use COALESCE because a CHECK passes on
    // NULL. Additive, so no rebuild and no read-model impact. The down migration drops the
    // CHECK-bearing column first (SQLite refuses to drop a column another CHECK refers to).
    Migration {
        version: 56,
        name: "operation_log_v2",
        up: "ALTER TABLE operation_log ADD COLUMN payload BLOB;
             ALTER TABLE operation_log ADD COLUMN payload_schema_version INTEGER;
             ALTER TABLE operation_log ADD COLUMN write_set_hash BLOB;
             ALTER TABLE operation_log ADD COLUMN engine_version TEXT
                 CHECK (
                     (engine_version IS NULL
                         AND payload IS NULL
                         AND payload_schema_version IS NULL
                         AND write_set_hash IS NULL)
                     OR
                     (engine_version IS NOT NULL
                         AND length(engine_version) BETWEEN 1 AND 64
                         AND payload IS NOT NULL
                         AND COALESCE(payload_schema_version, 0) >= 1
                         AND COALESCE(length(write_set_hash), 0) = 32)
                 );",
        down: Some(
            "ALTER TABLE operation_log DROP COLUMN engine_version;
             ALTER TABLE operation_log DROP COLUMN write_set_hash;
             ALTER TABLE operation_log DROP COLUMN payload_schema_version;
             ALTER TABLE operation_log DROP COLUMN payload;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Tombstones for the three hard deletes (personal-cfo-egn67, SYNC-2b; ADR 0073 exceptions
    // item 1, invariant 9 "nothing is deleted"). DeleteRecurringTransfer, DeleteIncomeSource and
    // DeleteRecurringBill used to run a literal DELETE, which has no valid resolution against a
    // concurrent edit on another device (the row is gone). They now set `deleted_at` to the
    // command's `issued_at`, and a Reinstate command clears it. NULL means "not deleted", so
    // every existing row is unaffected. The bill's contract row is tombstoned with its schedule.
    Migration {
        version: 57,
        name: "tombstones_for_recurring_deletes",
        up: "ALTER TABLE recurring_transfers ADD COLUMN deleted_at TEXT;
             ALTER TABLE income_sources ADD COLUMN deleted_at TEXT;
             ALTER TABLE recurring_events ADD COLUMN deleted_at TEXT;
             ALTER TABLE bill_contracts ADD COLUMN deleted_at TEXT;",
        // Reverse: dropping the column would quietly resurrect every deleted row, so the down
        // REFUSES while any tombstone exists (the guard insert violates its CHECK, and the
        // migration's transaction rolls back, temp table included), as v53's down refuses while
        // a `lunchflow` batch exists. Reinstate or keep the rows, then roll back.
        down: Some(
            "CREATE TEMP TABLE tombstone_down_guard (ok INTEGER NOT NULL CHECK (ok = 1));
             INSERT INTO tombstone_down_guard
                 SELECT CASE WHEN EXISTS (SELECT 1 FROM recurring_transfers WHERE deleted_at IS NOT NULL)
                               OR EXISTS (SELECT 1 FROM income_sources WHERE deleted_at IS NOT NULL)
                               OR EXISTS (SELECT 1 FROM recurring_events WHERE deleted_at IS NOT NULL)
                               OR EXISTS (SELECT 1 FROM bill_contracts WHERE deleted_at IS NOT NULL)
                        THEN 0 ELSE 1 END;
             DROP TABLE tombstone_down_guard;
             ALTER TABLE bill_contracts DROP COLUMN deleted_at;
             ALTER TABLE recurring_events DROP COLUMN deleted_at;
             ALTER TABLE income_sources DROP COLUMN deleted_at;
             ALTER TABLE recurring_transfers DROP COLUMN deleted_at;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Re-authentication notices (personal-cfo-pxi.22, ADR 0060 addendum 2026-10-23). A bank that
    // needs the user to sign in again at the provider (SimpleFIN `con.auth`, a refused token, an
    // expired key) must stay visible until the NEXT healthy refresh, but `last_error` is free text
    // that every healthy sync clears, and a one-bank `con.auth` is not even a failed sync. This
    // is this device's own operational state (class 3: never replicated, ADR 0074/CLASS-0), kept in
    // its own table rather than a column because `connector_connections` is about to be
    // restructured (personal-cfo-07u). `attention` is a small JSON array the desktop layer writes
    // and reads, never the UI: [{"kind": "reauth_institution" | "relink_connection",
    // "institution": <sanitized provider-supplied bank name, optional>}, ...]. One row per
    // connection, replaced on every refresh that has something to say and removed by the next
    // healthy one.
    Migration {
        version: 58,
        name: "connector_attention",
        up: "CREATE TABLE IF NOT EXISTS connector_attention (
                 connection_id BLOB PRIMARY KEY REFERENCES connector_connections(id),
                 attention     TEXT NOT NULL CHECK (json_valid(attention)),
                 updated_at    TEXT NOT NULL
             );",
        // Reverse: the table is only a visible reminder that the next refresh rebuilds, so the
        // down drops it without a guard (nothing else references it, no ledger data is lost).
        down: Some("DROP TABLE IF EXISTS connector_attention;"),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // A recurring bill may be LINKED to the loan it pays, and the user may say a bill is NOT a
    // given loan's payment (personal-cfo-6wk.22; ADR 0035 addendum 2026-10-25). Both are user
    // data on `recurring_events` (a class-1 synced table), set only through the Link /
    // Unlink / SetBillNotLoanPayment commands, never by a raw write.
    //   * `linked_loan_account_id`: the `loan_liability` account whose payment this bill is. While
    //     the link is in force the forecast counts this bill's cash and suppresses that loan's own
    //     due-date row. Cleared in the same command when the bill is archived or deleted, or when the
    //     loan is archived, so a link only ever sits on an active bill and a live loan.
    //   * `not_payment_of_loan_account_id`: the loan the user said this bill is NOT the payment of ("No, it's a
    //     different bill"); the overlap detector skips that pair.
    // Like `autopay_account_id`, both are plain BLOBs with no foreign key (an account is archived,
    // never removed, and a dangling reference is resolved by a deterministic rule, not rejected).
    // NULL is "not linked / no answer", so every existing row is unaffected.
    Migration {
        version: 59,
        name: "bill_loan_link",
        up: "ALTER TABLE recurring_events ADD COLUMN linked_loan_account_id BLOB;
             ALTER TABLE recurring_events ADD COLUMN not_payment_of_loan_account_id BLOB;",
        // Reverse: dropping the columns would quietly forget the user's links and answers, so the
        // down REFUSES while any is set, on a live bill or a deleted one (a deleted bill keeps its
        // "not this loan's payment" answer): the guard insert violates its CHECK and the migration's
        // transaction rolls back, temp table included, as 57's down does for tombstones. Unlink
        // and clear them, then roll back.
        down: Some(
            "CREATE TEMP TABLE bill_loan_link_down_guard (ok INTEGER NOT NULL CHECK (ok = 1));
             INSERT INTO bill_loan_link_down_guard
                 SELECT CASE WHEN EXISTS (SELECT 1 FROM recurring_events
                                           WHERE deleted_at IS NULL
                                             AND (linked_loan_account_id IS NOT NULL
                                                  OR not_payment_of_loan_account_id IS NOT NULL))
                               OR EXISTS (SELECT 1 FROM recurring_events
                                           WHERE deleted_at IS NOT NULL
                                             AND not_payment_of_loan_account_id IS NOT NULL)
                        THEN 0 ELSE 1 END;
             DROP TABLE bill_loan_link_down_guard;
             ALTER TABLE recurring_events DROP COLUMN not_payment_of_loan_account_id;
             ALTER TABLE recurring_events DROP COLUMN linked_loan_account_id;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Connection identity / device-local secret split (personal-cfo-07u, SYNC-2a; ADR 0074
    // decision 5 and Addendum B; ADR 0073 item 1). `connector_connections` held, in one row, what
    // every device agrees on (which provider, what it is called) and what only THIS device may
    // hold (the provider credential, when it last refreshed, its last error). Sync must replicate
    // the first and never the second, so the table is split by class:
    //   * `connector_connections` (CLASS 1) keeps id, adapter_id, display_hint, created_at,
    //     updated_at and gains `deleted_at`: deleting a connection is a tombstone (the fourth hard
    //     delete after egn67's three), because a hard delete replicated to another device would
    //     leave that device's class-3 rows pointing at a vanished identity and wedge its rebase.
    //   * `connector_account_links` (CLASS 1) keeps the external-account -> ledger-account mapping
    //     and currency; its per-account watermark `last_synced_on` is this device's fetch cursor.
    //   * `connector_connection_secrets` (CLASS 3, NEW): credential, last_synced_at, last_error.
    //   * `connector_link_cursors` (CLASS 3, NEW): the per-account `last_synced_on` watermark.
    // The class-3 tables reference the identity (RESTRICT, class 3 -> class 1 only, never the
    // reverse); the cursor's `external_id` is DATA, not a foreign key to the links, so a mapping a
    // remote command removed never wedges a rebase. Existing rows move across unchanged. The old
    // credential column is overwritten with the empty string before it is dropped, so the free
    // pages the rebuild leaves behind do not keep the secret (the file stays SQLCipher-encrypted
    // either way). Nothing here logs a row value.
    Migration {
        version: 60,
        name: "connection_identity_secret_split",
        up: "CREATE TABLE connector_connection_secrets (
                 connection_id  BLOB PRIMARY KEY REFERENCES connector_connections(id) ON DELETE RESTRICT,
                 credential     TEXT NOT NULL,
                 last_synced_at TEXT,
                 last_error     TEXT,
                 updated_at     TEXT NOT NULL
             );
             INSERT INTO connector_connection_secrets
                    (connection_id, credential, last_synced_at, last_error, updated_at)
                 SELECT id, credential, last_synced_at, last_error, updated_at
                   FROM connector_connections;
             CREATE TABLE connector_link_cursors (
                 connection_id  BLOB NOT NULL REFERENCES connector_connections(id) ON DELETE RESTRICT,
                 external_id    TEXT NOT NULL,
                 last_synced_on TEXT NOT NULL,
                 updated_at     TEXT NOT NULL,
                 PRIMARY KEY (connection_id, external_id)
             );
             INSERT INTO connector_link_cursors (connection_id, external_id, last_synced_on, updated_at)
                 SELECT connection_id, external_id, last_synced_on, updated_at
                   FROM connector_account_links
                  WHERE last_synced_on IS NOT NULL;
             UPDATE connector_connections SET credential = '';
             ALTER TABLE connector_connections DROP COLUMN credential;
             ALTER TABLE connector_connections DROP COLUMN last_synced_at;
             ALTER TABLE connector_connections DROP COLUMN last_error;
             ALTER TABLE connector_connections ADD COLUMN deleted_at TEXT;
             ALTER TABLE connector_account_links DROP COLUMN last_synced_on;",
        // Reverse: refuses (atomically, like 57 and 59) in two cases where merging the tables
        // back would lose or invent something. A connection with NO secret row on this device (it
        // was created on another device) has no credential to put back into the old NOT NULL
        // column, and one is never invented; a tombstoned connection would be silently
        // resurrected. Otherwise the old shape is rebuilt exactly (the old `credential` column
        // is NOT NULL, so the table is rebuilt rather than altered) and the cursors go back onto
        // their links. What it cannot restore is harmless: the connection's `updated_at` is taken
        // from the identity (equal to the secret's for every pre-split row), and the cursors'
        // own `updated_at` is dropped (the old shape never had one).
        down: Some(
            "CREATE TEMP TABLE connector_split_down_needs_a_secret_for_every_connection
                 (ok INTEGER NOT NULL CHECK (ok = 1));
             INSERT INTO connector_split_down_needs_a_secret_for_every_connection
                 SELECT CASE WHEN EXISTS (SELECT 1 FROM connector_connections c
                                           WHERE NOT EXISTS (SELECT 1 FROM connector_connection_secrets s
                                                              WHERE s.connection_id = c.id))
                        THEN 0 ELSE 1 END;
             DROP TABLE connector_split_down_needs_a_secret_for_every_connection;
             CREATE TEMP TABLE connector_split_down_refuses_a_tombstone (ok INTEGER NOT NULL CHECK (ok = 1));
             INSERT INTO connector_split_down_refuses_a_tombstone
                 SELECT CASE WHEN EXISTS (SELECT 1 FROM connector_connections WHERE deleted_at IS NOT NULL)
                        THEN 0 ELSE 1 END;
             DROP TABLE connector_split_down_refuses_a_tombstone;
             ALTER TABLE connector_account_links ADD COLUMN last_synced_on TEXT;
             UPDATE connector_account_links
                SET last_synced_on = (SELECT k.last_synced_on FROM connector_link_cursors k
                                       WHERE k.connection_id = connector_account_links.connection_id
                                         AND k.external_id = connector_account_links.external_id);
             CREATE TABLE connector_connections_v48 (
                 id BLOB PRIMARY KEY,
                 adapter_id TEXT NOT NULL,
                 credential TEXT NOT NULL,
                 display_hint TEXT,
                 last_synced_at TEXT,
                 last_error TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             INSERT INTO connector_connections_v48
                    (id, adapter_id, credential, display_hint, last_synced_at, last_error, created_at, updated_at)
                 SELECT c.id, c.adapter_id, s.credential, c.display_hint, s.last_synced_at, s.last_error,
                        c.created_at, c.updated_at
                   FROM connector_connections c
                   JOIN connector_connection_secrets s ON s.connection_id = c.id;
             DROP TABLE connector_link_cursors;
             DROP TABLE connector_connection_secrets;
             DROP TABLE connector_connections;
             ALTER TABLE connector_connections_v48 RENAME TO connector_connections;",
        ),
        rebuilds_read_models: false,
        // The down rebuilds a table other tables reference (the links and the attention notice),
        // SQLite's documented rebuild procedure; both directions run with enforcement off and
        // require an empty foreign_key_check before COMMIT.
        foreign_keys_off: true,
    },
    // The household record (personal-cfo-339): `households`, `household_members` and `profiles`,
    // all class 1. The timezone moves from `vault_metadata` onto the household row; the
    // post-step in `POST_UP` seeds an existing vault's household and member (ids derived from the
    // vault id, every column from the vault's own creation time, never this migration's clock)
    // and then drops the old column. Schema-only for a fresh vault, whose household is seeded
    // when its `vault_metadata` row is created.
    Migration {
        version: 61,
        name: "households",
        up: "CREATE TABLE households (
                 id                    BLOB PRIMARY KEY,
                 name                  TEXT NOT NULL,
                 household_timezone    TEXT NOT NULL,
                 platform_household_id TEXT,
                 created_at            TEXT NOT NULL,
                 updated_at            TEXT NOT NULL
             );
             CREATE TRIGGER households_single_row BEFORE INSERT ON households
             WHEN (SELECT COUNT(*) FROM households) >= 1
             BEGIN
                 SELECT RAISE(ABORT, 'a vault has exactly one household');
             END;
             CREATE TABLE household_members (
                 id                 BLOB PRIMARY KEY,
                 household_id       BLOB NOT NULL REFERENCES households(id) ON DELETE RESTRICT,
                 display_name       TEXT NOT NULL,
                 role               TEXT NOT NULL CHECK (role IN ('primary', 'partner')),
                 platform_member_id TEXT,
                 created_at         TEXT NOT NULL,
                 deleted_at         TEXT
             );
             CREATE INDEX idx_household_members_household ON household_members (household_id);
             CREATE UNIQUE INDEX idx_household_members_one_live_primary
                 ON household_members (household_id)
                 WHERE role = 'primary' AND deleted_at IS NULL;
             CREATE TABLE profiles (
                 member_id  BLOB PRIMARY KEY REFERENCES household_members(id) ON DELETE RESTRICT,
                 initials   TEXT,
                 color      TEXT,
                 updated_at TEXT NOT NULL
             );",
        // Reverse: refuses (atomically, like 57, 59 and 60) unless the three tables are EXACTLY
        // the backfill state, because dropping them would lose the household name, a renamed
        // primary, every other member, every profile and the platform ids. In that state the
        // timezone goes back onto `vault_metadata`, which is rebuilt in its old shape (the old
        // column was NOT NULL without a default).
        down: Some(
            "CREATE TEMP TABLE households_down_needs_the_backfill_state (ok INTEGER NOT NULL CHECK (ok = 1));
             INSERT INTO households_down_needs_the_backfill_state
                 SELECT CASE WHEN
                        ((SELECT COUNT(*) FROM households) = 0
                         AND (SELECT COUNT(*) FROM household_members) = 0
                         AND (SELECT COUNT(*) FROM profiles) = 0)
                     OR ((SELECT COUNT(*) FROM households) = 1
                         AND EXISTS (SELECT 1 FROM households
                                      WHERE name = 'My household' AND platform_household_id IS NULL)
                         AND (SELECT COUNT(*) FROM household_members) = 1
                         AND EXISTS (SELECT 1 FROM household_members
                                      WHERE role = 'primary' AND display_name = 'Me'
                                        AND platform_member_id IS NULL AND deleted_at IS NULL)
                         AND (SELECT COUNT(*) FROM profiles) = 0)
                    THEN 1 ELSE 0 END;
             DROP TABLE households_down_needs_the_backfill_state;
             CREATE TABLE vault_metadata_v60 (
                 singleton          INTEGER PRIMARY KEY CHECK (singleton = 1),
                 vault_id           BLOB    NOT NULL,
                 schema_version     INTEGER NOT NULL,
                 envelope_version   INTEGER NOT NULL,
                 kdf_algorithm      TEXT    NOT NULL,
                 kdf_memory_kib     INTEGER NOT NULL,
                 kdf_time_cost      INTEGER NOT NULL,
                 kdf_parallelism    INTEGER NOT NULL,
                 household_timezone TEXT    NOT NULL,
                 manifest_pointer   TEXT,
                 created_at         TEXT    NOT NULL
             );
             INSERT INTO vault_metadata_v60
                    (singleton, vault_id, schema_version, envelope_version, kdf_algorithm,
                     kdf_memory_kib, kdf_time_cost, kdf_parallelism, household_timezone,
                     manifest_pointer, created_at)
                 SELECT v.singleton, v.vault_id, v.schema_version, v.envelope_version,
                        v.kdf_algorithm, v.kdf_memory_kib, v.kdf_time_cost, v.kdf_parallelism,
                        COALESCE((SELECT h.household_timezone FROM households h), 'UTC'), v.manifest_pointer,
                        v.created_at
                   FROM vault_metadata v;
             DROP TABLE vault_metadata;
             ALTER TABLE vault_metadata_v60 RENAME TO vault_metadata;
             DROP TABLE profiles;
             DROP TABLE household_members;
             DROP TRIGGER households_single_row;
             DROP TABLE households;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Scenario tombstone (personal-cfo-5ymg8.4): `scenarios` is class 1 and a delete is a
    // tombstone, not a purge (ADR 0074 Decision 7 and Addendum H). One nullable add, no backfill:
    // every existing scenario is live. A scenario's events are not touched by a delete, so there
    // is nothing to migrate in `forecast_assumption_events`.
    Migration {
        version: 62,
        name: "scenario_tombstone",
        up: "ALTER TABLE scenarios ADD COLUMN deleted_at TEXT;",
        // Reverse: refuses (atomically, like 57, 59, 60 and 61) while any scenario is tombstoned,
        // because dropping the column would silently resurrect it.
        down: Some(
            "CREATE TEMP TABLE scenarios_down_needs_no_tombstone (ok INTEGER NOT NULL CHECK (ok = 1));
             INSERT INTO scenarios_down_needs_no_tombstone
                 SELECT CASE WHEN EXISTS (SELECT 1 FROM scenarios WHERE deleted_at IS NOT NULL)
                             THEN 0 ELSE 1 END;
             DROP TABLE scenarios_down_needs_no_tombstone;
             ALTER TABLE scenarios DROP COLUMN deleted_at;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Durable commit marker (personal-cfo-g3m.31): `committed_at` is set in the same transaction as a
    // running job's durable effect (the connector's import commit, the backup's receipt), and
    // `committed_outcome` says what committed: 'succeeded', or 'failed' for a backup receipt that
    // records a failure. Together they let recovery and every finish path tell "nothing committed" from
    // "committed", and record the truth either way. The two are always set and cleared together (the
    // paired CHECK). `durable_jobs` stays class 3 (device-local, never synced). Both are transient: set
    // while a run is in flight, cleared when the run is claimed again and at every terminal write. Two
    // nullable adds, no backfill: no existing row has committed anything it could not already say.
    Migration {
        version: 63,
        name: "durable_job_commit_marker",
        up: "ALTER TABLE durable_jobs ADD COLUMN committed_at TEXT;
             ALTER TABLE durable_jobs ADD COLUMN committed_outcome TEXT
                 CHECK (committed_outcome IS NULL OR committed_outcome IN ('succeeded', 'failed'))
                 CHECK ((committed_at IS NULL) = (committed_outcome IS NULL));",
        // Reverse: refuses (atomically, like 57, 59, 60, 61 and 62) while a running job carries a
        // marker, because dropping the columns would make a committed run look uncommitted to recovery.
        down: Some(
            "CREATE TEMP TABLE durable_jobs_down_needs_no_marker (ok INTEGER NOT NULL CHECK (ok = 1));
             INSERT INTO durable_jobs_down_needs_no_marker
                 SELECT CASE WHEN EXISTS (SELECT 1 FROM durable_jobs WHERE committed_at IS NOT NULL)
                             THEN 0 ELSE 1 END;
             DROP TABLE durable_jobs_down_needs_no_marker;
             ALTER TABLE durable_jobs DROP COLUMN committed_outcome;
             ALTER TABLE durable_jobs DROP COLUMN committed_at;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // personal-cfo-5ymg8.7 (ADR 0074 Decision 5): the household's preferences are `user.*` keys,
    // written only by `SetUserSetting`. The five unprefixed names every earlier build wrote are
    // renamed once, here, in the migration transaction (the owner chose an eager rename over a
    // dual-read fallback, D-ST1): value and `updated_at` are kept. A prefixed row already present
    // wins, so the DELETE of the legacy row comes FIRST; otherwise the UPDATE would hit the primary
    // key and abort. `locale` is not listed: no build ever wrote it. The migration runs before the
    // write-set capture is installed, so the rename is not captured as a change.
    //
    // Why rewriting replicated rows in a migration is safe NOW: no vault has a genesis snapshot yet
    // (sync is not wired into any build), so no replica holds a copy of these rows under the old
    // names. Once sync ships, a migration that rewrites rows of a replicated table needs its own
    // rule (what a replica that has not migrated sees, and how the rewrite reaches it), so this
    // is not a pattern to repeat after that point.
    Migration {
        version: 64,
        name: "household_settings_user_prefix",
        up: "DELETE FROM settings
                 WHERE key IN ('reporting_currency', 'minimum_cash_floor_minor',
                               'comfort_band_upper_minor', 'auto_categorize_on_import',
                               'future_cash_series_selection')
                   AND EXISTS (SELECT 1 FROM settings AS prefixed
                                WHERE prefixed.key = 'user.' || settings.key);
             UPDATE settings SET key = 'user.' || key
                 WHERE key IN ('reporting_currency', 'minimum_cash_floor_minor',
                               'comfort_band_upper_minor', 'auto_categorize_on_import',
                               'future_cash_series_selection');",
        // Reverse: drop the prefix of the five keys (a vault being taken down to schema 63).
        down: Some(
            "UPDATE settings SET key = substr(key, 6)
                 WHERE key IN ('user.reporting_currency', 'user.minimum_cash_floor_minor',
                               'user.comfort_band_upper_minor', 'user.auto_categorize_on_import',
                               'user.future_cash_series_selection');",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // personal-cfo-0g528.1: the position of a staged transaction within its batch. The materialized
    // import commit derives its ledger transaction id from (source batch, row ordinal) instead of
    // the staged row's own id, so the id is the same on every device and a re-upload (a new batch)
    // gets new ids. Staging is class 3 (scratch), so the column replicates nowhere. Rows staged
    // before this migration are numbered 1.. per batch in insertion order (rowid), the order the
    // staging paths wrote them; the staging paths number new rows the same way.
    Migration {
        version: 65,
        name: "staged_transactions_row_ordinal",
        up: "ALTER TABLE staged_transactions ADD COLUMN row_ordinal INTEGER;
             UPDATE staged_transactions SET row_ordinal = (
                 SELECT numbered.n FROM (
                     SELECT st.id AS id,
                            ROW_NUMBER() OVER (PARTITION BY sr.source_batch_id ORDER BY st.rowid) AS n
                       FROM staged_transactions st
                       JOIN source_records sr ON sr.id = st.source_record_id
                 ) numbered
                 WHERE numbered.id = staged_transactions.id);",
        down: Some("ALTER TABLE staged_transactions DROP COLUMN row_ordinal;"),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // SYNC-1c local state (personal-cfo-bxsbz.1; ADR 0074 decision 2, ADR 0073 section 4). The
    // outbox, the queue of commands awaiting a user disposition and the record of the base image
    // are DEVICE-LOCAL class-3 tables: never replicated, never part of a snapshot or a write set.
    //
    // * `sync_base` is a singleton. A row exists only once Sync is enabled on this device, and that
    //   is what turns outbox recording on: a vault that never enabled Sync grows nothing. `state`
    //   is NoBase (no image has been captured, so `base_digest` is NULL), Ready, or Rebasing (a
    //   swap is in flight). `base_digest` is the digest of `sync/base.db`; a restored vault whose
    //   digest does not match the file on disk has no base (the file is not carried by a backup).
    // * `sync_outbox` holds one row per unpushed local command, keyed by `command_id` and written
    //   in the same transaction as the command's operation-log row (the log itself stays
    //   append-only, so outbox state lives only here). `write_set_keys` is the canonical list of
    //   the command's changed class-1 rows, `(table, primary key)` pairs in write-set order, which
    //   is what the rebase compares against pulled envelopes. The sealed container exists once the
    //   entry is Approved; Bound adds the server seq and the result reference. The CHECK ties each
    //   state to the columns it needs, so a half-advanced row cannot be stored.
    // * `outbox_queue` keeps the commands a rebase queued for the user: one record per command,
    //   grouped by `correlation_id`, with the ADR 0073 reason, the preserved envelope and the audit
    //   evidence. Nothing here is ever dropped by a rebase.
    //
    // `operation_log` gains two device-local, nullable columns, defined exactly:
    // `applied_base_seq` is the server seq of the base the row was applied on (for a re-applied
    // local row, the new base's seq); `rebased_from` is the `op_seq` this row replaced when it was
    // re-applied in a rebase, otherwise NULL. ADD COLUMN rewrites no row and fires no trigger, so the
    // append-only triggers are unchanged, and both columns are excluded from every cross-replica
    // comparison (see `sync_state::DEVICE_LOCAL_OPERATION_LOG_COLUMNS`).
    Migration {
        version: 66,
        name: "sync_local_state",
        up: "CREATE TABLE sync_base (
                 singleton   INTEGER PRIMARY KEY CHECK (singleton = 1),
                 base_seq    INTEGER NOT NULL CHECK (base_seq >= 0),
                 base_digest BLOB CHECK (base_digest IS NULL OR length(base_digest) = 32),
                 state       TEXT NOT NULL CHECK (state IN ('NoBase', 'Ready', 'Rebasing')),
                 updated_at  TEXT NOT NULL,
                 CHECK ((state = 'NoBase') = (base_digest IS NULL))
             );
             CREATE TABLE sync_outbox (
                 command_id       BLOB PRIMARY KEY CHECK (length(command_id) = 16),
                 op_seq           INTEGER NOT NULL UNIQUE,
                 state            TEXT NOT NULL DEFAULT 'Pending'
                                  CHECK (state IN ('Pending', 'Approved', 'Bound')),
                 write_set_keys   BLOB NOT NULL,
                 sealed_container BLOB,
                 bound_seq        INTEGER,
                 result_ref       BLOB,
                 created_at       TEXT NOT NULL,
                 CHECK (
                     (state = 'Pending' AND sealed_container IS NULL
                         AND bound_seq IS NULL AND result_ref IS NULL)
                     OR (state = 'Approved' AND sealed_container IS NOT NULL
                         AND bound_seq IS NULL AND result_ref IS NULL)
                     OR (state = 'Bound' AND sealed_container IS NOT NULL
                         AND bound_seq IS NOT NULL AND result_ref IS NOT NULL)
                 )
             );
             CREATE TABLE outbox_queue (
                 command_id     BLOB PRIMARY KEY CHECK (length(command_id) = 16),
                 correlation_id BLOB NOT NULL CHECK (length(correlation_id) = 16),
                 causation_id   BLOB CHECK (causation_id IS NULL OR length(causation_id) = 16),
                 reason         TEXT NOT NULL CHECK (reason IN ('queue_choose', 'queue_edit')),
                 origin_node_id BLOB NOT NULL CHECK (length(origin_node_id) = 16),
                 queued_at      TEXT NOT NULL,
                 envelope       BLOB NOT NULL,
                 audit_evidence BLOB NOT NULL
             );
             CREATE INDEX idx_outbox_queue_correlation ON outbox_queue (correlation_id);
             ALTER TABLE operation_log ADD COLUMN applied_base_seq INTEGER
                 CHECK (applied_base_seq IS NULL OR applied_base_seq >= 0);
             ALTER TABLE operation_log ADD COLUMN rebased_from INTEGER
                 CHECK (rebased_from IS NULL OR rebased_from >= 1);",
        down: Some(
            "ALTER TABLE operation_log DROP COLUMN rebased_from;
             ALTER TABLE operation_log DROP COLUMN applied_base_seq;
             DROP INDEX IF EXISTS idx_outbox_queue_correlation;
             DROP TABLE outbox_queue;
             DROP TABLE sync_outbox;
             DROP TABLE sync_base;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // The issue time of an unpushed command (personal-cfo-bxsbz.4, AC0). A rebase re-applies a
    // local command under its COMPLETE original `CommandMeta`, and `issued_at` is part of it: the
    // ids `apply` derives and the `deleted_at` stamps it writes come from it, and the envelope's
    // immutable-metadata commitment binds it. Nothing else records it for a command that has not
    // been sealed (the operation log keeps the wall clock, not the issue time), so the outbox row
    // does. Microseconds since the Unix epoch, bounded as an envelope's `issued_at` is (C.6); NULL
    // only for a row written before this migration, which a rebase refuses by name.
    Migration {
        version: 67,
        name: "sync_outbox_issued_at",
        up: "ALTER TABLE sync_outbox ADD COLUMN issued_at INTEGER
                 CHECK (issued_at IS NULL OR (issued_at >= 0 AND issued_at <= 253402300799999999));",
        down: Some("ALTER TABLE sync_outbox DROP COLUMN issued_at;"),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // SYNC-3d device-local state (personal-cfo-1cltt.4; vault-format.md C.4 "Counter register and
    // nonce state", C.6 freshness rule 5). Three more class-3 tables: never replicated, never part
    // of a snapshot or a write set.
    //
    // * `sync_counter_register` is the durable `next_counter` of every
    //   `(vault_id, key_epoch, fingerprint, object_type)` context. A row is never 0 (the first
    //   advance goes to 1), so a witness at exactly 0 can only mean "no counter was handed out".
    //   The advance is a compare-and-set, and it is written under the writer lock, so a backup
    //   restore rolls it back: that rollback is what the independent witness detects.
    // * `sync_saved_snapshot` is the sealed snapshot saved for an exact-byte retransmission, one per
    //   context, with the hashes that say which publish it belongs to. The container bytes are a
    //   file beside the vault, not a column.
    // * `sync_genesis` is a singleton holding the genesis records this device minted at
    //   sync-enable, persisted BEFORE the keys are installed and before anything is published:
    //   the start of the accepted epoch chain. `state` only moves forward (Persisted, Installed,
    //   Published); VaultKnown is the terminal state of a genesis the service refused because the
    //   vault already exists, which is never published or re-minted; GenesisResumeLost is the
    //   terminal state of a genesis whose snapshot the service holds but whose saved container is
    //   gone (persisted so that a restart never retries and burns a counter). `export_high_water`
    //   is the `operation_log` high-water mark (the largest `op_seq`, 0 for an empty log) read in
    //   the transaction that exported the genesis snapshot: NULL until an export is made, and
    //   replaced by a later export only while nothing has been sealed. After the genesis is
    //   published, a log that has grown past the mark holds local commands the genesis does not
    //   carry and no envelope has sequenced.
    Migration {
        version: 68,
        name: "sync_register_and_genesis",
        up: "CREATE TABLE sync_counter_register (
                 vault_id     BLOB NOT NULL CHECK (length(vault_id) = 16),
                 key_epoch    INTEGER NOT NULL CHECK (key_epoch >= 1),
                 fingerprint  BLOB NOT NULL CHECK (length(fingerprint) = 32),
                 object_type  INTEGER NOT NULL CHECK (object_type IN (1, 2)),
                 next_counter INTEGER NOT NULL CHECK (next_counter >= 1),
                 PRIMARY KEY (vault_id, key_epoch, fingerprint, object_type)
             );
             CREATE TABLE sync_saved_snapshot (
                 vault_id         BLOB NOT NULL CHECK (length(vault_id) = 16),
                 key_epoch        INTEGER NOT NULL CHECK (key_epoch >= 1),
                 fingerprint      BLOB NOT NULL CHECK (length(fingerprint) = 32),
                 object_type      INTEGER NOT NULL CHECK (object_type IN (1, 2)),
                 object_id        BLOB NOT NULL CHECK (length(object_id) = 16),
                 base_seq         INTEGER NOT NULL CHECK (base_seq >= 0),
                 plaintext_sha256 BLOB NOT NULL CHECK (length(plaintext_sha256) = 32),
                 container_sha256 BLOB NOT NULL CHECK (length(container_sha256) = 32),
                 container_len    INTEGER NOT NULL CHECK (container_len >= 0),
                 counter          INTEGER NOT NULL CHECK (counter >= 0),
                 PRIMARY KEY (vault_id, key_epoch, fingerprint, object_type)
             );
             CREATE TABLE sync_genesis (
                 singleton         INTEGER PRIMARY KEY CHECK (singleton = 1),
                 vault_id          BLOB NOT NULL CHECK (length(vault_id) = 16),
                 membership_record BLOB NOT NULL CHECK (length(membership_record) > 0),
                 epoch_record      BLOB NOT NULL CHECK (length(epoch_record) > 0),
                 state             TEXT NOT NULL
                                   CHECK (state IN ('Persisted', 'Installed', 'Published', 'VaultKnown',
                                                    'GenesisResumeLost')),
                 export_high_water INTEGER
                                   CHECK (export_high_water IS NULL OR export_high_water >= 0),
                 persisted_at      TEXT NOT NULL,
                 updated_at        TEXT NOT NULL
             );",
        down: Some(
            "DROP TABLE sync_genesis;
             DROP TABLE sync_saved_snapshot;
             DROP TABLE sync_counter_register;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // SYNC-3e device-local state (personal-cfo-1cltt.5; vault-format.md B.2, C.8, C.10). Two more
    // class-3 tables: never replicated, never part of a snapshot or a write set.
    //
    // * `sync_voids` is the void ledger (ADR 0074 Addendum D and E): the voids this replica
    //   skips, each with the id of the void record, the range of log positions it skips and its
    //   `origin`: `seeded` from the snapshot the replica bootstrapped from (recorded by the
    //   snapshot's producer, never judged here, so never a fork check's input) or `honored` here.
    //   The table is append-only: an entry is never updated or deleted (a re-bootstrap builds a
    //   NEW vault), `ordinal` is dense and in the order the entries were written (a trigger
    //   refuses any other), and the ranges ascend without overlap (a trigger refuses any other).
    //   The key is the ordinal, not the void id: a snapshot may record one id at two ranges (the
    //   verifier does not refuse it), and a void a snapshot recorded may later be honored here as a
    //   new void (the seeded entry is history and never a fork check's input), so one id can hold
    //   more than one entry.
    // * `sync_position` is a singleton: `snapshot_root`, the last log position of the snapshot the
    //   replica bootstrapped from (NULL for the origin, which was never bootstrapped, and never
    //   changed after the row is written), and `next_to_apply`, the lowest position the replica has
    //   neither applied nor skipped. `next_to_apply` can pass positions the replica never applied
    //   (it moves past a range once a void for it is honored), so it is not derivable from
    //   `sync_base.base_seq`; it only increases. A row exists only once a replica has bootstrapped
    //   or Sync is armed on an origin: the ledger and the cursor are written in one transaction.
    Migration {
        version: 69,
        name: "sync_void_ledger_and_position",
        up: "CREATE TABLE sync_voids (
                 ordinal INTEGER PRIMARY KEY CHECK (ordinal >= 1),
                 void_id BLOB NOT NULL CHECK (length(void_id) = 32),
                 first   INTEGER NOT NULL CHECK (first >= 1),
                 last    INTEGER NOT NULL CHECK (last >= first),
                 origin  TEXT NOT NULL CHECK (origin IN ('honored', 'seeded'))
             );
             CREATE TRIGGER sync_voids_next_ordinal BEFORE INSERT ON sync_voids
             WHEN NEW.ordinal <> COALESCE((SELECT MAX(ordinal) FROM sync_voids), 0) + 1
             BEGIN
                 SELECT RAISE(ABORT, 'a void takes the next ordinal');
             END;
             CREATE TRIGGER sync_voids_ascending BEFORE INSERT ON sync_voids
             WHEN COALESCE((SELECT MAX(last) FROM sync_voids), 0) >= NEW.first
             BEGIN
                 SELECT RAISE(ABORT, 'void ranges ascend without overlap');
             END;
             CREATE TRIGGER sync_voids_no_update BEFORE UPDATE ON sync_voids
             BEGIN
                 SELECT RAISE(ABORT, 'sync_voids is append-only');
             END;
             CREATE TRIGGER sync_voids_no_delete BEFORE DELETE ON sync_voids
             BEGIN
                 SELECT RAISE(ABORT, 'sync_voids is append-only');
             END;
             CREATE TABLE sync_position (
                 singleton     INTEGER PRIMARY KEY CHECK (singleton = 1),
                 snapshot_root INTEGER CHECK (snapshot_root IS NULL OR snapshot_root >= 0),
                 next_to_apply INTEGER NOT NULL CHECK (next_to_apply >= 1),
                 CHECK (snapshot_root IS NULL OR next_to_apply > snapshot_root)
             );
             CREATE TRIGGER sync_position_forward BEFORE UPDATE ON sync_position
             WHEN NEW.next_to_apply < OLD.next_to_apply
                 OR NEW.snapshot_root IS NOT OLD.snapshot_root
             BEGIN
                 SELECT RAISE(ABORT, 'sync_position only moves forward and keeps its root');
             END;
             CREATE TRIGGER sync_position_no_delete BEFORE DELETE ON sync_position
             BEGIN
                 SELECT RAISE(ABORT, 'sync_position is never deleted');
             END;",
        down: Some(
            "DROP TRIGGER sync_position_no_delete;
             DROP TRIGGER sync_position_forward;
             DROP TABLE sync_position;
             DROP TRIGGER sync_voids_no_delete;
             DROP TRIGGER sync_voids_no_update;
             DROP TRIGGER sync_voids_ascending;
             DROP TRIGGER sync_voids_next_ordinal;
             DROP TABLE sync_voids;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Version 70 (personal-cfo-0g528.2, S2c part 2; provisional: renumbered above the void ledger's
    // 69 when that landed first):
    // * `sync_artifacts` is the device-local mapping from a class-1b artifact's service address in
    //   an epoch (`sync_artifact_id_e`) to the local identity that carries it, with the ciphertext
    //   hash this device verified when it published the artifact or fetched it (the only hash a
    //   producer may bind; a cache for the no-refetch rule that never decides a verdict) and, for
    //   a producer, whether a durable receipt matched. It is class 3: never replicated, exported
    //   or snapshotted.
    // * For an attachment received from a peer, the `staged_*` columns hold the receiver-local
    //   crypto fields (storage id, wrapped content key and nonces, algorithm, size, MIME type)
    //   from the moment the bytes are materialized until the attach command's `apply` creates the
    //   class-4 `attachments` row from them. The same transaction then sets them to NULL, so the
    //   `attachments` row is the only long-lived copy of the key material of that artifact (several
    //   rows may hold or stage one blob, each with a copy of its key material: spec 48). While staged
    //   they are re-wrapped and re-addressed by a key rotation, like an `attachments` row (ADR 0083).
    //   The CHECKs tie them to an attachment, all present or all absent.
    Migration {
        version: 70,
        name: "sync_artifacts",
        up: "CREATE TABLE sync_artifacts (
                 key_epoch                  INTEGER NOT NULL CHECK (key_epoch >= 1),
                 sync_artifact_id           BLOB NOT NULL CHECK (length(sync_artifact_id) = 32),
                 ciphertext_sha256          BLOB NOT NULL CHECK (length(ciphertext_sha256) = 32),
                 kind                       INTEGER NOT NULL CHECK (kind IN (1, 2)),
                 logical_id                 BLOB NOT NULL CHECK (length(logical_id) = 16),
                 part_index                 INTEGER,
                 part_count                 INTEGER,
                 receipt_at                 TEXT,
                 staged_storage_id          TEXT,
                 staged_wrapped_content_key BLOB,
                 staged_content_key_nonce   BLOB,
                 staged_content_nonce       BLOB,
                 staged_content_alg         TEXT,
                 staged_plaintext_size      INTEGER,
                 staged_mime_type           TEXT,
                 PRIMARY KEY (key_epoch, sync_artifact_id),
                 CHECK (
                     (kind = 1 AND part_index IS NULL AND part_count IS NULL)
                     OR (kind = 2 AND part_index IS NOT NULL AND part_count IS NOT NULL
                         AND part_count >= 1 AND part_index >= 0 AND part_index < part_count)
                 ),
                 CHECK (
                     (staged_storage_id IS NULL AND staged_wrapped_content_key IS NULL
                      AND staged_content_key_nonce IS NULL AND staged_content_nonce IS NULL
                      AND staged_content_alg IS NULL AND staged_plaintext_size IS NULL
                      AND staged_mime_type IS NULL)
                     OR (kind = 1 AND staged_storage_id IS NOT NULL
                         AND staged_wrapped_content_key IS NOT NULL
                         AND staged_content_key_nonce IS NOT NULL
                         AND staged_content_nonce IS NOT NULL
                         AND staged_content_alg IS NOT NULL
                         AND staged_plaintext_size IS NOT NULL AND staged_plaintext_size >= 0)
                 )
             );
             CREATE INDEX idx_sync_artifacts_logical
                 ON sync_artifacts (kind, logical_id, key_epoch);
             CREATE UNIQUE INDEX idx_sync_artifacts_staged_storage
                 ON sync_artifacts (staged_storage_id) WHERE staged_storage_id IS NOT NULL;",
        down: Some(
            "DROP INDEX idx_sync_artifacts_staged_storage;
             DROP INDEX idx_sync_artifacts_logical;
             DROP TABLE sync_artifacts;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Version 71 (personal-cfo-0g528.2, S2c part 2; provisional, one above the artifact mapping's
    // version, so it moves with it):
    // * `sync_notices` is the durable, device-local notice that Sync discarded something the user
    //   was in the middle of. Its only kind today is `import_discarded`: a received batch replaced
    //   the rows of an import that was still being reviewed on this device, and the staged
    //   import for that batch was removed in the same transaction (C.9), so the user must run the
    //   import again. `source_name` is the file name the user chose, so the notice can name it; it
    //   is shown to the user and never logged. Class 3: never replicated, exported or snapshotted.
    Migration {
        version: 71,
        name: "sync_notices",
        up: "CREATE TABLE sync_notices (
                 id              INTEGER PRIMARY KEY AUTOINCREMENT,
                 kind            TEXT NOT NULL CHECK (kind IN ('import_discarded')),
                 batch_id        BLOB NOT NULL CHECK (length(batch_id) = 16),
                 source_name     TEXT,
                 discarded_count INTEGER NOT NULL CHECK (discarded_count >= 0),
                 created_at      TEXT NOT NULL,
                 dismissed_at    TEXT
             );
             CREATE INDEX idx_sync_notices_pending ON sync_notices (dismissed_at, id);",
        down: Some(
            "DROP INDEX idx_sync_notices_pending;
             DROP TABLE sync_notices;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // An accepted command may have no result (personal-cfo-bxsbz.5). Migration 66 required a
    // `Bound` row's `result_ref` to be non-null, but the protocol allows an accepted command with
    // none, and the accepted-envelope record (C.10) says a PRESENT `result_ref` is 1 to 64 bytes
    // because an empty one would be a second encoding of an absent one. So the `Bound` arm now
    // allows NULL or 1 to 64 bytes; `Pending` and `Approved` still require NULL. SQLite cannot
    // alter a CHECK, so the table is rebuilt in the migration transaction (create, copy, drop,
    // rename). Nothing references `sync_outbox` (no foreign key, trigger or view), its only index is
    // the `op_seq` uniqueness the new table declares again, and every other constraint is kept,
    // `issued_at` (migration 67) included. A row that could only have meant "no result" under the
    // old shape, an empty `result_ref`, is carried over as NULL; any other out-of-range value
    // fails the copy and so the migration, rather than being truncated. The down migration
    // rebuilds the old shape and FAILS CLOSED: a `Bound` row with a NULL result violates the old
    // CHECK, the copy aborts, and the migration rolls back. It never invents a value.
    Migration {
        version: 72,
        name: "sync_outbox_bound_without_result",
        up: "CREATE TABLE sync_outbox_rebuilt (
                 command_id       BLOB PRIMARY KEY CHECK (length(command_id) = 16),
                 op_seq           INTEGER NOT NULL UNIQUE,
                 state            TEXT NOT NULL DEFAULT 'Pending'
                                  CHECK (state IN ('Pending', 'Approved', 'Bound')),
                 write_set_keys   BLOB NOT NULL,
                 sealed_container BLOB,
                 bound_seq        INTEGER,
                 result_ref       BLOB,
                 created_at       TEXT NOT NULL,
                 issued_at        INTEGER
                     CHECK (issued_at IS NULL OR (issued_at >= 0 AND issued_at <= 253402300799999999)),
                 CHECK (
                     (state = 'Pending' AND sealed_container IS NULL
                         AND bound_seq IS NULL AND result_ref IS NULL)
                     OR (state = 'Approved' AND sealed_container IS NOT NULL
                         AND bound_seq IS NULL AND result_ref IS NULL)
                     OR (state = 'Bound' AND sealed_container IS NOT NULL
                         AND bound_seq IS NOT NULL
                         AND (result_ref IS NULL OR length(result_ref) BETWEEN 1 AND 64))
                 )
             );
             INSERT INTO sync_outbox_rebuilt
                 (command_id, op_seq, state, write_set_keys, sealed_container, bound_seq, result_ref,
                 created_at, issued_at)
             SELECT command_id, op_seq, state, write_set_keys, sealed_container, bound_seq,
                    CASE WHEN length(result_ref) = 0 THEN NULL ELSE result_ref END,
                    created_at, issued_at
               FROM sync_outbox;
             DROP TABLE sync_outbox;
             ALTER TABLE sync_outbox_rebuilt RENAME TO sync_outbox;",
        down: Some(
            "CREATE TABLE sync_outbox_rebuilt (
                 command_id       BLOB PRIMARY KEY CHECK (length(command_id) = 16),
                 op_seq           INTEGER NOT NULL UNIQUE,
                 state            TEXT NOT NULL DEFAULT 'Pending'
                                  CHECK (state IN ('Pending', 'Approved', 'Bound')),
                 write_set_keys   BLOB NOT NULL,
                 sealed_container BLOB,
                 bound_seq        INTEGER,
                 result_ref       BLOB,
                 created_at       TEXT NOT NULL,
                 issued_at        INTEGER
                     CHECK (issued_at IS NULL OR (issued_at >= 0 AND issued_at <= 253402300799999999)),
                 CHECK (
                     (state = 'Pending' AND sealed_container IS NULL
                         AND bound_seq IS NULL AND result_ref IS NULL)
                     OR (state = 'Approved' AND sealed_container IS NOT NULL
                         AND bound_seq IS NULL AND result_ref IS NULL)
                     OR (state = 'Bound' AND sealed_container IS NOT NULL
                         AND bound_seq IS NOT NULL
                         AND result_ref IS NOT NULL)
                 )
             );
             INSERT INTO sync_outbox_rebuilt
                 (command_id, op_seq, state, write_set_keys, sealed_container, bound_seq, result_ref,
                 created_at, issued_at)
             SELECT command_id, op_seq, state, write_set_keys, sealed_container, bound_seq, result_ref,
                 created_at, issued_at
               FROM sync_outbox;
             DROP TABLE sync_outbox;
             ALTER TABLE sync_outbox_rebuilt RENAME TO sync_outbox;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Version 73 (personal-cfo-1cltt.5, slice C; provisional: it moves if another migration lands first).
    // Two device-local additions:
    // * `operation_log.issued_at` (INTEGER, nullable, Unix microseconds, the encoding of
    //   `sync_outbox.issued_at`) is the instant the command was issued, which `apply` reads for
    //   every id and timestamp it mints. Until now it lived only in the outbox row, so a command
    //   taken after the genesis export and before Sync was armed (no outbox row) lost it and could
    //   not be replayed faithfully. `apply` now records it on every row (a local command, a server
    //   envelope, a rebase step), Sync on or off. A row written before this column has NULL. It is
    //   device-local like `created_at`: outside the write set, and out of every cross-replica and
    //   snapshot comparison.
    // * `sync_genesis` records **which snapshot the genesis was and the SHA-256 of the container this
    // device sent**: `genesis_snapshot_id` and `genesis_container_sha256`, written in the transaction
    // that records the genesis as published and BEFORE the saved container is dropped. The saved
    // container (and its `sync_saved_snapshot` record) is the only other place either lives, and it
    // goes the moment the publish is recorded. A vault that enabled Sync under an earlier build has
    // neither (NULL): a bootstrap that needs them for its genesis check says so by name rather than
    // trusting the service's copy. Both or neither: a digest without an id, or the reverse, is
    // refused.
    Migration {
        version: 73,
        name: "sync_genesis_snapshot_identity",
        up: "ALTER TABLE operation_log ADD COLUMN issued_at INTEGER;
             ALTER TABLE sync_genesis ADD COLUMN genesis_snapshot_id BLOB
                 CHECK (genesis_snapshot_id IS NULL OR length(genesis_snapshot_id) = 16);
             ALTER TABLE sync_genesis ADD COLUMN genesis_container_sha256 BLOB
                 CHECK (genesis_container_sha256 IS NULL
                        OR (length(genesis_container_sha256) = 32
                            AND genesis_snapshot_id IS NOT NULL));
             CREATE TRIGGER sync_genesis_snapshot_identity_whole
             BEFORE UPDATE OF genesis_snapshot_id ON sync_genesis
             WHEN NEW.genesis_snapshot_id IS NOT NULL
                  AND NEW.genesis_container_sha256 IS NULL
                  AND OLD.genesis_container_sha256 IS NULL
             BEGIN
                 SELECT RAISE(ABORT, 'a genesis snapshot id is recorded with its container digest');
             END;",
        down: Some(
            "DROP TRIGGER sync_genesis_snapshot_identity_whole;
             ALTER TABLE sync_genesis DROP COLUMN genesis_container_sha256;
             ALTER TABLE sync_genesis DROP COLUMN genesis_snapshot_id;
             ALTER TABLE operation_log DROP COLUMN issued_at;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Version 74 (personal-cfo-1cltt.7, slice D1; provisional: it moves if another migration lands first).
    // `sync_live_closure` is where a replica holds **the live closure** (C.9): for the epoch in
    // force, each `sync_artifact_id` that a snapshot at the vault's accepted log position would
    // carry, with the `ciphertext_sha256` that snapshot binds to it and the logical attachment or
    // batch it belongs to. The materializer and the stage-9 verdicts judge an envelope's delta against
    // it (never against the `sync_artifacts` cache). It is derived, class 3 (outside every write set,
    // row digest, snapshot and portable export), and **written only where the vault's class-1 state
    // equals the accepted state**: by an accepted envelope's apply, the seed at bootstrap and the
    // rebuild at a key epoch, never by a local command (a prune run beside an unpushed delete would
    // drop an object the accepted log still holds live). The key is `(key_epoch, sync_artifact_id)`,
    // so an id carries one hash; the index serves the lookups by logical object.
    Migration {
        version: 74,
        name: "sync_live_closure",
        up: "CREATE TABLE sync_live_closure (
                 key_epoch         INTEGER NOT NULL CHECK (key_epoch >= 1),
                 sync_artifact_id  BLOB    NOT NULL CHECK (length(sync_artifact_id) = 32),
                 ciphertext_sha256 BLOB    NOT NULL CHECK (length(ciphertext_sha256) = 32),
                 logical_kind      TEXT    NOT NULL CHECK (logical_kind IN ('attachment', 'batch')),
                 logical_id        BLOB    NOT NULL CHECK (length(logical_id) = 16),
                 PRIMARY KEY (key_epoch, sync_artifact_id)
             ) WITHOUT ROWID;
             CREATE INDEX sync_live_closure_logical
                 ON sync_live_closure (key_epoch, logical_kind, logical_id);",
        down: Some(
            "DROP INDEX sync_live_closure_logical;
             DROP TABLE sync_live_closure;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Version 75 (personal-cfo-0g528.2, AC9; provisional: it moves if another migration lands first).
    // Two attachments may share one blob (ADR 0023 addendum, vault-format C.9): the same bytes
    // attached on two devices are two attachment ids and one blob, because the blob's name (the
    // StorageId) is a keyed hash of the bytes. `attachments.storage_id` stops being UNIQUE, which
    // SQLite can only do by rebuilding the table (the constraint was written inline in version 5),
    // and the staging index on `sync_artifacts.staged_storage_id` becomes non-unique for the same
    // reason. The lookup by storage id keeps an index. Nothing references `attachments` (so no
    // foreign-key pause), and the columns do not change. Down refuses, atomically and typed (like 57
    // and 59), while any blob name is shared across the rows and stagings together, because version
    // 74's code assumes the two are disjoint; it never drops a row.
    Migration {
        version: 75,
        name: "attachments_share_a_blob",
        up: "CREATE TABLE attachments_v75 (
                 id                  BLOB    PRIMARY KEY,
                 storage_id          TEXT    NOT NULL,
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
             INSERT INTO attachments_v75 (id, storage_id, wrapped_content_key, content_key_nonce,
                     content_nonce, content_alg, plaintext_size, mime_type, original_filename,
                     ref_count, created_at)
                 SELECT id, storage_id, wrapped_content_key, content_key_nonce, content_nonce,
                        content_alg, plaintext_size, mime_type, original_filename, ref_count,
                        created_at
                   FROM attachments ORDER BY rowid;
             DROP TABLE attachments;
             ALTER TABLE attachments_v75 RENAME TO attachments;
             CREATE INDEX idx_attachments_storage_id ON attachments (storage_id);
             DROP INDEX idx_sync_artifacts_staged_storage;
             CREATE INDEX idx_sync_artifacts_staged_storage
                 ON sync_artifacts (staged_storage_id) WHERE staged_storage_id IS NOT NULL;",
        down: Some(
            "CREATE TEMP TABLE attachments_down_refuses_a_shared_blob (ok INTEGER NOT NULL CHECK (ok = 1));
             INSERT INTO attachments_down_refuses_a_shared_blob
                 SELECT CASE WHEN
                        EXISTS (SELECT 1 FROM attachments GROUP BY storage_id HAVING COUNT(*) > 1)
                     OR EXISTS (SELECT 1 FROM sync_artifacts WHERE staged_storage_id IS NOT NULL
                                 GROUP BY staged_storage_id HAVING COUNT(*) > 1)
                     OR EXISTS (SELECT 1 FROM attachments a
                                  JOIN sync_artifacts s ON s.staged_storage_id = a.storage_id)
                     THEN 0 ELSE 1 END;
             DROP TABLE attachments_down_refuses_a_shared_blob;
             DROP INDEX idx_sync_artifacts_staged_storage;
             CREATE UNIQUE INDEX idx_sync_artifacts_staged_storage
                 ON sync_artifacts (staged_storage_id) WHERE staged_storage_id IS NOT NULL;
             CREATE TABLE attachments_v5 (
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
             INSERT INTO attachments_v5 (id, storage_id, wrapped_content_key, content_key_nonce,
                     content_nonce, content_alg, plaintext_size, mime_type, original_filename,
                     ref_count, created_at)
                 SELECT id, storage_id, wrapped_content_key, content_key_nonce, content_nonce,
                        content_alg, plaintext_size, mime_type, original_filename, ref_count,
                        created_at
                   FROM attachments ORDER BY rowid;
             DROP INDEX idx_attachments_storage_id;
             DROP TABLE attachments;
             ALTER TABLE attachments_v5 RENAME TO attachments;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Version 76 (personal-cfo-0g528.3, S3b part 1; provisional: it moves if another migration lands
    // first). The class-1 tables of ADR 0074 Addendum J: `import_identities` maps a committed row's
    // cross-device identity (a provider transaction's derived id, or a file row's) to the transaction
    // committed for it, with a snapshot to compare against; `import_dispositions` holds the decisions
    // about an identity or a file (skip, import anyway, a provider revision, the restore question).
    // Neither names a class-3 row. The migration REFUSES, before it writes anything, on a vault where
    // Sync is armed (a `sync_base` or `sync_genesis` row): the backfill below writes class-1 rows
    // outside a command, which no other device would receive. Its post-step backfills the identities of
    // what the vault imported before (`import_identity_backfill`). Down refuses while either table
    // holds a row, so it never drops one.
    Migration {
        version: 76,
        name: "import_identities_and_dispositions",
        up: "CREATE TABLE import_identities (
                 identity_id          BLOB    PRIMARY KEY CHECK (length(identity_id) = 16),
                 kind                 TEXT    NOT NULL CHECK (kind IN ('provider', 'file')),
                 connection_id        BLOB    CHECK (connection_id IS NULL OR length(connection_id) = 16),
                 provider_account_key TEXT,
                 provider_txn_id      TEXT,
                 file_fingerprint     BLOB    CHECK (file_fingerprint IS NULL OR length(file_fingerprint) = 32),
                 row_ordinal          INTEGER CHECK (row_ordinal IS NULL OR row_ordinal >= 1),
                 transaction_id       BLOB    NOT NULL REFERENCES ledger_transactions(id),
                 account_id           BLOB    NOT NULL,
                 amount_minor         INTEGER NOT NULL,
                 currency             TEXT    NOT NULL,
                 posted_date          TEXT    NOT NULL,
                 provider_description TEXT,
                 created_at           TEXT    NOT NULL,
                 CHECK (
                     (kind = 'provider'
                        AND connection_id IS NOT NULL AND provider_account_key IS NOT NULL
                        AND provider_txn_id IS NOT NULL
                        AND file_fingerprint IS NULL AND row_ordinal IS NULL)
                  OR (kind = 'file'
                        AND file_fingerprint IS NOT NULL AND row_ordinal IS NOT NULL
                        AND connection_id IS NULL AND provider_account_key IS NULL
                        AND provider_txn_id IS NULL AND provider_description IS NULL)),
                 UNIQUE (connection_id, provider_account_key, provider_txn_id),
                 UNIQUE (file_fingerprint, row_ordinal)
             );
             CREATE INDEX idx_import_identities_transaction ON import_identities (transaction_id);
             CREATE TABLE import_dispositions (
                 disposition_id BLOB    PRIMARY KEY CHECK (length(disposition_id) = 16),
                 subject_id     BLOB    NOT NULL CHECK (length(subject_id) = 16),
                 kind           TEXT    NOT NULL CHECK (kind IN (
                     'skipped', 'import_anyway', 'provider_revision', 'file_row_conflict',
                     'restore_question')),
                 status         TEXT    NOT NULL CHECK (status IN ('pending', 'resolved')),
                 generation     INTEGER NOT NULL DEFAULT 1 CHECK (generation >= 1),
                 outcome        TEXT    CHECK (outcome IN ('restored', 'dismissed', 'import_anyway')),
                 claimed_source_batch_id BLOB CHECK (length(claimed_source_batch_id) = 16),
                 detail         TEXT,
                 created_at     TEXT    NOT NULL,
                 decided_at     TEXT,
                 CHECK ((outcome IS NOT NULL) = (kind = 'restore_question' AND status = 'resolved')),
                 CHECK ((claimed_source_batch_id IS NOT NULL) = (outcome IS 'import_anyway'))
             );
             CREATE INDEX idx_import_dispositions_subject ON import_dispositions (subject_id);",
        down: Some(
            "CREATE TEMP TABLE import_identities_down_refuses_a_row (ok INTEGER NOT NULL CHECK (ok = 1));
             INSERT INTO import_identities_down_refuses_a_row
                 SELECT CASE WHEN EXISTS (SELECT 1 FROM import_identities)
                              OR EXISTS (SELECT 1 FROM import_dispositions)
                        THEN 0 ELSE 1 END;
             DROP TABLE import_identities_down_refuses_a_row;
             DROP TABLE import_dispositions;
             DROP TABLE import_identities;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
    // Version 77 (personal-cfo-0g528.3, S3b part 2; provisional: it moves if another migration lands
    // first). `dedupe_decisions` becomes a class-1 table (ADR 0074 Addendum J item 6), keyed to what the
    // decision is about instead of to a device-local staging row: `transaction_id` (the committed
    // ledger transaction) and `identity_id` (its import identity), both nullable because a decision
    // from before this migration may have neither. `staged_transaction_id` and the `flagged` decision
    // leave the table: a *flagged* decision is advisory state about a staging row, so it moves to two
    // class-3 columns on that row (`flag_reason`, `flag_matched_transaction_id`), which never
    // replicate. The old table is renamed and converted row by row by `dedupe_reshape` (the post-step),
    // which then drops it. The migration REFUSES on a vault where Sync is armed, like 76: the
    // conversion writes class-1 rows outside a command. Down rebuilds the old shape: a flagged row
    // returns from its staged row (its decision id is the staged row's id), a committed row returns
    // with the staged row that committed its transaction, when there is one.
    Migration {
        version: 77,
        name: "dedupe_decisions_keyed_to_transactions",
        up: "DROP INDEX idx_dedupe_decisions_batch;
             DROP INDEX idx_dedupe_decisions_staged;
             ALTER TABLE dedupe_decisions RENAME TO dedupe_decisions_legacy;
             ALTER TABLE staged_transactions ADD COLUMN flag_reason TEXT;
             ALTER TABLE staged_transactions ADD COLUMN flag_matched_transaction_id BLOB
                 CHECK (flag_matched_transaction_id IS NULL
                        OR length(flag_matched_transaction_id) = 16);
             CREATE TABLE dedupe_decisions (
                 id                  BLOB    PRIMARY KEY,
                 source_batch_id     BLOB    NOT NULL REFERENCES source_batches(id),
                 layer               TEXT    NOT NULL CHECK (layer IN ('file', 'transaction')),
                 identity_id         BLOB    REFERENCES import_identities(identity_id),
                 transaction_id      BLOB    REFERENCES ledger_transactions(id),
                 matched_entity_type TEXT,
                 matched_entity_id   BLOB,
                 decision            TEXT    NOT NULL CHECK (decision IN (
                     'committed', 'skipped', 'merged')),
                 reason              TEXT    NOT NULL,
                 decided_at          TEXT    NOT NULL
             );
             CREATE INDEX idx_dedupe_decisions_batch ON dedupe_decisions (source_batch_id);
             CREATE INDEX idx_dedupe_decisions_transaction ON dedupe_decisions (transaction_id);
             CREATE INDEX idx_dedupe_decisions_identity ON dedupe_decisions (identity_id);",
        down: Some(
            "CREATE TABLE dedupe_decisions_old (
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
             INSERT INTO dedupe_decisions_old
                 (id, source_batch_id, layer, staged_transaction_id, matched_entity_type,
                  matched_entity_id, decision, reason, decided_at)
             SELECT dd.id, dd.source_batch_id, dd.layer,
                    (SELECT st.id FROM staged_transactions st
                      WHERE st.committed_transaction_id = dd.transaction_id
                      ORDER BY st.created_at, st.id LIMIT 1),
                    dd.matched_entity_type, dd.matched_entity_id, dd.decision, dd.reason, dd.decided_at
               FROM dedupe_decisions dd;
             INSERT INTO dedupe_decisions_old
                 (id, source_batch_id, layer, staged_transaction_id, matched_entity_type,
                  matched_entity_id, decision, reason, decided_at)
             SELECT st.id, sr.source_batch_id, 'transaction', st.id,
                    CASE WHEN st.flag_matched_transaction_id IS NULL
                         THEN NULL ELSE 'ledger_transaction' END,
                    st.flag_matched_transaction_id, 'flagged', st.flag_reason, st.created_at
               FROM staged_transactions st
               JOIN source_records sr ON sr.id = st.source_record_id
              WHERE st.flag_reason IS NOT NULL;
             DROP INDEX idx_dedupe_decisions_batch;
             DROP INDEX idx_dedupe_decisions_transaction;
             DROP INDEX idx_dedupe_decisions_identity;
             DROP TABLE dedupe_decisions;
             ALTER TABLE dedupe_decisions_old RENAME TO dedupe_decisions;
             CREATE INDEX idx_dedupe_decisions_batch ON dedupe_decisions (source_batch_id);
             CREATE INDEX idx_dedupe_decisions_staged ON dedupe_decisions (staged_transaction_id);
             ALTER TABLE staged_transactions DROP COLUMN flag_matched_transaction_id;
             ALTER TABLE staged_transactions DROP COLUMN flag_reason;",
        ),
        rebuilds_read_models: false,
        foreign_keys_off: false,
    },
];

/// The migrations that write class-1 rows outside a command (a backfill): `(version, what it writes)`.
/// This is the ONE place that says so, and two rules read it (ADR 0074 Addendum J item 7):
///
/// - **An armed vault refuses it.** A write to class-1 rows outside a command is in no log row and no
///   envelope, so no other device receives it (`refuse_on_an_armed_vault`, run before the step's SQL
///   in the step's own transaction).
/// - **A replica refuses a snapshot below it.** A replica installs the snapshot's rows and then
///   migrates them, so such a migration would run over rows it was not written for and write
///   class-1 rows no log row explains. The bootstrap checks [`first_row_writing_migration_after`]
///   before it creates anything.
///
/// A later migration that backfills class-1 rows adds its row here, and the rule is standing: once
/// Sync is user-reachable a backfill of class-1 rows is a command with a materialized payload.
pub(crate) const CLASS_1_WRITING_MIGRATIONS: &[(i64, &str)] = &[
    (76, "the import identity backfill"),
    (77, "the dedupe decision conversion"),
];

/// The first marked migration above `schema` and at or below this build's schema, if any.
pub(crate) fn first_row_writing_migration_after(schema: i64) -> Option<i64> {
    CLASS_1_WRITING_MIGRATIONS
        .iter()
        .map(|(version, _)| *version)
        .filter(|version| *version > schema && *version <= CURRENT_VERSION)
        .min()
}

fn writes_class_1_rows(version: i64) -> bool {
    CLASS_1_WRITING_MIGRATIONS
        .iter()
        .any(|(v, _)| *v == version)
}

/// Rust steps that run in the same transaction as a migration's SQL, BEFORE it, so a step that refuses
/// leaves nothing written. A marked migration runs the armed-vault refusal. They run only on the way up.
type PreUp = fn(&rusqlite::Connection, i64) -> Result<(), DbError>;

fn refuse_on_an_armed_vault(conn: &rusqlite::Connection, version: i64) -> Result<(), DbError> {
    crate::import_identity_backfill::refuse_on_an_armed_vault(conn, version)
}

/// Rust steps that run in the same transaction as a migration's SQL, after it, for work SQLite
/// cannot do (hashing). Keyed by migration version. They run only on the way up.
type PostUp = fn(&rusqlite::Connection) -> rusqlite::Result<()>;

const POST_UP: &[(i64, PostUp)] = &[
    (61, crate::households::migrate_up),
    (76, crate::import_identity_backfill::migrate_up),
    (77, crate::dedupe_reshape::migrate_up),
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

/// Inspect an already-keyed existing vault without creating tables or changing
/// version stamps. Call this on a read-only connection before opening a writer,
/// and once more on the newly opened writer before its first PRAGMA write.
pub(crate) fn inspect_existing(conn: &Connection) -> Result<(), DbError> {
    let markers = inspect_markers(conn).map_err(|error| match error {
        newer @ DbError::NewerSchema { .. } => newer,
        // Authenticated existing-vault inspection must not expose arbitrary
        // SQLite schema text through a freeform error at the UI boundary.
        _ => DbError::UnsupportedSchema,
    })?;
    let (Some(applied), Some(_)) = (markers.applied, markers.metadata_version) else {
        return Err(DbError::UnsupportedSchema);
    };
    if applied.is_empty() {
        return Err(DbError::UnsupportedSchema);
    }
    for (version, recorded_hash) in &applied {
        let Some(migration) = MIGRATIONS
            .iter()
            .find(|migration| migration.version == *version)
        else {
            return Err(DbError::UnsupportedSchema);
        };
        if recorded_hash != &content_hash(migration.up) {
            return Err(DbError::UnsupportedSchema);
        }
    }
    // Each migration is committed with its tracker row. A contiguous prefix is
    // the only proven interrupted-upgrade layout; a gap is not recoverable here.
    if applied.len() > MIGRATIONS.len()
        || applied
            .iter()
            .zip(MIGRATIONS)
            .any(|((version, _), migration)| *version != migration.version)
    {
        return Err(DbError::UnsupportedSchema);
    }

    crate::read_vault_metadata(conn).map_err(|_| DbError::UnsupportedSchema)?;
    Ok(())
}

struct SchemaMarkers {
    applied: Option<Vec<(i64, String)>>,
    metadata_version: Option<i64>,
}

/// Read all independent version witnesses, including on a not-yet-bootstrapped
/// connection. A missing/corrupt witness never hides credible newer evidence
/// in another witness. No table creation, journal changes or version stamping.
fn inspect_markers(conn: &Connection) -> Result<SchemaMarkers, DbError> {
    let user_version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let table_exists = |name: &str| -> Result<bool, DbError> {
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [name],
            |row| row.get(0),
        )?)
    };
    let applied = if table_exists("schema_migrations")? {
        applied_versions(conn)
            .map(Some)
            .map_err(|_| DbError::UnsupportedSchema)
    } else {
        Ok(None)
    };
    let metadata_version = if table_exists("vault_metadata")? {
        conn.query_row(
            "SELECT schema_version FROM vault_metadata WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|_| DbError::UnsupportedSchema)
    } else {
        Ok(None)
    };
    let observed = std::iter::once(user_version)
        .chain(
            applied
                .as_ref()
                .ok()
                .and_then(Option::as_ref)
                .into_iter()
                .flatten()
                .map(|(v, _)| *v),
        )
        .chain(metadata_version.as_ref().ok().copied().flatten())
        .max()
        .unwrap_or(user_version);
    if observed > CURRENT_VERSION {
        return Err(DbError::NewerSchema {
            observed,
            supported: CURRENT_VERSION,
        });
    }
    let applied = applied?;
    let metadata_version = metadata_version?;
    if user_version < 0 || metadata_version.is_some_and(|version| version < 0) {
        return Err(DbError::UnsupportedSchema);
    }
    Ok(SchemaMarkers {
        applied,
        metadata_version,
    })
}

/// Apply all pending migrations to `conn` using the canonical [`MIGRATIONS`] set.
/// Returns the number applied. Runs at vault open, before the worker is usable.
pub(crate) fn run_migrations(conn: &mut Connection) -> Result<u64, DbError> {
    apply(conn, MIGRATIONS)
}

/// Apply the migrations up to and including `target` and stamp `PRAGMA user_version` with
/// `target`, for a new vault that must be at a snapshot's schema version before the snapshot is
/// installed (`personal-cfo-1cltt.5`); [`run_migrations`] then carries it the rest of the way, over
/// the installed rows, as it carries any older vault.
///
/// # Errors
/// [`DbError::UnsupportedSchema`] when `target` is not a migration's version (or 0), otherwise as
/// [`apply`].
pub(crate) fn run_migrations_to(conn: &mut Connection, target: i64) -> Result<u64, DbError> {
    if !MIGRATIONS.iter().any(|m| m.version == target) {
        return Err(DbError::UnsupportedSchema);
    }
    let through = MIGRATIONS.partition_point(|m| m.version <= target);
    apply_to(conn, &MIGRATIONS[..through], target)
}

/// Apply pending migrations from `migrations` (split out so tests can drive a
/// synthetic set). Each migration commits in its own transaction; a tracker row
/// records its content hash. If any applied migration rebuilds read models, the
/// transaction-display projection is rebuilt afterward.
pub(crate) fn apply(conn: &mut Connection, migrations: &[Migration]) -> Result<u64, DbError> {
    apply_to(conn, migrations, CURRENT_VERSION)
}

/// [`apply`], stamping `PRAGMA user_version` with `stamp` instead of the build's version.
fn apply_to(conn: &mut Connection, migrations: &[Migration], stamp: i64) -> Result<u64, DbError> {
    // Defensive last line of defense for callers that bypass DbWorker::open.
    // In particular, never let an unknown future row reach ensure_tracker or
    // the unconditional user_version stamp below.
    inspect_markers(conn)?;
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
        let pre: Option<PreUp> = writes_class_1_rows(m.version).then_some(refuse_on_an_armed_vault);
        run_step(conn, m, m.up, pre, |tx| {
            for (version, step) in POST_UP {
                if *version == m.version {
                    step(tx)?;
                }
            }
            tx.execute(
                "INSERT INTO schema_migrations (version, name, content_hash, applied_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    m.version,
                    m.name,
                    content_hash(m.up),
                    Utc::now().to_rfc3339()
                ],
            )
        })?;
        count += 1;
        needs_rebuild |= m.rebuilds_read_models;
    }

    conn.pragma_update(None, "user_version", stamp)?;
    if needs_rebuild {
        projection::rebuild(conn)?;
    }
    Ok(count)
}

/// One `PRAGMA foreign_key_check` row: (child table, child rowid, parent
/// table, constraint index).
type ForeignKeyViolation = (String, Option<i64>, String, i64);

/// Every foreign-key violation in the database.
fn foreign_key_violations(
    conn: &Connection,
) -> rusqlite::Result<std::collections::BTreeSet<ForeignKeyViolation>> {
    let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    rows.collect()
}

/// Run one migration step — `sql` plus its tracker update — in its own
/// transaction. A step flagged [`Migration::foreign_keys_off`] follows SQLite's
/// documented table-rebuild procedure: enforcement off before `BEGIN` (the
/// pragma is a no-op inside a transaction), no NEW foreign-key violation
/// allowed before `COMMIT`, and the prior setting restored whether the step
/// committed or rolled back.
///
/// "New" is judged against a `PRAGMA foreign_key_check` taken before the
/// step: a violation some earlier bug left anywhere in the vault must not
/// stop the vault from opening — only one this step introduced may. Rows are
/// matched by child table + rowid, so a step must not rebuild a table that
/// is itself a CHILD with pre-existing violations (its rowids renumber);
/// rebuilding a parent, as v53 does, leaves every child rowid in place.
fn run_step(
    conn: &mut Connection,
    m: &Migration,
    sql: &str,
    pre: Option<PreUp>,
    track: impl FnOnce(&rusqlite::Transaction<'_>) -> rusqlite::Result<usize>,
) -> Result<(), DbError> {
    let (restore_foreign_keys, violations_before) = if m.foreign_keys_off {
        let was_on: bool = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
        let before = foreign_key_violations(conn)?;
        conn.pragma_update(None, "foreign_keys", false)?;
        (was_on, before)
    } else {
        (false, std::collections::BTreeSet::new())
    };
    let result = (|| -> Result<(), DbError> {
        let tx = conn.transaction()?;
        if let Some(pre) = pre {
            pre(&tx, m.version)?;
        }
        tx.execute_batch(sql)?;
        if m.foreign_keys_off {
            let introduced = foreign_key_violations(&tx)?
                .difference(&violations_before)
                .count();
            if introduced > 0 {
                // Dropping `tx` rolls the step back.
                return Err(DbError::SelfTestFailed(format!(
                    "migration {} ({}) introduced {introduced} foreign-key violation(s)",
                    m.version, m.name
                )));
            }
        }
        track(&tx)?;
        tx.commit()?;
        Ok(())
    })();
    if restore_foreign_keys {
        conn.pragma_update(None, "foreign_keys", true)?;
    }
    result
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
        run_step(conn, m, down, None, |tx| {
            tx.execute(
                "DELETE FROM schema_migrations WHERE version = ?1",
                [m.version],
            )
        })?;
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
    let mut stmt =
        conn.prepare("SELECT version, content_hash FROM schema_migrations ORDER BY version")?;
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

    /// Every marked migration is a real one, listed once, and a marked migration has a POST step that
    /// writes the rows (the marker and the step name the same version).
    #[test]
    fn the_class_1_writing_marker_names_real_migrations_once() {
        let mut seen = std::collections::BTreeSet::new();
        for (version, what) in super::CLASS_1_WRITING_MIGRATIONS {
            assert!(!what.is_empty());
            assert!(seen.insert(*version), "listed twice: {version}");
            assert!(
                MIGRATIONS.iter().any(|m| m.version == *version),
                "no migration {version}"
            );
            assert!(super::writes_class_1_rows(*version));
            assert!(
                super::POST_UP.iter().any(|(v, _)| v == version),
                "migration {version} is marked as writing class-1 rows but has no POST step"
            );
        }
        assert_eq!(super::first_row_writing_migration_after(0), Some(76));
        assert_eq!(super::first_row_writing_migration_after(75), Some(76));
        assert_eq!(super::first_row_writing_migration_after(76), Some(77));
        assert_eq!(super::first_row_writing_migration_after(77), None);
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

    /// Migration v53 rebuilds `source_batches` to admit the `lunchflow` token
    /// (personal-cfo-r2pow, ADR 0076 decision 8). The round-trip loops cover it
    /// against an empty table; this proves it on real rows: existing batches and
    /// their child records survive, `lunchflow` round-trips, unknown tokens are
    /// still refused, and the down refuses (atomically) while a `lunchflow`
    /// batch exists rather than rewriting its provenance.
    #[test]
    fn v53_admits_lunchflow_and_preserves_existing_batches() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..52]).unwrap(); // schema through v52
        let batch = |conn: &Connection, id: u8, source_type: &str| {
            conn.execute(
                "INSERT INTO source_batches (id, source_type, source_name, status,
                    created_at, updated_at)
                 VALUES (?1, ?2, 'fixture', 'staged', '2026-09-27T00:00:00Z',
                    '2026-09-27T00:00:00Z')",
                rusqlite::params![&[id; 16][..], source_type],
            )
        };
        batch(&conn, 1, "simplefin").unwrap();
        conn.execute(
            "INSERT INTO source_records (id, source_batch_id, source_hash, normalized_json,
                created_at)
             VALUES (?1, ?2, 'sha256:fixture', '{}', '2026-09-27T00:00:00Z')",
            rusqlite::params![&[9u8; 16][..], &[1u8; 16][..]],
        )
        .unwrap();
        assert!(
            batch(&conn, 2, "lunchflow").is_err(),
            "v52 must refuse the token"
        );

        apply(&mut conn, &MIGRATIONS[52..53]).unwrap(); // v53

        let kept: (String, String) = conn
            .query_row(
                "SELECT sb.source_type, sb.status FROM source_records sr
                 JOIN source_batches sb ON sb.id = sr.source_batch_id",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kept, ("simplefin".to_owned(), "staged".to_owned()));
        batch(&conn, 2, "lunchflow").unwrap();
        let token: String = conn
            .query_row(
                "SELECT source_type FROM source_batches WHERE id = ?1",
                rusqlite::params![&[2u8; 16][..]],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(token, "lunchflow");
        assert!(
            batch(&conn, 3, "not-a-token").is_err(),
            "CHECK still enforced"
        );
        let null_type = conn.execute(
            "INSERT INTO source_batches (id, source_type, created_at, updated_at)
             VALUES (?1, NULL, 'x', 'x')",
            rusqlite::params![&[4u8; 16][..]],
        );
        assert!(null_type.is_err(), "NOT NULL kept, no default invented");
        let indexed: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master
                 WHERE type = 'index' AND name = 'idx_source_batches_fingerprint'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(indexed, 1, "fingerprint index recreated");
        assert!(integrity_ok(&conn));
        let enforced: bool = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert!(
            enforced,
            "foreign-key enforcement restored after the rebuild"
        );
        let dangling = conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_some();
        assert!(!dangling, "every child row still resolves");

        // Down refuses while a lunchflow batch exists — and changes nothing.
        assert!(migrate_down(&mut conn, 52).is_err());
        let count: i64 = conn
            .query_row("SELECT count(*) FROM source_batches", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2, "a refused down leaves every batch in place");

        conn.execute(
            "DELETE FROM source_batches WHERE source_type = 'lunchflow'",
            [],
        )
        .unwrap();
        migrate_down(&mut conn, 52).unwrap();
        assert!(batch(&conn, 2, "lunchflow").is_err(), "v52 CHECK restored");
        assert!(integrity_ok(&conn));
    }

    /// The `foreign_keys_off` runner path refuses a step that leaves a child
    /// row dangling: the step rolls back, nothing is recorded, and enforcement
    /// is restored anyway.
    #[test]
    fn foreign_keys_off_step_rolls_back_on_a_dangling_child() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..16]).unwrap(); // source_batches + children
        conn.execute(
            "INSERT INTO source_batches (id, source_type, created_at, updated_at)
             VALUES (?1, 'csv', 'x', 'x')",
            rusqlite::params![&[1u8; 16][..]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO source_records (id, source_batch_id, source_hash, normalized_json,
                created_at)
             VALUES (?1, ?2, 'sha256:x', '{}', 'x')",
            rusqlite::params![&[2u8; 16][..], &[1u8; 16][..]],
        )
        .unwrap();
        let orphaning = [super::Migration {
            version: 9002,
            name: "test_orphaning_rebuild",
            up: "DELETE FROM source_batches;",
            down: None,
            rebuilds_read_models: false,
            foreign_keys_off: true,
        }];

        assert!(apply(&mut conn, &orphaning).is_err());

        let batches: i64 = conn
            .query_row("SELECT count(*) FROM source_batches", [], |r| r.get(0))
            .unwrap();
        assert_eq!(batches, 1, "the orphaning step rolled back");
        let recorded: i64 = conn
            .query_row(
                "SELECT count(*) FROM schema_migrations WHERE version = 9002",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(recorded, 0, "a refused step is not recorded");
        let enforced: bool = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert!(enforced, "enforcement restored after a refused step");
    }

    /// Review F2 (personal-cfo-r2pow): the `foreign_keys_off` check fails only
    /// on violations the step introduced. A violation some earlier bug left
    /// elsewhere in the vault must not stop v53 — and so the vault — from
    /// opening, and it is still there afterwards.
    #[test]
    fn a_pre_existing_unrelated_violation_does_not_block_v53() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..52]).unwrap();
        seed_accounts(&conn, 1);
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        conn.execute(
            "UPDATE accounts SET linked_account_id = ?1",
            rusqlite::params![&[0xEEu8; 16][..]],
        )
        .unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        let before = super::foreign_key_violations(&conn).unwrap();
        assert_eq!(before.len(), 1, "the planted violation exists");

        apply(&mut conn, &MIGRATIONS[52..53]).unwrap();

        assert_eq!(
            super::foreign_key_violations(&conn).unwrap(),
            before,
            "v53 neither fixed nor added a violation"
        );
        let enforced: bool = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert!(enforced);
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

    /// The connection identity / secret split (v60, personal-cfo-07u): existing rows move across
    /// unchanged, the credential never lands in a class-1 column, the old credential bytes do
    /// not survive in the file, and the schema matches the manifest's shape.
    const CANARY: &str = "https://canary-user:canary-pass@bridge.invalid/simplefin";

    fn seed_v59_connection(conn: &Connection) {
        conn.execute(
            "INSERT INTO connector_connections
                (id, adapter_id, credential, display_hint, last_synced_at, last_error,
                 created_at, updated_at)
             VALUES (?1, 'simplefin', ?2, 'Primary bank', '2026-10-20T10:00:00Z',
                     'a past error', '2026-09-01T00:00:00Z', '2026-10-20T10:00:00Z')",
            rusqlite::params![&[7u8; 16][..], CANARY],
        )
        .unwrap();
        for (external, synced) in [("acct-1", Some("2026-10-19")), ("acct-2", None)] {
            conn.execute(
                "INSERT INTO connector_account_links
                    (connection_id, external_id, external_name, account_id, last_synced_on,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?2, NULL, ?3, '2026-09-01T00:00:00Z', '2026-10-19T00:00:00Z')",
                rusqlite::params![&[7u8; 16][..], external, synced],
            )
            .unwrap();
        }
    }

    #[test]
    fn v60_moves_the_credential_and_state_to_the_device_local_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v60.db");
        let mut conn = Connection::open(&path).unwrap();
        apply(&mut conn, &MIGRATIONS[..59]).unwrap(); // schema through v59
        seed_v59_connection(&conn);

        apply(&mut conn, &MIGRATIONS[59..60]).unwrap(); // v60

        // The secret table holds exactly what the connection row held.
        let secret: (String, Option<String>, Option<String>, String) = conn
            .query_row(
                "SELECT credential, last_synced_at, last_error, updated_at
                   FROM connector_connection_secrets WHERE connection_id = ?1",
                rusqlite::params![&[7u8; 16][..]],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            secret,
            (
                CANARY.to_owned(),
                Some("2026-10-20T10:00:00Z".to_owned()),
                Some("a past error".to_owned()),
                "2026-10-20T10:00:00Z".to_owned()
            )
        );
        // The cursors hold only the links that had a watermark.
        let cursors: Vec<(String, String)> = conn
            .prepare("SELECT external_id, last_synced_on FROM connector_link_cursors ORDER BY 1")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            cursors,
            vec![("acct-1".to_owned(), "2026-10-19".to_owned())]
        );
        // The class-1 identity keeps its identity columns and gained the tombstone.
        let identity: (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT adapter_id, display_hint, deleted_at FROM connector_connections",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            identity,
            (
                "simplefin".to_owned(),
                Some("Primary bank".to_owned()),
                None
            )
        );
        // No class-1 column mentions the credential, and the class-1 tables lost the moved columns.
        for (table, gone) in [
            (
                "connector_connections",
                ["credential", "last_synced_at", "last_error"],
            ),
            ("connector_account_links", ["last_synced_on", "", ""]),
        ] {
            let columns: Vec<String> = conn
                .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            for column in gone.iter().filter(|c| !c.is_empty()) {
                assert!(
                    !columns.iter().any(|c| c == column),
                    "{table}.{column} remains"
                );
            }
        }
        for table in ["connector_connections", "connector_account_links"] {
            let dump: String = conn
                .query_row(
                    &format!("SELECT group_concat(quote(t.rowid) || quote(t.*)) FROM {table} t"),
                    [],
                    |r| r.get(0),
                )
                .unwrap_or_default();
            assert!(!dump.contains("canary"), "{table} holds the credential");
        }
        assert!(
            conn.query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
                .is_err(),
            "no foreign-key violation"
        );
        drop(conn);
        // The old credential bytes do not survive in the database file.
        let bytes = std::fs::read(&path).unwrap();
        let occurrences = bytes
            .windows(CANARY.len())
            .filter(|w| *w == CANARY.as_bytes())
            .count();
        // One copy remains: the secret row itself, in the class-3 table.
        assert_eq!(
            occurrences, 1,
            "the credential survives only in the secret row"
        );
    }

    #[test]
    fn v60_down_restores_the_old_shape_and_refuses_when_it_would_lose_or_invent_a_secret() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..59]).unwrap();
        seed_v59_connection(&conn);
        apply(&mut conn, &MIGRATIONS[59..60]).unwrap();

        // An identity with no secret row (it came from another device) refuses the down, atomically.
        conn.execute(
            "INSERT INTO connector_connections (id, adapter_id, created_at, updated_at)
             VALUES (?1, 'simplefin', '2026-10-21T00:00:00Z', '2026-10-21T00:00:00Z')",
            rusqlite::params![&[8u8; 16][..]],
        )
        .unwrap();
        assert!(
            migrate_down(&mut conn, 59).is_err(),
            "must refuse: no credential to restore"
        );
        let still_split: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM connector_connection_secrets",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still_split, 1, "a refused down changes nothing");
        conn.execute(
            "DELETE FROM connector_connections WHERE id = ?1",
            rusqlite::params![&[8u8; 16][..]],
        )
        .unwrap();

        // A tombstone refuses the down (it would be silently resurrected).
        conn.execute(
            "UPDATE connector_connections SET deleted_at = '2026-10-22T00:00:00Z'",
            [],
        )
        .unwrap();
        assert!(
            migrate_down(&mut conn, 59).is_err(),
            "must refuse: a tombstone"
        );
        conn.execute("UPDATE connector_connections SET deleted_at = NULL", [])
            .unwrap();

        // Otherwise the old shape comes back with its data.
        migrate_down(&mut conn, 59).unwrap();
        let restored: (String, String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT adapter_id, credential, last_synced_at, last_error FROM connector_connections",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            restored,
            (
                "simplefin".to_owned(),
                CANARY.to_owned(),
                Some("2026-10-20T10:00:00Z".to_owned()),
                Some("a past error".to_owned())
            )
        );
        let watermark: Option<String> = conn
            .query_row(
                "SELECT last_synced_on FROM connector_account_links WHERE external_id = 'acct-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(watermark.as_deref(), Some("2026-10-19"));
        let secrets_table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'connector_connection_secrets'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(secrets_table, 0);
        // And it composes: up again gives the split back.
        apply(&mut conn, MIGRATIONS).unwrap();
        let again: String = conn
            .query_row(
                "SELECT credential FROM connector_connection_secrets",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(again, CANARY);
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

    /// A vault at schema version 60 with a `vault_metadata` row, as an older build left it.
    fn v60_vault_with_metadata(created_at: &str, timezone: &str) -> (Connection, uuid::Uuid) {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..60]).unwrap();
        let vault_id = uuid::Uuid::from_u128(0x0123_4567_89ab_7def_8123_4567_89ab_cdef);
        conn.execute(
            "INSERT INTO vault_metadata (singleton, vault_id, schema_version, envelope_version,
                kdf_algorithm, kdf_memory_kib, kdf_time_cost, kdf_parallelism,
                household_timezone, manifest_pointer, created_at)
             VALUES (1, ?1, 60, 1, 'argon2id', 65536, 3, 1, ?2, NULL, ?3)",
            rusqlite::params![vault_id, timezone, created_at],
        )
        .unwrap();
        (conn, vault_id)
    }

    /// Every class-1 household row of a vault as comparable text.
    fn household_rows(conn: &Connection) -> Vec<String> {
        let mut rows = Vec::new();
        for sql in [
            "SELECT hex(id) || '|' || name || '|' || household_timezone || '|' ||
                    COALESCE(platform_household_id, '~') || '|' || created_at || '|' || updated_at
               FROM households",
            "SELECT hex(id) || '|' || hex(household_id) || '|' || display_name || '|' || role || '|' ||
                    COALESCE(platform_member_id, '~') || '|' || created_at || '|' ||
                    COALESCE(deleted_at, '~')
               FROM household_members",
            "SELECT hex(member_id) || '|' || COALESCE(initials, '~') FROM profiles",
        ] {
            let mut stmt = conn.prepare(sql).unwrap();
            rows.extend(
                stmt.query_map([], |r| r.get::<_, String>(0))
                    .unwrap()
                    .map(Result::unwrap),
            );
        }
        rows
    }

    /// The migration backfills one household and ONE primary member from the vault's own id and
    /// creation time, moves the timezone, and drops the old column. Two copies of one vault
    /// migrated at different wall-clock times hold byte-identical class-1 rows.
    #[test]
    fn v61_backfills_one_household_and_one_primary_deterministically() {
        let created = "2026-03-01T09:30:00+00:00";
        let (mut a, vault_id) = v60_vault_with_metadata(created, "America/Denver");
        let (mut b, _) = v60_vault_with_metadata(created, "America/Denver");
        apply(&mut a, &MIGRATIONS[60..61]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        apply(&mut b, &MIGRATIONS[60..61]).unwrap();

        let rows = household_rows(&a);
        assert_eq!(
            rows,
            household_rows(&b),
            "independent migrations agree on every column"
        );
        assert_eq!(rows.len(), 2, "one household and one member: {rows:?}");
        let household = crate::households::household_id(&vault_id);
        let member = crate::households::primary_member_id(&vault_id);
        assert_eq!(
            rows[0],
            format!(
                "{}|My household|America/Denver|~|{created}|{created}",
                hex_upper(household.as_bytes())
            )
        );
        assert_eq!(
            rows[1],
            format!(
                "{}|{}|Me|primary|~|{created}|~",
                hex_upper(member.as_bytes()),
                hex_upper(household.as_bytes())
            )
        );
        let old_column: i64 = a
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('vault_metadata') WHERE name = 'household_timezone'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_column, 0, "the timezone moved");
    }

    /// The down migration restores the column from the household row in the backfill state and
    /// refuses (atomically) in every state that dropping the tables would lose data from.
    #[test]
    fn v61_down_restores_the_timezone_and_refuses_when_it_would_lose_data() {
        let created = "2026-03-01T09:30:00+00:00";
        let (mut conn, vault_id) = v60_vault_with_metadata(created, "Asia/Tokyo");
        apply(&mut conn, &MIGRATIONS[60..61]).unwrap();
        let member = crate::households::primary_member_id(&vault_id);
        let household = crate::households::household_id(&vault_id);

        // Each of these is data the down would lose, so it refuses and changes nothing.
        let edits: [(&str, &str); 6] = [
            (
                "a renamed household",
                "UPDATE households SET name = 'Garcias'",
            ),
            (
                "a renamed primary",
                "UPDATE household_members SET display_name = 'Pat'",
            ),
            (
                "a platform household link",
                "UPDATE households SET platform_household_id = 'h-1'",
            ),
            (
                "a platform member link",
                "UPDATE household_members SET platform_member_id = 'm-1'",
            ),
            (
                "a second member",
                "INSERT INTO household_members (id, household_id, display_name, role, created_at)
                 VALUES (x'00000000000000000000000000000001', (SELECT id FROM households),
                         'Sam', 'partner', '2026-03-02T00:00:00+00:00')",
            ),
            (
                "a profile",
                "INSERT INTO profiles (member_id, initials, color, updated_at)
                 SELECT id, 'ME', NULL, '2026-03-02T00:00:00+00:00' FROM household_members",
            ),
        ];
        for (label, edit) in edits {
            let tx = conn.unchecked_transaction().unwrap();
            tx.execute_batch(edit).unwrap();
            tx.commit().unwrap();
            let rows_before = household_rows(&conn);
            assert!(
                migrate_down(&mut conn, 60).is_err(),
                "down must refuse with {label}"
            );
            assert_eq!(
                user_version_of(&conn),
                CURRENT_VERSION,
                "{label}: the refused down changed nothing"
            );
            assert_eq!(
                household_rows(&conn),
                rows_before,
                "{label}: nothing changed"
            );
            // Undo the edit for the next case.
            let undo = match label {
                "a renamed household" => "UPDATE households SET name = 'My household'",
                "a renamed primary" => "UPDATE household_members SET display_name = 'Me'",
                "a platform household link" => "UPDATE households SET platform_household_id = NULL",
                "a platform member link" => {
                    "UPDATE household_members SET platform_member_id = NULL"
                }
                "a second member" => "DELETE FROM household_members WHERE role = 'partner'",
                _ => "DELETE FROM profiles",
            };
            conn.execute_batch(undo).unwrap();
        }
        let _ = (member, household);

        // The pristine backfill state goes back, with the timezone on the old column.
        assert_eq!(migrate_down(&mut conn, 60).unwrap(), 1);
        let timezone: String = conn
            .query_row("SELECT household_timezone FROM vault_metadata", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(timezone, "Asia/Tokyo");
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('households', 'household_members', 'profiles')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
        // And up again yields the same rows.
        apply(&mut conn, MIGRATIONS).unwrap();
        assert_eq!(household_rows(&conn).len(), 2);
    }

    /// The five legacy household names, in the order the migration renames them.
    const LEGACY_SETTING_KEYS: [&str; 5] = [
        "reporting_currency",
        "minimum_cash_floor_minor",
        "comfort_band_upper_minor",
        "auto_categorize_on_import",
        "future_cash_series_selection",
    ];

    fn setting_rows(conn: &Connection) -> Vec<(String, String, String)> {
        conn.prepare("SELECT key, value, updated_at FROM settings ORDER BY key")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn plant_setting(conn: &Connection, key: &str, value: &str, updated_at: &str) {
        conn.execute(
            "INSERT INTO settings (key, value, value_schema_version, updated_at)
             VALUES (?1, ?2, 1, ?3)",
            [key, value, updated_at],
        )
        .unwrap();
    }

    /// 64 renames the five legacy names to `user.<name>` once, keeping value and `updated_at`, and
    /// leaves every other row alone: a `device.*` key, and `locale`, which no build ever wrote.
    #[test]
    fn v64_renames_the_five_household_keys_and_nothing_else() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..63]).unwrap();
        for (n, key) in LEGACY_SETTING_KEYS.iter().enumerate() {
            plant_setting(&conn, key, &format!("v{n}"), &format!("t{n}"));
        }
        plant_setting(&conn, "device.window", "1", "td");
        plant_setting(&conn, "locale", "en-US", "tl");

        apply(&mut conn, &MIGRATIONS[63..64]).unwrap();

        let mut expected: Vec<(String, String, String)> = LEGACY_SETTING_KEYS
            .iter()
            .enumerate()
            .map(|(n, key)| (format!("user.{key}"), format!("v{n}"), format!("t{n}")))
            .collect();
        expected.push(("device.window".into(), "1".into(), "td".into()));
        expected.push(("locale".into(), "en-US".into(), "tl".into()));
        expected.sort();
        assert_eq!(setting_rows(&conn), expected);
    }

    /// A prefixed row that already exists wins, and the legacy row goes: the DELETE runs before the
    /// UPDATE, so the rename does not hit the primary key and abort.
    #[test]
    fn v64_keeps_an_existing_prefixed_row_and_drops_the_legacy_one() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..63]).unwrap();
        plant_setting(&conn, "minimum_cash_floor_minor", "100", "old");
        plant_setting(&conn, "user.minimum_cash_floor_minor", "200", "new");
        plant_setting(&conn, "reporting_currency", "EUR", "t");

        apply(&mut conn, &MIGRATIONS[63..64]).unwrap();

        assert_eq!(
            setting_rows(&conn),
            vec![
                (
                    "user.minimum_cash_floor_minor".into(),
                    "200".into(),
                    "new".into()
                ),
                ("user.reporting_currency".into(), "EUR".into(), "t".into()),
            ]
        );
    }

    /// Running the migration's SQL again changes nothing, and a fresh vault has no settings rows.
    #[test]
    fn v64_is_idempotent_and_adds_nothing_to_a_fresh_vault() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();
        assert!(setting_rows(&conn).is_empty());
        plant_setting(&conn, "user.reporting_currency", "USD", "t");
        conn.execute_batch(MIGRATIONS[63].up).unwrap();
        assert_eq!(
            setting_rows(&conn),
            vec![("user.reporting_currency".into(), "USD".into(), "t".into())]
        );
    }

    /// Down returns the five names to their unprefixed form, and up renames them again.
    #[test]
    fn v64_down_restores_the_unprefixed_names_and_up_renames_them_again() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();
        for (n, key) in LEGACY_SETTING_KEYS.iter().enumerate() {
            plant_setting(&conn, &format!("user.{key}"), &format!("v{n}"), "t");
        }
        plant_setting(&conn, "device.window", "1", "t");

        // Every migration above 63 is rolled back (64 itself, and any that follow it).
        assert_eq!(
            migrate_down(&mut conn, 63).unwrap(),
            u64::try_from(CURRENT_VERSION - 63).unwrap()
        );
        let down: Vec<String> = setting_rows(&conn).into_iter().map(|r| r.0).collect();
        let mut want: Vec<String> = LEGACY_SETTING_KEYS
            .iter()
            .map(|k| (*k).to_owned())
            .collect();
        want.push("device.window".into());
        want.sort();
        assert_eq!(down, want);

        apply(&mut conn, MIGRATIONS).unwrap();
        let up: Vec<String> = setting_rows(&conn).into_iter().map(|r| r.0).collect();
        assert!(up.contains(&"user.reporting_currency".to_owned()));
        assert!(!up.contains(&"reporting_currency".to_owned()));
    }

    /// A staged row under `batch` (a record of its own), inserted in call order.
    fn plant_staged(conn: &Connection, batch: u8, n: u8) -> u8 {
        let record = [batch, n];
        conn.execute(
            "INSERT OR IGNORE INTO source_batches
                (id, source_type, status, created_at, updated_at)
             VALUES (?1, 'csv', 'staged', 't', 't')",
            [vec![batch]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO source_records
                (id, source_batch_id, source_hash, normalized_json, created_at)
             VALUES (?1, ?2, ?3, '{}', 't')",
            rusqlite::params![record.to_vec(), vec![batch], format!("h{batch}{n}")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO staged_transactions
                (id, source_record_id, posted_at, amount_minor, currency, txn_fingerprint,
                 created_at)
             VALUES (?1, ?1, '2026-06-01', -100, 'USD', ?2, 't')",
            rusqlite::params![record.to_vec(), format!("fp{batch}{n}")],
        )
        .unwrap();
        n
    }

    /// `(batch, n, row_ordinal)` for every staged row.
    fn ordinals(conn: &Connection) -> Vec<(u8, u8, Option<i64>)> {
        conn.prepare("SELECT id, row_ordinal FROM staged_transactions ORDER BY rowid")
            .unwrap()
            .query_map([], |r| {
                let id: Vec<u8> = r.get(0)?;
                Ok((id[0], id[1], r.get(1)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// 65 numbers the rows staged before it 1.. within their batch, in insertion order, whatever
    /// order the batches interleave in.
    #[test]
    fn v65_numbers_existing_staged_rows_per_batch_in_insertion_order() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..64]).unwrap();
        for (batch, n) in [(1, 1), (2, 1), (1, 2), (1, 3), (2, 2)] {
            plant_staged(&conn, batch, n);
        }

        apply(&mut conn, &MIGRATIONS[64..65]).unwrap();

        assert_eq!(
            ordinals(&conn),
            vec![
                (1, 1, Some(1)),
                (2, 1, Some(1)),
                (1, 2, Some(2)),
                (1, 3, Some(3)),
                (2, 2, Some(2)),
            ]
        );
    }

    /// A vault with no staged rows migrates, and down removes the column so up numbers again.
    #[test]
    fn v65_down_drops_the_column_and_up_numbers_the_rows_again() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..65]).unwrap();
        assert!(ordinals(&conn).is_empty());
        plant_staged(&conn, 1, 1);

        assert_eq!(migrate_down(&mut conn, 64).unwrap(), 1);
        let has_column: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('staged_transactions')
                  WHERE name = 'row_ordinal'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_column, 0);

        apply(&mut conn, MIGRATIONS).unwrap();
        assert_eq!(ordinals(&conn), vec![(1, 1, Some(1))]);
    }

    /// 67 adds the outbox's issue time; down drops the column and keeps the rows, and up adds it
    /// back empty (a row from before it, which a rebase refuses by name).
    #[test]
    fn v67_down_drops_the_issue_time_and_keeps_the_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..67]).unwrap();
        conn.execute(
            "INSERT INTO sync_outbox
               (command_id, op_seq, state, write_set_keys, created_at, issued_at)
             VALUES (zeroblob(16), 1, 'Pending', zeroblob(0), '2026-11-06T00:00:00Z', 5)",
            [],
        )
        .unwrap();
        let has_column = |conn: &Connection| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sync_outbox') WHERE name = 'issued_at'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(has_column(&conn), 1);

        assert_eq!(migrate_down(&mut conn, 66).unwrap(), 1);
        assert_eq!(has_column(&conn), 0, "down drops the column");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "down keeps the outbox rows");

        apply(&mut conn, MIGRATIONS).unwrap();
        assert_eq!(has_column(&conn), 1);
        let issued: Option<i64> = conn
            .query_row("SELECT issued_at FROM sync_outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            issued, None,
            "a row from before the column has no issue time"
        );
    }

    /// 68 adds the counter register, the saved snapshot record and the persisted genesis; down
    /// drops the three, and up creates them again empty.
    #[test]
    fn v68_down_drops_the_register_and_genesis_tables_and_up_creates_them_empty() {
        let tables = |conn: &Connection| -> Vec<String> {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type = 'table' AND name IN
                     ('sync_counter_register', 'sync_saved_snapshot', 'sync_genesis')
                     ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..68]).unwrap();
        assert_eq!(
            tables(&conn),
            [
                "sync_counter_register",
                "sync_genesis",
                "sync_saved_snapshot"
            ]
        );

        assert_eq!(migrate_down(&mut conn, 67).unwrap(), 1);
        assert!(tables(&conn).is_empty(), "down drops all three");

        apply(&mut conn, MIGRATIONS).unwrap();
        assert_eq!(tables(&conn).len(), 3);
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_genesis", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    /// 69 adds the void ledger and the applied cursor; down drops both with their triggers, and up
    /// creates them again empty. Each trigger is exercised: the ledger is append-only with a dense
    /// ordinal and ascending ranges, and the cursor only moves forward and keeps its root.
    #[test]
    fn v69_creates_the_ledger_and_the_cursor_with_their_guards_and_down_drops_them() {
        let objects = |conn: &Connection| -> Vec<String> {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master WHERE name LIKE 'sync_voids%'
                     OR name LIKE 'sync_position%' ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();
        assert_eq!(objects(&conn).len(), 8, "two tables and six triggers");

        let insert = |conn: &Connection, id: u8, first: i64, last: i64, ordinal: i64| {
            conn.execute(
                "INSERT INTO sync_voids (void_id, first, last, ordinal, origin)
                 VALUES (?1, ?2, ?3, ?4, 'seeded')",
                rusqlite::params![vec![id; 32], first, last, ordinal],
            )
        };
        insert(&conn, 1, 2, 3, 1).unwrap();
        insert(&conn, 2, 5, 6, 2).unwrap();
        assert!(insert(&conn, 3, 8, 9, 4).is_err(), "a gap in the ordinals");
        assert!(insert(&conn, 3, 8, 9, 2).is_err(), "a repeated ordinal");
        assert!(insert(&conn, 3, 6, 9, 3).is_err(), "overlapping ranges");
        assert!(insert(&conn, 3, 4, 4, 3).is_err(), "a range below the last");
        assert!(
            insert(&conn, 3, 9, 8, 3).is_err(),
            "a range that runs backwards"
        );
        insert(&conn, 1, 7, 9, 3).unwrap();
        assert!(conn
            .execute("UPDATE sync_voids SET origin = 'honored'", [])
            .is_err());
        assert!(conn.execute("DELETE FROM sync_voids", []).is_err());

        conn.execute(
            "INSERT INTO sync_position (singleton, snapshot_root, next_to_apply) VALUES (1, 10, 11)",
            [],
        )
        .unwrap();
        assert!(
            conn.execute(
                "INSERT INTO sync_position (singleton, snapshot_root, next_to_apply)
                 VALUES (1, 10, 11)",
                []
            )
            .is_err(),
            "one row"
        );
        conn.execute("UPDATE sync_position SET next_to_apply = 15", [])
            .unwrap();
        assert!(conn
            .execute("UPDATE sync_position SET next_to_apply = 14", [])
            .is_err());
        assert!(conn
            .execute("UPDATE sync_position SET snapshot_root = 11", [])
            .is_err());
        assert!(conn
            .execute("UPDATE sync_position SET snapshot_root = NULL", [])
            .is_err());
        assert!(conn.execute("DELETE FROM sync_position", []).is_err());
        let origin = conn.execute(
            "UPDATE sync_position SET next_to_apply = 15 WHERE singleton = 1",
            [],
        );
        assert!(origin.is_ok(), "an unchanged cursor is not a move back");

        // 70 to 77 sit above 69: stepping to 68 reverts all nine.
        assert_eq!(migrate_down(&mut conn, 68).unwrap(), 9);
        assert!(
            objects(&conn).is_empty(),
            "down drops both tables and every trigger"
        );
        apply(&mut conn, MIGRATIONS).unwrap();
        assert_eq!(objects(&conn).len(), 8);
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_voids", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    /// 70 adds the device-local artifact mapping; down drops it with its indexes, and up creates it
    /// again empty.
    #[test]
    fn v70_down_drops_the_artifact_mapping_and_up_creates_it_empty() {
        let objects = |conn: &Connection| -> Vec<String> {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master WHERE name LIKE '%sync_artifacts%'
                       AND name NOT LIKE 'sqlite_autoindex%' ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();
        assert_eq!(
            objects(&conn),
            [
                "idx_sync_artifacts_logical",
                "idx_sync_artifacts_staged_storage",
                "sync_artifacts"
            ]
        );
        // 71 to 77 sit above 70: stepping to 69 reverts all eight.
        assert_eq!(migrate_down(&mut conn, 69).unwrap(), 8);
        assert!(objects(&conn).is_empty());
        apply(&mut conn, MIGRATIONS).unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_artifacts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    /// The mapping's CHECKs: an attachment has no part, a part has an index inside its count, and
    /// the staged crypto fields are all present on an attachment or all absent.
    #[test]
    fn the_artifact_mapping_refuses_a_row_that_is_not_one_of_its_two_shapes() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();
        // `n` makes the address unique; `staged` is the five-part crypto set or all NULLs.
        let insert = |n: u8, id_len: usize, kind: i64, part: (&str, &str), staged: &str| {
            conn.execute(
                &format!(
                    "INSERT INTO sync_artifacts
                         (key_epoch, sync_artifact_id, ciphertext_sha256, kind, logical_id,
                          part_index, part_count, staged_storage_id, staged_wrapped_content_key,
                          staged_content_key_nonce, staged_content_nonce, staged_content_alg,
                          staged_plaintext_size)
                     VALUES (1, x'{}', zeroblob(32), {kind}, zeroblob(16), {}, {}, {staged})",
                    format!("{n:02x}").repeat(id_len),
                    part.0,
                    part.1
                ),
                [],
            )
        };
        const NONE: &str = "NULL, NULL, NULL, NULL, NULL, NULL";
        const STAGED: &str = "'sb', x'aa', x'00', x'01', 'AES-256-GCM', 5";

        // The two shapes: an attachment with no part, and a batch part inside its count.
        assert!(insert(1, 32, 1, ("NULL", "NULL"), NONE).is_ok());
        assert!(insert(2, 32, 2, ("0", "3"), NONE).is_ok());
        assert!(insert(3, 32, 2, ("2", "3"), NONE).is_ok(), "the last part");

        // Refused: an attachment with a part, a part at or past its count, a part with no index or
        // no count, a count of zero, a negative index, an unknown kind, a short address.
        for (n, (id_len, kind, part)) in [
            (32, 1, ("0", "1")),
            (32, 2, ("3", "3")),
            (32, 2, ("NULL", "3")),
            (32, 2, ("0", "NULL")),
            (32, 2, ("0", "0")),
            (32, 2, ("-1", "3")),
            (32, 3, ("NULL", "NULL")),
            (31, 1, ("NULL", "NULL")),
        ]
        .into_iter()
        .enumerate()
        {
            assert!(
                insert(10 + n as u8, id_len, kind, part, NONE).is_err(),
                "kind {kind}, part {part:?}, id length {id_len}"
            );
        }

        // Staged crypto fields: all present on an attachment is fine; some missing, or on a batch
        // part, is not; and a staged storage id names one blob.
        assert!(insert(20, 32, 1, ("NULL", "NULL"), STAGED).is_ok());
        assert!(
            insert(
                21,
                32,
                1,
                ("NULL", "NULL"),
                &STAGED.replace("x'aa'", "NULL")
            )
            .is_err(),
            "a storage id with no key"
        );
        assert!(
            insert(22, 32, 1, ("NULL", "NULL"), &STAGED.replace("'sb'", "NULL")).is_err(),
            "a key with no storage id"
        );
        assert!(
            insert(23, 32, 2, ("0", "1"), &STAGED.replace("'sb'", "'sc'")).is_err(),
            "key material on a batch part"
        );
        assert!(
            insert(24, 32, 1, ("NULL", "NULL"), STAGED).is_ok(),
            "two rows may stage one blob (version 75)"
        );
        assert!(
            insert(25, 32, 1, ("NULL", "NULL"), &STAGED.replace("'sb'", "'sd'")).is_ok(),
            "another blob"
        );

        // Clearing the staging on its own is allowed (what the attach command's apply does).
        let cleared = conn
            .execute(
                "UPDATE sync_artifacts SET staged_storage_id = NULL,
                        staged_wrapped_content_key = NULL, staged_content_key_nonce = NULL,
                        staged_content_nonce = NULL, staged_content_alg = NULL,
                        staged_plaintext_size = NULL, staged_mime_type = NULL
                  WHERE staged_storage_id = 'sb'",
                [],
            )
            .unwrap();
        assert_eq!(cleared, 2, "both rows that staged the blob (version 75)");
    }

    /// 71 adds the device-local notice table; down drops it with its index, and up creates it
    /// again empty. Its CHECKs refuse an unknown kind, a short batch id and a negative count.
    #[test]
    fn v71_down_drops_the_notice_table_and_up_creates_it_empty() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();
        let insert = |kind: &str, batch_len: usize, count: i64| {
            conn.execute(
                &format!(
                    "INSERT INTO sync_notices (kind, batch_id, source_name, discarded_count, created_at)
                     VALUES ('{kind}', zeroblob({batch_len}), 'statement.csv', {count}, 't')"
                ),
                [],
            )
        };
        assert!(insert("import_discarded", 16, 3).is_ok());
        assert!(insert("import_discarded", 16, 0).is_ok());
        assert!(insert("something_else", 16, 3).is_err());
        assert!(insert("import_discarded", 15, 3).is_err());
        assert!(insert("import_discarded", 16, -1).is_err());
        // 72 to 77 sit above 71: stepping to 70 reverts all seven.
        assert_eq!(migrate_down(&mut conn, 70).unwrap(), 7);
        let left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE '%sync_notices%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
        apply(&mut conn, MIGRATIONS).unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_notices", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    fn outbox_insert(
        conn: &Connection,
        id: u8,
        state: &str,
        sealed: &str,
        bound_seq: &str,
        result: &str,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            &format!(
                "INSERT INTO sync_outbox
                   (command_id, op_seq, state, write_set_keys, sealed_container, bound_seq,
                    result_ref, created_at, issued_at)
                 VALUES (x'000000000000000000000000000000{id:02x}', {id}, '{state}', zeroblob(0), {sealed},
                         {bound_seq}, {result}, '2026-11-06T00:00:00Z', 5)"
            ),
            [],
        )
    }

    /// 72: a `Bound` row may have no result, or a 1 to 64 byte one, and nothing else.
    #[test]
    fn v72_a_bound_row_has_no_result_or_a_one_to_sixty_four_byte_one() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();

        outbox_insert(&conn, 1, "Bound", "zeroblob(1)", "6", "NULL").unwrap();
        outbox_insert(&conn, 2, "Bound", "zeroblob(1)", "7", "zeroblob(1)").unwrap();
        outbox_insert(&conn, 3, "Bound", "zeroblob(1)", "8", "zeroblob(64)").unwrap();
        assert!(
            outbox_insert(&conn, 4, "Bound", "zeroblob(1)", "9", "zeroblob(0)").is_err(),
            "an empty result is a second encoding of none"
        );
        assert!(
            outbox_insert(&conn, 5, "Bound", "zeroblob(1)", "9", "zeroblob(65)").is_err(),
            "a 65 byte result is over the bound"
        );
        // The other states are unchanged: no result, no position.
        outbox_insert(&conn, 6, "Pending", "NULL", "NULL", "NULL").unwrap();
        outbox_insert(&conn, 7, "Approved", "zeroblob(1)", "NULL", "NULL").unwrap();
        assert!(outbox_insert(&conn, 8, "Pending", "NULL", "NULL", "zeroblob(1)").is_err());
        assert!(outbox_insert(&conn, 9, "Approved", "zeroblob(1)", "NULL", "zeroblob(1)").is_err());
        assert!(outbox_insert(&conn, 10, "Bound", "zeroblob(1)", "NULL", "NULL").is_err());
        assert!(outbox_insert(&conn, 11, "Bound", "NULL", "6", "NULL").is_err());
        let none: Option<Vec<u8>> = conn
            .query_row(
                "SELECT result_ref FROM sync_outbox WHERE op_seq = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(none, None, "a Bound row with no result reads back as None");
    }

    /// Up and down with nothing the old shape cannot hold restores the old shape, and the rows.
    #[test]
    fn v72_down_restores_the_old_shape_and_keeps_the_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();
        outbox_insert(&conn, 1, "Bound", "zeroblob(1)", "6", "zeroblob(2)").unwrap();
        outbox_insert(&conn, 2, "Pending", "NULL", "NULL", "NULL").unwrap();

        // 73 (the genesis identity), 74 (the live closure), 75 (shared blobs) and 76 (import
        // identities) sit above 72: step past them first.
        assert_eq!(migrate_down(&mut conn, 72).unwrap(), 5);
        assert_eq!(migrate_down(&mut conn, 71).unwrap(), 1);

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2, "down keeps the outbox rows");
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('sync_outbox') ORDER BY cid")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            columns,
            [
                "command_id",
                "op_seq",
                "state",
                "write_set_keys",
                "sealed_container",
                "bound_seq",
                "result_ref",
                "created_at",
                "issued_at"
            ]
        );
        // The old check is back: a Bound row needs a result.
        assert!(outbox_insert(&conn, 3, "Bound", "zeroblob(1)", "7", "NULL").is_err());
        outbox_insert(&conn, 4, "Bound", "zeroblob(1)", "7", "zeroblob(1)").unwrap();

        // And up again carries the rows over.
        apply(&mut conn, MIGRATIONS).unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 3);
        outbox_insert(&conn, 5, "Bound", "zeroblob(1)", "8", "NULL").unwrap();
    }

    /// Down refuses, and changes nothing, while a `Bound` row has no result: it never invents one.
    #[test]
    fn v72_down_fails_closed_when_a_bound_row_has_no_result() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();
        outbox_insert(&conn, 1, "Bound", "zeroblob(1)", "6", "NULL").unwrap();
        outbox_insert(&conn, 2, "Pending", "NULL", "NULL", "NULL").unwrap();

        assert_eq!(migrate_down(&mut conn, 72).unwrap(), 5);
        assert!(migrate_down(&mut conn, 71).is_err());

        let applied: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 72",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(applied, 1, "72 is still applied");
        let rows: Vec<(i64, Option<Vec<u8>>)> = conn
            .prepare("SELECT op_seq, result_ref FROM sync_outbox ORDER BY op_seq")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows,
            [(1, None), (2, None)],
            "both rows untouched, no value invented"
        );
        // The new shape still stands.
        outbox_insert(&conn, 3, "Bound", "zeroblob(1)", "7", "NULL").unwrap();
    }

    /// An upgrade carries a row the old shape could only have meant "no result" for, an empty
    /// `result_ref`, over as NULL, and keeps every other row as it was.
    #[test]
    fn v72_up_carries_an_empty_result_over_as_none() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..71]).unwrap();
        outbox_insert(&conn, 1, "Bound", "zeroblob(1)", "6", "zeroblob(0)").unwrap();
        outbox_insert(&conn, 2, "Bound", "zeroblob(1)", "7", "zeroblob(3)").unwrap();

        apply(&mut conn, MIGRATIONS).unwrap();

        let rows: Vec<(i64, Option<Vec<u8>>)> = conn
            .prepare("SELECT op_seq, result_ref FROM sync_outbox ORDER BY op_seq")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, [(1, None), (2, Some(vec![0, 0, 0]))]);
    }

    /// 73 adds the genesis snapshot's identity to `sync_genesis`: two nullable columns, both or
    /// neither, that an existing row keeps as NULL; down drops them and up adds them again.
    #[test]
    fn v73_adds_the_genesis_snapshot_identity_both_or_neither_and_down_drops_it() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..72]).unwrap();
        // A genesis recorded by an earlier build.
        conn.execute(
            "INSERT INTO sync_genesis (singleton, vault_id, membership_record, epoch_record, state,
                                       persisted_at, updated_at)
             VALUES (1, zeroblob(16), x'01', x'02', 'Published', 't', 't')",
            [],
        )
        .unwrap();
        // Through 75: version 76 refuses a vault where Sync is armed, and this one has a genesis row.
        apply(&mut conn, &MIGRATIONS[..75]).unwrap();
        let identity = |conn: &Connection| -> (Option<Vec<u8>>, Option<Vec<u8>>) {
            conn.query_row(
                "SELECT genesis_snapshot_id, genesis_container_sha256 FROM sync_genesis",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(
            identity(&conn),
            (None, None),
            "an earlier build's row keeps NULL"
        );

        let set = |sql: &str| conn.execute(sql, []);
        assert!(set("UPDATE sync_genesis SET genesis_snapshot_id = zeroblob(15)").is_err());
        assert!(
            set("UPDATE sync_genesis SET genesis_snapshot_id = zeroblob(16)").is_err(),
            "an id without its digest"
        );
        assert!(
            set("UPDATE sync_genesis SET genesis_container_sha256 = zeroblob(32)").is_err(),
            "a digest without its id"
        );
        assert!(
            set("UPDATE sync_genesis SET genesis_snapshot_id = zeroblob(16),
                                     genesis_container_sha256 = zeroblob(31)")
            .is_err()
        );
        assert!(
            set("UPDATE sync_genesis SET genesis_snapshot_id = zeroblob(16),
                                     genesis_container_sha256 = zeroblob(32)")
            .is_ok()
        );
        assert_eq!(identity(&conn).0.map(|id| id.len()), Some(16));

        assert_eq!(migrate_down(&mut conn, 72).unwrap(), 3);
        let columns: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sync_genesis')
                 WHERE name LIKE 'genesis_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(columns, 0, "down drops both columns");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_genesis", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "and keeps the row");
        apply(&mut conn, &MIGRATIONS[..75]).unwrap();
        assert_eq!(identity(&conn), (None, None));
    }

    /// Version 75 lets two attachments share one blob (the same storage id) and two staged artifacts
    /// name one blob, keeps every row of the rebuilt table in its order, and keeps a lookup index.
    #[test]
    fn v75_lets_rows_share_a_blob_and_carries_every_row_over() {
        let mut conn = Connection::open_in_memory().unwrap();
        // Up to 74: the unique constraint is still there.
        apply(&mut conn, &MIGRATIONS[..MIGRATIONS.len() - 3]).unwrap();
        let attach = |conn: &Connection, id: u8, sid: &str| {
            conn.execute(
                "INSERT INTO attachments (id, storage_id, wrapped_content_key, content_key_nonce,
                     content_nonce, content_alg, plaintext_size, mime_type, original_filename,
                     ref_count, created_at)
                 VALUES (?1, ?2, x'aa', x'01', x'02', 'AES-256-GCM', 5,
                         'text/plain', 'f.txt', 3, 't')",
                rusqlite::params![vec![id; 16], sid],
            )
        };
        assert!(attach(&conn, 1, "sb").is_ok());
        assert!(
            attach(&conn, 2, "sb").is_err(),
            "version 74 keeps it unique"
        );
        assert!(attach(&conn, 2, "sc").is_ok());

        apply(&mut conn, MIGRATIONS).unwrap();
        let rows: Vec<(String, i64)> = conn
            .prepare("SELECT storage_id, ref_count FROM attachments ORDER BY rowid")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(rows, [("sb".to_owned(), 3), ("sc".to_owned(), 3)]);
        assert!(attach(&conn, 3, "sb").is_ok(), "two rows share a blob");
        assert!(attach(&conn, 3, "sd").is_err(), "an id is still one row");
        let index: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                  WHERE name = 'idx_attachments_storage_id' AND sql NOT LIKE '%UNIQUE%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index, 1);

        // Two staged artifacts may name one blob too.
        let stage = |n: u8| {
            conn.execute(
                &format!(
                    "INSERT INTO sync_artifacts
                         (key_epoch, sync_artifact_id, ciphertext_sha256, kind, logical_id,
                          staged_storage_id, staged_wrapped_content_key, staged_content_key_nonce,
                          staged_content_nonce, staged_content_alg, staged_plaintext_size)
                     VALUES (1, x'{}', zeroblob(32), 1, zeroblob(16), 'sb', x'aa', x'00', x'01',
                             'AES-256-GCM', 5)",
                    format!("{n:02x}").repeat(32)
                ),
                [],
            )
        };
        assert!(stage(1).is_ok());
        assert!(stage(2).is_ok(), "two staged artifacts name one blob");
    }

    /// Down with no shared blob restores the unique constraint and keeps the rows; while two rows
    /// share a blob it refuses, rolls back, and drops nothing.
    #[test]
    fn v75_down_restores_uniqueness_and_refuses_while_a_blob_is_shared() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();
        let attach = |conn: &Connection, id: u8, sid: &str| {
            conn.execute(
                "INSERT INTO attachments (id, storage_id, wrapped_content_key, content_key_nonce,
                     content_nonce, content_alg, plaintext_size, ref_count, created_at)
                 VALUES (?1, ?2, x'aa', x'01', x'02', 'AES-256-GCM', 5, 1, 't')",
                rusqlite::params![vec![id; 16], sid],
            )
            .unwrap();
        };
        let count = |conn: &Connection| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0))
                .unwrap()
        };
        attach(&conn, 1, "sb");
        attach(&conn, 2, "sb");
        assert!(
            migrate_down(&mut conn, 74).is_err(),
            "a shared blob refuses"
        );
        assert_eq!(count(&conn), 2, "a refused down drops no row");
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .unwrap();
        // Version 76 stepped down first (its tables are empty); 75 refused and stays recorded.
        assert_eq!(version, 75, "and the step is not recorded");
        conn.execute("DELETE FROM attachments WHERE id = ?1", [vec![2_u8; 16]])
            .unwrap();
        // A row and a staged artifact naming one blob refuse too (version 74 assumes the two are
        // disjoint), and the refusal leaves nothing behind.
        conn.execute(
            "INSERT INTO sync_artifacts
                 (key_epoch, sync_artifact_id, ciphertext_sha256, kind, logical_id,
                  staged_storage_id, staged_wrapped_content_key, staged_content_key_nonce,
                  staged_content_nonce, staged_content_alg, staged_plaintext_size)
             VALUES (1, zeroblob(32), zeroblob(32), 1, zeroblob(16), 'sb', x'aa', x'00', x'01',
                     'AES-256-GCM', 5)",
            [],
        )
        .unwrap();
        assert!(
            migrate_down(&mut conn, 74).is_err(),
            "a row and a staging share a blob"
        );
        assert_eq!(count(&conn), 1);
        let leftover: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_temp_master WHERE name LIKE 'attachments_down%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftover, 0, "the refusal leaves no temporary table");
        conn.execute("DELETE FROM sync_artifacts", []).unwrap();
        assert_eq!(migrate_down(&mut conn, 74).unwrap(), 1);
        assert_eq!(count(&conn), 1);
        let duplicate = conn.execute(
            "INSERT INTO attachments (id, storage_id, wrapped_content_key, content_key_nonce,
                 content_nonce, content_alg, plaintext_size, ref_count, created_at)
             VALUES (zeroblob(16), 'sb', x'aa', x'01', x'02', 'AES-256-GCM', 5, 1, 't')",
            [],
        );
        assert!(duplicate.is_err(), "unique again");
        apply(&mut conn, MIGRATIONS).unwrap();
        assert_eq!(count(&conn), 1, "up again carries the row over");
    }

    /// Version 74 creates the live closure's table empty, with its keys and its checks (an id and a
    /// hash of 32 bytes, a logical id of 16, a kind of two values, an epoch of 1 or more); an id has one
    /// row per epoch; down drops the table and its index, and up creates it empty again.
    #[test]
    fn v74_creates_the_live_closure_with_its_checks_and_down_drops_it() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();
        let count = |conn: &Connection| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM sync_live_closure", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(count(&conn), 0);
        let insert =
            |conn: &Connection, epoch: i64, id: &str, hash: &str, kind: &str, logical: &str| {
                conn.execute(
                    &format!(
                        "INSERT INTO sync_live_closure
                         (key_epoch, sync_artifact_id, ciphertext_sha256, logical_kind, logical_id)
                     VALUES ({epoch}, {id}, {hash}, '{kind}', {logical})"
                    ),
                    [],
                )
            };
        let good = |conn: &Connection, epoch: i64| {
            insert(
                conn,
                epoch,
                "zeroblob(32)",
                "zeroblob(32)",
                "attachment",
                "zeroblob(16)",
            )
        };
        assert!(good(&conn, 1).is_ok());
        assert!(good(&conn, 1).is_err(), "an id has one row per epoch");
        assert!(
            good(&conn, 2).is_ok(),
            "the same id in another epoch is another row"
        );
        assert!(good(&conn, 0).is_err(), "an epoch is 1 or more");
        assert!(insert(
            &conn,
            3,
            "zeroblob(31)",
            "zeroblob(32)",
            "batch",
            "zeroblob(16)"
        )
        .is_err());
        assert!(insert(
            &conn,
            3,
            "zeroblob(32)",
            "zeroblob(31)",
            "batch",
            "zeroblob(16)"
        )
        .is_err());
        assert!(insert(
            &conn,
            3,
            "zeroblob(32)",
            "zeroblob(32)",
            "batch",
            "zeroblob(15)"
        )
        .is_err());
        assert!(
            insert(
                &conn,
                3,
                "zeroblob(32)",
                "zeroblob(32)",
                "part",
                "zeroblob(16)"
            )
            .is_err(),
            "a kind is attachment or batch"
        );
        assert!(insert(
            &conn,
            3,
            "zeroblob(32)",
            "zeroblob(32)",
            "batch",
            "zeroblob(16)"
        )
        .is_ok());
        let index: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'sync_live_closure_logical'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index, 1);

        assert_eq!(migrate_down(&mut conn, 73).unwrap(), 4);
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'sync_live_closure%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0, "down drops the table and its index");
        apply(&mut conn, MIGRATIONS).unwrap();
        assert_eq!(count(&conn), 0, "up creates it empty again");
    }

    fn hex_upper(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02X}")).collect()
    }

    fn user_version_of(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    /// A fresh vault has no `vault_metadata` row when migrations run: nothing is backfilled and
    /// the column still goes.
    #[test]
    fn v61_on_a_fresh_vault_creates_empty_tables() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, MIGRATIONS).unwrap();
        assert!(household_rows(&conn).is_empty());
    }

    /// 62 only adds the tombstone column, so every existing scenario is live, and its down
    /// refuses while a scenario is tombstoned (dropping the column would resurrect it).
    #[test]
    fn v62_leaves_scenarios_live_and_its_down_refuses_while_one_is_tombstoned() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..61]).unwrap();
        conn.execute(
            "INSERT INTO scenarios (id, name, status, created_at, updated_at)
             VALUES (x'00000000000000000000000000000001', 'Move', 'draft', 't', 't')",
            [],
        )
        .unwrap();
        apply(&mut conn, &MIGRATIONS[61..62]).unwrap();
        let tombstoned = |conn: &Connection| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM scenarios WHERE deleted_at IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(tombstoned(&conn), 0, "an existing scenario is live");

        conn.execute("UPDATE scenarios SET deleted_at = 't'", [])
            .unwrap();
        assert!(
            migrate_down(&mut conn, 61).is_err(),
            "down must refuse with a tombstoned scenario"
        );
        // `apply` stamps the newest version it knows, so "still applied" is what the tracker says.
        assert!(
            super::applied_versions(&conn)
                .unwrap()
                .iter()
                .any(|(v, _)| *v == 62),
            "62 is still applied"
        );
        assert_eq!(tombstoned(&conn), 1, "nothing changed");

        conn.execute("UPDATE scenarios SET deleted_at = NULL", [])
            .unwrap();
        assert_eq!(migrate_down(&mut conn, 61).unwrap(), 1);
        assert_eq!(user_version_of(&conn), 61);
        let kept: i64 = conn
            .query_row("SELECT COUNT(*) FROM scenarios", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 1, "the scenario survives the down");
    }

    /// 63 adds the marker and its outcome as NULL on every existing job, refuses an unpaired or unknown
    /// marker, and its down refuses while a job carries one.
    #[test]
    fn v63_adds_an_empty_paired_commit_marker_and_its_down_refuses_while_one_is_set() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..62]).unwrap();
        conn.execute(
            "INSERT INTO durable_jobs (id, kind, cadence, state, next_due_at, max_attempts,
                                       created_at, updated_at)
             VALUES (x'00000000000000000000000000000001', 'k', 'once', 'queued', 't', 3, 't', 't')",
            [],
        )
        .unwrap();
        apply(&mut conn, &MIGRATIONS[62..63]).unwrap();
        let marked = |conn: &Connection| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM durable_jobs
                  WHERE committed_at IS NOT NULL AND committed_outcome IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(marked(&conn), 0, "an existing job has committed nothing");

        // The two are set and cleared together, and the outcome is one of two words.
        for refused in [
            "UPDATE durable_jobs SET committed_at = 't'",
            "UPDATE durable_jobs SET committed_outcome = 'succeeded'",
            "UPDATE durable_jobs SET committed_at = 't', committed_outcome = 'maybe'",
        ] {
            assert!(conn.execute(refused, []).is_err(), "{refused}");
        }
        for outcome in ["succeeded", "failed"] {
            conn.execute(
                "UPDATE durable_jobs SET committed_at = 't', committed_outcome = ?1",
                [outcome],
            )
            .unwrap();
            assert_eq!(marked(&conn), 1);
            conn.execute(
                "UPDATE durable_jobs SET committed_at = NULL, committed_outcome = NULL",
                [],
            )
            .unwrap();
        }

        conn.execute(
            "UPDATE durable_jobs SET committed_at = 't', committed_outcome = 'failed'",
            [],
        )
        .unwrap();
        assert!(
            migrate_down(&mut conn, 62).is_err(),
            "down must refuse while a job carries a marker"
        );
        assert!(
            super::applied_versions(&conn)
                .unwrap()
                .iter()
                .any(|(v, _)| *v == 63),
            "63 is still applied"
        );
        assert_eq!(marked(&conn), 1, "nothing changed");

        conn.execute(
            "UPDATE durable_jobs SET committed_at = NULL, committed_outcome = NULL",
            [],
        )
        .unwrap();
        assert_eq!(migrate_down(&mut conn, 62).unwrap(), 1);
        assert_eq!(user_version_of(&conn), 62);
        let kept: i64 = conn
            .query_row("SELECT COUNT(*) FROM durable_jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 1, "the job survives the down");
        for column in ["committed_at", "committed_outcome"] {
            let present: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('durable_jobs') WHERE name = ?1",
                    [column],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(present, 0, "{column} is gone");
        }
    }

    /// A vault has exactly one household, however a row is inserted.
    #[test]
    fn a_second_household_is_refused_by_the_schema() {
        let (mut conn, _) = v60_vault_with_metadata("2026-03-01T09:30:00+00:00", "UTC");
        apply(&mut conn, &MIGRATIONS[60..61]).unwrap();
        let second = conn.execute(
            "INSERT INTO households (id, name, household_timezone, created_at, updated_at)
             VALUES (x'00000000000000000000000000000002', 'x', 'UTC', 't', 't')",
            [],
        );
        assert!(second.is_err());
    }

    /// At most one live primary exists, and a tombstoned primary does not count.
    #[test]
    fn the_schema_allows_at_most_one_live_primary() {
        let (mut conn, _) = v60_vault_with_metadata("2026-03-01T09:30:00+00:00", "UTC");
        apply(&mut conn, &MIGRATIONS[60..61]).unwrap();
        let insert =
            "INSERT INTO household_members (id, household_id, display_name, role, created_at)
                      VALUES (x'00000000000000000000000000000009', (SELECT id FROM households),
                              'Second', 'primary', 't')";
        assert!(conn.execute(insert, []).is_err());
        conn.execute_batch("UPDATE household_members SET deleted_at = 't'")
            .unwrap();
        conn.execute(insert, []).unwrap();
    }
}
