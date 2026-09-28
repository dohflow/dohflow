# Logging and local observability policy

- **Bead:** `personal-cfo-vkda` (this document) · **Plan:** §6.6 Logging Policy,
  §6.6.1 Local Observability Policy, §21.2 required docs
- **Implemented by:** `personal-cfo-2vs` (the `observability` crate — redactor,
  tracing layer, subscriber), `personal-cfo-zobt` (the release-blocking redaction
  gate)
- **Open work this policy governs:** `personal-cfo-q5ko` (cross-cutting redaction
  suite and span schema), `personal-cfo-lyd` (local-only metrics, export off by
  default), `personal-cfo-3cw` (privacy modes and redacted export),
  `personal-cfo-fps` (crash-report redaction), `personal-cfo-ryjx` (telemetry
  export round-trip)
- Formerly `docs/security/logging-redaction.md`, which now points here.

**Default logs must be safe to paste into a public bug report.** This page is
what that means, how the code enforces it, where it does not yet, and what a
change to logging has to update.

## 1. What may and may not be logged (§6.6)

**Never log:**

- account, card and routing numbers;
- full transaction descriptions, and merchant or payee names;
- balances and amounts;
- provider tokens, API keys, vault keys, passwords, and connector credentials
  (including SimpleFIN access URLs and one-time setup/claim tokens);
- raw document text (imported files, attachments, statement contents);
- agent prompts containing financial data;
- LLM responses containing sensitive data, unless they are intentionally stored
  as encrypted agent reports;
- local file paths the user chose (backup destinations, export paths).

**Allowed:**

- event types, command kinds and names, module names;
- error codes (typed variants, not free-text messages built from user data);
- timings and durations, counts;
- hashes and opaque identifiers (UUIDs, adapter ids and versions);
- outcome labels (`success`, an IPC error code).

## 2. How it is enforced

Enforcement has three layers. Each is expected to fail safe if the one above it
misses something.

### 2.1 Source discipline (first line)

Call sites log an allow-list of non-sensitive fields only. The boundary spans
(§3) record identifiers, kinds, codes, durations and outcomes, never payloads,
amounts, paths or URLs. Where a value that *might* carry a secret has to be
logged, the call site redacts it before the event is emitted:
`record_release_update_failure` passes the updater's raw error text through
`observability::redact()` itself.

**This is the only layer that protects free text.** Merchant names,
descriptions and document text have no reliable pattern, so the redactor below
cannot catch them. They must never reach a log field in the first place.

### 2.2 The redactor (backstop)

The `observability` crate (`crates/observability/src/lib.rs`) holds one redactor:

- **`redact(&str) -> String`** rewrites sensitive substrings to typed
  placeholders. Rules apply in this order (order matters: credential URLs are
  consumed before the email and number rules can split them):

  | # | Pattern | Placeholder |
  |---|---|---|
  | 1 | URL with userinfo (`https://user:pass@host/…` — the SimpleFIN access-URL shape, where the whole URL is the secret, ADR 0060) | `[CREDENTIAL_URL]` |
  | 2 | `…/simplefin/claim/<token>` (one-time setup secret) | host kept, token `[REDACTED]` |
  | 3 | email address | `[EMAIL]` |
  | 4 | `password=` / `passwd` / `secret` / `token` / `api_key` / `vault_key` / `private_key` followed by `=` or `:` and a value | key kept, value `[REDACTED]` |
  | 5 | `Bearer …`, `sk-…`, `pk-…`, `sk_live_`/`sk_test_`/`pk_live_`/`pk_test_` tokens | `[TOKEN]` |
  | 6 | `$` amounts, or thousands-grouped numbers (`98,765.43`) | `[BALANCE]` |
  | 7 | 12–19 digit runs, contiguous or grouped by single spaces/dashes (`4111 1111 1111 1111`) | `[ACCT_NUMBER]` |

  UUIDs (hyphenated, with letters), op-log sequence numbers, counts and
  durations are shorter or differently shaped, and pass through.

- **`RedactingMakeWriter` / `RedactingWriter`** wrap any `tracing` writer, so
  **every formatted line** is redacted before it reaches the sink, whichever
  call site produced it.

- **`init()`** installs the one global subscriber, called first thing in the
  desktop app's `run()`:
  - a `tracing_subscriber` fmt layer over `RedactingMakeWriter::new(stdout)`;
  - filter `RUST_LOG`, default `info`;
  - **hard floors appended after `RUST_LOG`**, so a user override cannot lift
    them: `ureq=warn` (ureq debug-logs full request URLs, and a SimpleFIN claim
    URL is a one-time secret) and `rustls=warn`.

  The desktop app deliberately does **not** also install `tauri-plugin-log`. It
  would install a second logger (and panic on the second `set_logger`), and its
  lines would bypass the redactor.

**Rule for any new sink.** A file log, crash report, telemetry export or
diagnostic bundle must route through this redactor (`RedactingMakeWriter`, or
`redact()` on every string it writes), never a second implementation.
`personal-cfo-q5ko` makes "one redactor for every sink" a tested invariant.

### 2.3 CI gates

- **Unit corpus**, in `observability`'s own tests: `known_positive_corpus_is_redacted`,
  `known_negative_corpus_passes_through`, `tracing_events_flow_through_the_redactor`,
  `transaction_note_account_numbers_are_redacted`.
- **Release-blocking gate** (`personal-cfo-zobt`):
  `crates/finance-kernel/tests/log_redaction.rs`
  (`no_sensitive_value_survives_into_logs`). A real Finance Kernel workload runs
  under the **production** redacting subscriber, and the full known-positive
  corpus is emitted as deliberate slips. The test fails if any raw secret appears
  in the captured output, or if any known-negative value is altered (no
  over-redaction).

Both run in `cargo test --workspace` (the CI `Rust (workspace)` job), which
blocks merge. There is no separately named "redaction" CI step. A redaction
regression fails the same job as any other Rust test failure (noted as residual
risk in `release-review-v0.1.md`).

## 3. Span schema

### 3.1 The target (Definition of Done §2.1)

Every Tauri command, Finance Kernel command, importer, connector, agent
invocation and projection run emits **one** boundary span, carrying:

| Field | Meaning | Allowed values |
|---|---|---|
| `command_id` | this operation | UUID v7 |
| `correlation_id` | the user-initiated action it belongs to | UUID v7 |
| `causation_id` | the operation that caused it | UUID v7, or `none` |
| `actor_type` / `actor_id` | who initiated it | `user` / `local-user`, or a system actor |
| `command` | what ran | a static command name |
| `duration_ms` | how long | integer |
| `outcome` | how it ended | `success`, or a typed error code |

Operation-specific fields are allowed only if they are identifiers from §1's
allowed list (for example `adapter_id`, `adapter_version`).

### 3.2 What ships today

| Span | Where | Fields | Covers |
|---|---|---|---|
| `tauri_command` | `apps/desktop/src-tauri/src/ipc/commands.rs` | the full §3.1 set, plus `adapter_id` and `adapter_version` for connectors | `export_backup`, `record_release_update_failure`, and every connector operation (`connector_span`) |
| `kernel.dispatch` | `crates/finance-kernel/src/lib.rs` (`Kernel::dispatch`) | `command.kind`, `command.id`, `actor.type` | every Finance Kernel command |
| `job.invocation` | `crates/job-runtime/src/lib.rs` | `job.kind`, `attempt`, `outcome` | every durable-job run |

### 3.3 Known gaps (owned by `personal-cfo-q5ko`)

- **Most IPC commands have no `tauri_command` span yet.** Kernel-backed ones
  are covered by `kernel.dispatch`; read-only and settings commands are not
  spanned.
- **Correlation isn't propagated.** Each `tauri_command` span mints a fresh
  `correlation_id`, and `causation_id` is always `none`. `kernel.dispatch` does
  not record the correlation id. The op-log carries correlation and causation
  for replay (ADR 0011), but the logs do not yet join one action across layers.
  The standalone log-correlation test (`personal-cfo-1de5`) was closed as
  superseded, not shipped.
- **Naming is inconsistent.** `kernel.dispatch` and `job.invocation` use dotted
  fields (`command.kind`); `tauri_command` uses snake_case (`command_id`). The
  span schema is not versioned yet.

New spans follow §3.1 (snake_case), and closing these gaps is `q5ko`'s job.
Until then, the §1 field rules apply to every span regardless of shape.

## 4. The never-log corpus

The corpus is the executable form of §1. It lives in two places, and the two
must stay in step:

| Corpus | File | What it proves |
|---|---|---|
| Unit known-positive / known-negative | `crates/observability/src/lib.rs` tests | every pattern class in §2.2 is replaced, including credential URLs and SimpleFIN claim tokens; innocuous strings (module names, command kinds, error codes, counts, `duration_ms`, `op_seq`) pass unchanged |
| Live known-positive / known-negative (`POSITIVE` / `NEGATIVE`) | `crates/finance-kernel/tests/log_redaction.rs` | the same classes, through the **production** subscriber, alongside a real kernel workload, with none surviving and nothing over-redacted |

**Classes covered by both corpora:**
- account, card and routing numbers;
- `$` and grouped balances;
- key=value secrets;
- bearer and provider tokens;
- emails.

Credential URLs and SimpleFIN claim tokens are covered by the unit corpus only.
Adding them to the live gate is a cheap follow-up.

**Not covered by any corpus:** free-text classes (merchant names, descriptions,
document text). These have no reliable pattern, and source discipline (§2.1) is
their only control. The kernel-workload half of the gate exercises that
discipline for kernel commands.

**Not built:** redactor fuzzing (`personal-cfo-mt6s`) and a broader
every-area/malformed-input corpus (`personal-cfo-64st`) were closed as
superseded by the zobt gate ("revisit post-launch if redaction bugs surface").
The redactor has no fuzz or property tests today.

**Extending the corpus.** A new sensitive class, or a new source of secrets
(for example a new connector's credential format), adds a known-positive case to
**both** files and a pattern to §2.2 in the same PR. Anything that must pass
through untouched goes in both known-negative lists.

## 5. Local observability (§6.6.1)

**The policy.** The app may collect local-only operational telemetry for
reliability, with export disabled by default:
- **Allowed metrics:** job durations, import row counts, dedupe candidate counts,
  forecast and query duration buckets, UI render timing buckets, redacted
  connector status codes, retry counts, migration durations, and backup/restore
  success or failure.
- **Where it lives:** telemetry stays inside the encrypted vault or in ephemeral
  memory.
- **Leaving the device:** nothing leaves unless the user previews it and
  explicitly exports it.
- **Diagnostic export:** has a **redacted** mode and a **full-local** mode, and
  full-local stays encrypted and user-controlled.
- **CI:** redaction tests are part of CI (§2.3).

**What ships today (the conservative subset of the policy):**

| Surface | State |
|---|---|
| Rust logs | Redacted `tracing` output to **stdout only**. The app writes no log file. |
| Telemetry and metrics collection | **None** (`personal-cfo-lyd` builds the local-only metrics store) |
| Telemetry or diagnostic export | **None** (`lyd`, with privacy modes and redacted export from `personal-cfo-3cw`, and the round-trip test from `personal-cfo-ryjx`) |
| Crash reporting | **None** (`personal-cfo-fps` owns crash-report redaction before any reporter ships) |
| Network egress for any of the above | **None**. The WebView CSP allows no remote origin (ADR 0010, `csp-egress`), and no Rust-side telemetry client exists. |
| WebView console | Not persisted or collected anywhere. But it sits **outside** the redactor: frontend `console.*` calls (failed IPC calls, a refused external link) must follow §1 at the call site. Logging a user-entered value or a financial field there is a policy violation. |

Adding any row, a file sink, a metric store, an exporter or a crash reporter, is
a change to this policy (§6).

## 6. Changing logging

In the same PR as the change:

- **A new log field or span** follows §1 and §3.1. If it could carry user data,
  it is redacted at the call site (§2.1).
- **A new sink** (file, crash report, telemetry, diagnostic export) routes
  through the one redactor (§2.2), gets a known-positive round-trip test, and
  updates §5's table. Anything leaving the device also needs the user-preview
  and explicit-export flow of §6.6.1, and its own review against ADR 0003 and
  ADR 0010.
- **A new sensitive class or secret format** extends both corpora (§4) and the
  pattern table (§2.2).
- **This document** is updated. Security-sensitive changes follow the full review
  path (AGENTS.md §16).

## 7. Related

- `docs/security/threat-model.md`, row "Financial data leaks into logs".
- `docs/security/capability-audit-2026-09-27.md` and
  `docs/security/tauri-regression-suite.md`: the WebView egress controls that
  keep anything logged in the frontend from leaving the device.
- `docs/architecture/definition-of-done.md` §2: logging obligations per feature.
