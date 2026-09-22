//! Logging-redaction gate (personal-cfo-zobt, §6.6, release-blocking).
//!
//! Proves that no §6.6 sensitive value reaches a log sink. A realistic Finance
//! Kernel workload runs **under the production redacting subscriber**
//! (`observability::RedactingMakeWriter` over the `tracing` fmt layer) so every
//! span the kernel actually emits flows through the redactor; alongside it, the
//! full known-positive corpus is emitted as deliberate "slips". The captured log
//! buffer must contain **none** of the raw secrets, and the known-negative corpus
//! of innocuous values must survive **unchanged** (no over-redaction). Runs in CI
//! via `cargo test --workspace`; a survivor fails the build.
//!
//! This is the live, end-to-end complement to the redactor's unit corpus in the
//! `observability` crate (personal-cfo-2vs). Source-level field discipline is the
//! first line of defence; this gate proves the backstop catches what slips
//! through. Note the redactor matches *patterns* (numbers, balances, tokens,
//! keys, emails) — free-text classes (merchant names, descriptions) are kept out
//! of logs by source discipline, which the kernel-workload half exercises.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope, CommandMeta,
    CreateAccount, Currency, Kernel, LedgerAccountId, Money, RecordTransaction, TransactionId,
};
use observability::RedactingMakeWriter;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

/// §6.6 known-positive corpus: `(log line, the raw secret that must be absent
/// after redaction)`. Covers every pattern class the redactor enforces — account
/// / card / routing numbers, `$` and grouped balances, key=value secrets, bearer
/// / provider tokens, and emails.
const POSITIVE: &[(&str, &str)] = &[
    ("account=4111111111111111", "4111111111111111"),
    ("card 5500005555555559", "5500005555555559"),
    ("routing 123456789012", "123456789012"),
    ("balance $98,765.43", "98,765.43"),
    ("net pay $4,210.00", "4,210.00"),
    ("amount 1,234,567.89", "1,234,567.89"),
    ("password=hunter2", "hunter2"),
    ("api_key: sk-live_abcdefgh12345", "sk-live_abcdefgh12345"),
    ("vault_key=deadbeefcafef00d", "deadbeefcafef00d"),
    ("Authorization: Bearer abcdef0123456789", "abcdef0123456789"),
    (
        "statement to jane.doe+stmt@bank.co.uk",
        "jane.doe+stmt@bank.co.uk",
    ),
];

/// §6.6 known-negative corpus: allowed, non-sensitive values (event kinds, actor
/// types, op sequence, counts, timings, error codes, module names) that must pass
/// through the redactor unchanged.
const NEGATIVE: &[&str] = &[
    "command.kind=record_transaction",
    "actor.type=User",
    "op_seq=7",
    "count=3",
    "duration_ms=42",
    "error_code=VaultLocked",
    "schema_version=5",
    "module=db-worker",
];

/// The account number embedded in the seeded account's name — if any kernel span
/// logs the name, the backstop must still redact the number.
const NAME_EMBEDDED_NUMBER: &str = "4111111111111111";

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "zobt".to_owned(),
        idempotency_key: Uuid::now_v7().to_string(),
    }
}

fn at(rfc3339: &str) -> DateTime<Utc> {
    rfc3339.parse().unwrap()
}

#[test]
fn no_sensitive_value_survives_into_logs() {
    use tracing_subscriber::prelude::*;

    let buf = Capture::default();
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(RedactingMakeWriter::new(buf.clone()));
    // No EnvFilter: every event at every level reaches the redacting layer.
    let subscriber = tracing_subscriber::registry().with(layer);

    tracing::subscriber::with_default(subscriber, || {
        // A realistic workload, so the kernel's own spans flow through the
        // redactor. The account name embeds a card-like number to also exercise
        // source discipline (the name must not reach a log un-redacted).
        let dir = tempfile::tempdir().unwrap();
        let kernel =
            Kernel::create_vault(dir.path().join("vault.db"), b"redaction gate pw").unwrap();
        let acct = Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            format!("Checking {NAME_EMBEDDED_NUMBER}"),
            CashflowRole::LiquidCash,
            Currency::Usd,
            AccountFlags::default(),
        );
        let acct_id = acct.id();
        kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                CreateAccount::with_opening_balance(acct, Money::new(9_876_543, Currency::Usd)),
            ))
            .unwrap();

        kernel
            .export_unattended(
                &dir.path().join("redaction-check.pcfobk"),
                "0.1.0-test",
                "2026-09-22T00:00:00Z".to_owned(),
                Uuid::from_bytes([0x42; 16]),
            )
            .unwrap();
        kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                RecordTransaction::new(
                    TransactionId::new(),
                    acct_id,
                    Money::new(-12_345, Currency::Usd),
                    at("2026-06-05T00:00:00Z"),
                ),
            ))
            .unwrap();

        // Emit the corpus as deliberate slips (use `{}` so the line is data, not a
        // format string). The layer must redact each positive before the sink.
        for (line, _) in POSITIVE {
            tracing::info!("{}", line);
        }
        for line in NEGATIVE {
            tracing::info!("{}", line);
        }
    });

    let logged = String::from_utf8_lossy(&buf.0.lock().unwrap()).into_owned();
    assert!(!logged.is_empty(), "redacting subscriber captured nothing");

    // No known-positive secret — from a slip or a kernel span — survives.
    for (line, secret) in POSITIVE {
        assert!(
            !logged.contains(secret),
            "secret {secret:?} from {line:?} survived redaction into the log:\n{logged}"
        );
    }
    assert!(
        !logged.contains(NAME_EMBEDDED_NUMBER),
        "an account number leaked into the log (source discipline + backstop):\n{logged}"
    );
    assert!(
        !logged.contains("redaction gate pw"),
        "the password used for unattended export leaked into the log:\n{logged}"
    );

    // Every known-negative value passes through unchanged (no over-redaction).
    for line in NEGATIVE {
        assert!(
            logged.contains(line),
            "innocuous value {line:?} was over-redacted or dropped:\n{logged}"
        );
    }
}
