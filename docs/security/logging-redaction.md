# Logging redaction policy (§6.6)

Default logs must be **safe to share in a bug report**. Enforcement is the
`observability` crate (`personal-cfo-2vs`): a [`redact`] backstop plus a
`tracing` layer (`RedactingMakeWriter`) that scrubs every formatted log line
before it reaches any sink.

## Never log

- Account numbers
- Full transaction descriptions
- Balances
- Provider tokens
- API keys
- Vault keys
- Passwords
- Raw document text
- Agent prompts containing financial data
- LLM responses containing sensitive data (unless intentionally stored as
  encrypted agent reports)

## Allowed

- Event types
- Error codes
- Module names
- Timings
- Counts
- Hashes / redacted IDs (UUIDs)

## How it is enforced

1. **Source discipline (first line).** Call sites log an allow-list of
   non-sensitive fields only — e.g. the Finance Kernel boundary span records
   `command.kind`, `command.id`, `actor.type`, never amounts or account
   contents.
2. **Redaction layer (backstop).** `observability::init()` installs the global
   subscriber with the redacting fmt layer. Every line passes through
   `redact()`, which rewrites sensitive substrings to typed placeholders:

   | Pattern | Placeholder |
   |---|---|
   | email address | `[EMAIL]` |
   | `password=`/`secret=`/`token=`/`*_key=` value | key kept, value `[REDACTED]` |
   | `Bearer …`, `sk-…`, `pk-…` provider tokens | `[TOKEN]` |
   | `$` amounts / thousands-grouped numbers | `[BALANCE]` |
   | 12–19 digit runs (cards / account numbers) | `[ACCT_NUMBER]` |

3. **CI.** The known-positive / known-negative corpus tests run in
   `cargo test --workspace`. The exhaustive every-area / fuzz corpora
   (`personal-cfo-q5ko`/`-64st`/`-mt6s`) and the release-blocking gate
   (`personal-cfo-zobt`) build on this redactor.

## Out of scope here (owned by other beads)

- Crash-report redaction wiring + test → `personal-cfo-fps`.
- Telemetry-export emitter + privacy modes → `personal-cfo-lyd` / `-3cw`.
- Persistent file-log sink → `personal-cfo-lyd`.
- Per-area structured-logging specs → their respective beads.
