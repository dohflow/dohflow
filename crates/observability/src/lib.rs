//! Logging redaction for DohFlow (plan §6.6, personal-cfo-2vs).
//!
//! Default logs must be safe to share in a bug report. This crate provides a
//! defense-in-depth backstop: [`redact`] rewrites sensitive substrings to typed
//! placeholders, and [`RedactingMakeWriter`] wraps any `tracing` writer so
//! **every** formatted log line is redacted before it reaches a sink — even if
//! some call site accidentally puts a sensitive value in a span field.
//!
//! Source-level discipline (an allow-list of non-sensitive fields, as the
//! Finance Kernel already does) remains the first line of defence; this layer
//! catches what slips through.
//!
//! **Never log** (§6.6): account numbers, transaction descriptions, balances,
//! provider tokens, API/vault keys, passwords, raw document text, agent
//! prompts/LLM responses with financial data. **Allowed**: event types, error
//! codes, module names, timings, counts, hashes / redacted IDs.
//!
//! Scope of this crate: the redactor + the tracing layer + a representative
//! corpus. Crash-report redaction (`personal-cfo-fps`), exhaustive/fuzz corpora
//! (`q5ko`/`64st`/`mt6s`), the release-blocking CI gate (`zobt`), and the
//! telemetry-export emitter (`lyd`/`3cw`) build on this.

use std::io::{self, Write};
use std::sync::OnceLock;

use regex::Regex;
use tracing_subscriber::fmt::MakeWriter;

struct Rule {
    re: Regex,
    replacement: &'static str,
}

/// The ordered redaction rules. Order matters: higher-confidence, more specific
/// patterns (emails, key=value secrets, tokens, currency) run before the broad
/// long-digit-run account-number rule, so a balance like `4,210.55` is not
/// partially rewritten as an account number.
fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            // URLs carrying credentials: userinfo (`user:pass@`) is the SimpleFIN
            // access-URL shape — the whole URL is the secret (ADR 0060 §1) —
            // and `/claim/<token>` paths carry the one-time setup secret.
            Rule {
                re: Regex::new(r"https?://[^\s/@]+:[^\s/@]*@\S+").unwrap(),
                replacement: "[CREDENTIAL_URL]",
            },
            Rule {
                re: Regex::new(r"(https?://[^\s/]+)/simplefin/claim/\S+").unwrap(),
                replacement: "${1}/simplefin/claim/[REDACTED]",
            },
            Rule {
                re: Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap(),
                replacement: "[EMAIL]",
            },
            // key=value (or key: value) secrets — keep the key, drop the value.
            Rule {
                re: Regex::new(
                    r"(?i)\b(password|passwd|secret|token|api[_-]?key|vault[_-]?key|private[_-]?key)\b(\s*[=:]\s*)(\S+)",
                )
                .unwrap(),
                replacement: "${1}${2}[REDACTED]",
            },
            // Bearer / provider token prefixes.
            Rule {
                re: Regex::new(
                    r"(?i)\b(?:bearer\s+|sk-|pk-|sk_live_|sk_test_|pk_live_|pk_test_)[A-Za-z0-9_\-]{8,}",
                )
                .unwrap(),
                replacement: "[TOKEN]",
            },
            // Currency / balances: `$` amounts, or thousands-grouped numbers.
            Rule {
                re: Regex::new(r"\$\s?\d[\d,]*(?:\.\d+)?|\b\d{1,3}(?:,\d{3})+(?:\.\d+)?\b").unwrap(),
                replacement: "[BALANCE]",
            },
            // Card / account numbers: 12–19 digits, contiguous OR split into groups by
            // single spaces or dashes (free-text notes carry "4111 1111 1111 1111" and
            // "4111-1111-1111-1111", personal-cfo-4d8.16). UUIDs are hyphenated but mix
            // letters; short numbers (op_seq, counts, durations) are well under 12.
            Rule {
                re: Regex::new(r"\b\d(?:[ -]?\d){11,18}\b").unwrap(),
                replacement: "[ACCT_NUMBER]",
            },
        ]
    })
}

/// Rewrite sensitive substrings of `input` to typed placeholders (§6.6).
///
/// Safe strings (event types, module names, timings, counts, redacted UUIDs)
/// pass through unchanged.
#[must_use]
pub fn redact(input: &str) -> String {
    let mut out = input.to_owned();
    for rule in rules() {
        out = rule.re.replace_all(&out, rule.replacement).into_owned();
    }
    out
}

/// A [`Write`] that redacts each chunk before forwarding it to `inner`.
pub struct RedactingWriter<W: Write> {
    inner: W,
}

impl<W: Write> RedactingWriter<W> {
    /// Wrap `inner` so writes pass through [`redact`].
    pub fn new(inner: W) -> Self {
        Self { inner }
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let redacted = redact(&String::from_utf8_lossy(buf));
        self.inner.write_all(redacted.as_bytes())?;
        // Report the original length consumed (the redacted bytes were written).
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A [`MakeWriter`] that wraps another make-writer so the `tracing` fmt layer's
/// output flows through [`redact`]. Use via
/// `fmt::layer().with_writer(RedactingMakeWriter::new(inner))`.
pub struct RedactingMakeWriter<M> {
    inner: M,
}

impl<M> RedactingMakeWriter<M> {
    /// Wrap `inner` (e.g. `std::io::stdout`, or any `MakeWriter`).
    pub fn new(inner: M) -> Self {
        Self { inner }
    }
}

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for RedactingMakeWriter<M> {
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new(self.inner.make_writer())
    }
}

/// Install the global `tracing` subscriber with the redacting fmt layer writing
/// to stdout, honouring `RUST_LOG` (default `info`). Idempotent: a second call
/// (or a subscriber already set by another component, e.g. tests) is ignored.
pub fn init() {
    use tracing_subscriber::prelude::*;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        // Hard floors appended AFTER RUST_LOG so user overrides cannot
        // re-enable them: ureq debug-logs full request URLs, and for
        // SimpleFIN the claim URL is a one-time secret (personal-cfo-w3gh).
        .add_directive("ureq=warn".parse().expect("static directive"))
        .add_directive("rustls=warn".parse().expect("static directive"));
    let layer = tracing_subscriber::fmt::layer().with_writer(RedactingMakeWriter::new(io::stdout));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init();
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// §6.6 known-positive corpus: each must be redacted to a placeholder, with
    /// the raw sensitive value absent from the output.
    #[test]
    fn known_positive_corpus_is_redacted() {
        let cases = [
            (
                "account 1234567890123456",
                "[ACCT_NUMBER]",
                "1234567890123456",
            ),
            ("balance $4,210.55", "[BALANCE]", "4,210.55"),
            ("password=hunter2", "[REDACTED]", "hunter2"),
            (
                "api_key: sk-live_abcdefgh12345",
                "[REDACTED]",
                "sk-live_abcdefgh12345",
            ),
            (
                "Authorization: Bearer abcdef0123456789",
                "[TOKEN]",
                "abcdef0123456789",
            ),
            ("contact user@example.com", "[EMAIL]", "user@example.com"),
            // SimpleFIN credential shapes (personal-cfo-w3gh): the access URL
            // IS the secret, and a claim URL path carries the one-time token.
            (
                "sending request GET https://demo:s3cretpw@bridge.example/simplefin/accounts",
                "[CREDENTIAL_URL]",
                "s3cretpw",
            ),
            (
                "POST https://beta-bridge.simplefin.org/simplefin/claim/DEMO-abc123XYZ",
                "/simplefin/claim/[REDACTED]",
                "DEMO-abc123XYZ",
            ),
        ];
        for (input, placeholder, secret) in cases {
            let out = redact(input);
            assert!(
                out.contains(placeholder),
                "{input:?} -> {out:?} missing {placeholder}"
            );
            assert!(
                !out.contains(secret),
                "{input:?} -> {out:?} leaked {secret}"
            );
        }
    }

    /// §6.6 known-negative corpus: safe strings must pass through unchanged.
    #[test]
    fn known_negative_corpus_passes_through() {
        for safe in [
            "create_account",
            "module=db-worker",
            "duration_ms=42",
            "count=3",
            "op_seq=7",
            "command.kind=create_account actor.type=User",
            "error_code=VaultLocked",
        ] {
            assert_eq!(redact(safe), safe, "over-redacted a safe string");
        }
    }

    #[derive(Clone, Default)]
    struct BufWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for BufWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for BufWriter {
        type Writer = BufWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// A `tracing` event carrying a sensitive field is redacted by the layer
    /// before it reaches the sink.
    #[test]
    fn tracing_events_flow_through_the_redactor() {
        use tracing_subscriber::prelude::*;

        let buf = BufWriter::default();
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(RedactingMakeWriter::new(buf.clone()));
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(account = "1234567890123456", "account created");
        });

        let logged = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(
            logged.contains("[ACCT_NUMBER]"),
            "layer did not redact: {logged:?}"
        );
        assert!(
            !logged.contains("1234567890123456"),
            "layer leaked the account: {logged:?}"
        );
    }

    /// §6.6 / personal-cfo-4d8.16: a transaction NOTE is free text where a user may paste
    /// a card / account number in any format. Each must be scrubbed — by the pure
    /// `redact` and through a `tracing` event carrying the note as a field.
    #[test]
    fn transaction_note_account_numbers_are_redacted() {
        use tracing_subscriber::prelude::*;

        let cases = [
            ("paid from account 4111111111111111", "4111111111111111"),
            ("card 4111 1111 1111 1111 on file", "4111 1111 1111 1111"),
            ("acct 4111-1111-1111-1111 reimbursed", "4111-1111-1111-1111"),
        ];

        for (note, secret) in cases {
            // The pure redactor scrubs the number regardless of separators.
            let out = redact(note);
            assert!(
                out.contains("[ACCT_NUMBER]"),
                "redact missed: {note:?} -> {out:?}"
            );
            assert!(!out.contains(secret), "redact leaked: {note:?} -> {out:?}");

            // And so does a tracing event carrying the note as a field.
            let buf = BufWriter::default();
            let layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(RedactingMakeWriter::new(buf.clone()));
            tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
                tracing::info!(note = note, "transaction note saved");
            });
            let logged = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
            assert!(
                logged.contains("[ACCT_NUMBER]"),
                "layer missed the note: {logged:?}"
            );
            assert!(
                !logged.contains(secret),
                "layer leaked the note: {logged:?}"
            );
        }
    }
}
