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
- **Source decisions:** plan §6.6; ADR 0066-A §5 (no client telemetry; the only
  diagnostic export is a user-initiated, previewed, redacted bundle; seven-day
  service logs); the session-only capture decision recorded on
  `personal-cfo-vkda`. §6–§8 are the contract `personal-cfo-lyd` implements.
- Formerly `docs/security/logging-redaction.md`, which now points here.

**Default logs must be safe to paste into a public bug report.** This page is
what that means, how the code enforces it, where it does not yet, the contract
for the planned local capture and diagnostic export, and what a change to
logging has to update.

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
  | 7 | 12–19 digit runs, contiguous or grouped by single spaces or dashes (a card number written in four groups of four) | `[ACCT_NUMBER]` |

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
| Diagnostic export corpus (`FORBIDDEN`) and allowlist | `apps/desktop/src-tauri/tests/diagnostics_export_roundtrip.rs` | account and routing numbers, descriptions, balances, the vault password, a connector setup token, a provider body, raw error text and paths, fed through real vault operations, never reach a saved diagnostic bundle; the bundle holds only §7's fields and values |

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

## 5. Five kinds of record, and who owns each

"Logging" covers five different things with different rules. They must not be
confused, and no record moves from one kind to another.

| Kind | What it is | Where it lives | Lifetime | Owner (bead) | Source decision |
|---|---|---|---|---|---|
| **Stdout tracing** | redacted `tracing` output from the running app | process stdout | the process | `personal-cfo-2vs` (ships) | plan §6.6; this doc §1–§4 |
| **Local capture** | typed operational metrics collected for reliability | **process memory only**, one bounded ring per unlocked-vault session | until lock, vault switch or exit (§6.2) | `personal-cfo-lyd` (planned) | vkda session-only decision (§6.1); ADR 0066-A §5 |
| **Exported diagnostic bundle** | a redacted, previewed snapshot of the local capture that **the user chooses to save** | a file the user picks | until the user deletes it | `personal-cfo-lyd` (exporter), `personal-cfo-ryjx` (round-trip test), `personal-cfo-fps` (crash-path redaction), `personal-cfo-3cw` (the "redacted export" mode *is* this exporter) | ADR 0066-A §5 ("the only diagnostic export is a user-initiated, previewed, redacted bundle") |
| **Service operational logs** | failure and lifecycle events kept by a future DohFlow service (Sync, backup upload, AI proxy, extension registry) | the service | deleted **within seven days** | each service's bead; none ship today | ADR 0066-A §5 |
| **Protocol records** | the ciphertext-safe records a Sync service needs to work (encrypted snapshots, envelopes, receipts, retry index) | the service | ADR 0074's recovery windows; **not** subject to the seven-day rule | Sync beads | ADR 0074, ADR 0066-A §5 |

The desktop app sends **no** client telemetry, analytics, automatic crash report
or automatic diagnostic bundle anywhere (ADR 0066-A §5). The only way diagnostic
data leaves a device is a user saving a bundle and choosing to share that file.

## 6. Local capture: storage and lifecycle (for `personal-cfo-lyd`)

### 6.1 The decision

**Local capture is session-only, bounded, in-memory capture. There is no
automatic diagnostic history on disk.** The owner asked for the best long-term
option. Planning selected this on 2026-09-27 (recorded on `personal-cfo-vkda`
and `personal-cfo-lyd`), and ADR 0066-A §5 limits export to user-initiated,
previewed, redacted bundles. It is a deliberate privacy decision, not a
temporary shortcut:
- Nothing captured survives an unexpected exit or a crash. That is the stated
  trade-off, and the fix is not to persist silently.
- The durable artifact is a bundle the user explicitly saves.

The numeric limits in §6.2 were proposed in `personal-cfo-vkda`'s PR and are
approved with it. A limit is an engineering bound. It never permits capturing a
field §7 excludes.

### 6.2 Limits and lifecycle

| Property | Rule | Testable as |
|---|---|---|
| Scope | One capture ring per **unlocked-vault session**. Capture starts at unlock; events before unlock are not captured (they exist only as stdout tracing). | a record emitted while locked is not in the ring |
| Capacity | **2,000 records.** Records are fixed-size (§7), so the ring's memory is bounded; the ring must stay under **256 KiB**. | fill past 2,000 → length stays 2,000; a size assertion stays under 256 KiB |
| Eviction | Oldest-first. Every evicted record increments a **per-metric dropped counter** (a count only). | overflow → the oldest record is gone and `dropped[metric]` is incremented |
| Vault lock | Clears the ring and the dropped counters, and **invalidates any pending preview**. | after lock, the ring is empty and the preview is gone |
| Vault switch | Same as lock: the old vault's capture never follows into the new vault's session. | switch → empty ring, no preview |
| App exit (normal or crash) | Nothing is flushed to disk. The ring is lost. | no diagnostic file exists after exit |
| Restart | Nothing survives except bundles the user saved (§8). | a fresh launch has an empty ring |
| Preview snapshot | Created when the user opens Preview: an **immutable copy** of the ring and counters, rendered to the exact bytes Save would write. It lives until Save completes, Cancel, vault lock/switch, app exit, or a new Preview replaces it. Records captured after the snapshot never enter it. | a record added after Preview is absent from the saved file |

**What memory clearing does not promise.** Dropping the ring frees process
memory. It does **not** guarantee the bytes are erased from OS swap, a
hibernation image or a core dump. Keeping sensitive values out of the capture
(§7) is what protects them. `personal-cfo-fps` documents what cannot be
guaranteed about OS crash artifacts.

## 7. Admission: the typed field allowlist

**Admission is by type, not by redaction.** A record can hold only these fields.
There is no string, path or free-form message field, so a sensitive value has
nowhere to go:

| Field | Type | Values |
|---|---|---|
| `at_s` | integer | whole seconds since this capture session began (no wall-clock time) |
| `metric` | closed enum | `job_duration`, `import_rows`, `dedupe_candidates`, `forecast_duration`, `query_duration`, `ui_render_timing`, `connector_status`, `retry_count`, `migration_duration`, `backup_outcome`, `restore_outcome` (plan §6.6.1's list) |
| `value` | closed enum, one of: | |
| · `duration` | bucket | `<10ms`, `10–100ms`, `100ms–1s`, `1–10s`, `>10s` |
| · `count` | integer | saturates at 1,000,000 (shown as `≥1000000`) |
| · `status` | closed enum | a fixed category per source (for example connector `ok`, `auth_failed`, `rate_limited`, `provider_error`, `network_unavailable`), mapped from typed errors, **never** a raw HTTP status line, body, URL or error message |
| · `outcome` | closed enum | `success`, or `failure` with a fixed error category |

**Excluded, whatever the value:**
- arbitrary strings, and messages or error text;
- filesystem paths;
- any financial data (amounts, balances, account, merchant or payee names,
  descriptions);
- credentials and tokens;
- **stable identifiers** (vault, account, device and connection ids, and
  command or correlation UUIDs), plus hashes of any excluded value.

A metric source that needs anything outside this table needs this section
changed and reviewed first.

**Where capture enters (a requirement on `lyd`).** Rust call sites must
construct records directly from the enums. UI render timings must arrive through
one typed IPC command that accepts only `(ui_render_timing, duration bucket)`.
`lyd` does not add that command; frontend timing capture is
`personal-cfo-6s97`'s, and it must follow this rule.
That command is granted to `main` only, and the untrusted windows cannot reach
it (ADR 0010). Anything that doesn't fit the types is rejected at the boundary,
and the rejection is counted, never stored.

**Defense in depth.** The export serializer's output must also pass through
`observability::redact()` (§2.2). For allowlisted records that is a no-op, and
`personal-cfo-ryjx`'s round-trip test proves both.

**Honest completeness.** Every bundle states what it is *not*. Its header
carries:
- the build version and channel, and the platform;
- the bundle's creation **date** (UTC day, no time);
- how many records it holds, how many were **dropped by capacity** (per
  metric), and how many were **rejected at admission**;
- a fixed sentence: the bundle covers only the current unlocked session since its
  last unlock, and excludes earlier sessions, other vaults, and anything before a
  crash.

It never claims lossless capture.

## 8. Export contract

1. **Preview first.** The user opens Diagnostics and chooses Preview, which
   renders the **exact redacted snapshot** (§6.2), byte for byte what Save would
   write.
2. **Explicit save consent.** Saving requires a separate user action after
   preview, through the native Save dialog (the existing `dialog:allow-save`
   grant). The user picks the location.
3. **Nothing in the background.** No automatic disk export, no scheduled
   bundle, no upload, and no network request of any kind. The WebView has no
   remote origin (ADR 0010), and the exporter has no endpoint.
4. **Cancel writes nothing.** Cancelling the preview or the Save dialog creates
   no file.
5. **Safe failure.** A failed save reports a fixed category (for example
   permission denied or disk full), with no path and no payload.
6. **Redacted only.** There is **no full-local or unredacted mode.** The saved
   bundle is plaintext, deliberately: it is already redacted, the user created
   it on purpose, and it is the only durable diagnostic artifact.
7. **Lock or switch while previewing** discards the preview (§6.2); nothing is
   saved.

Plan §6.6.1 used to require "redacted and full-local modes" and allow telemetry
"inside the encrypted vault". That wording is superseded by ADR 0066-A §5 and
this section, and the plan now points here.

## 9. Local capture versus service logs

- **Local capture** (§6) is session-only and never leaves the device except
  through a user-saved bundle.
- **Service operational logs** (ADR 0066-A §5) belong to future DohFlow
  services, not to the desktop app. They record only failures and lifecycle
  events, and only a timestamp, service/build version, fixed operation name,
  status or error category, and a coarse duration bucket. They exclude
  identifiers, sizes, IP addresses, credentials, URLs, payloads and raw
  exception text, and they are **deleted within seven days**, with no archive.
  None ship today.
- **The seven-day rule never applies to local capture.** Local capture keeps
  nothing across sessions at all, and its rules are the shorter ones.

**How the related beads fit:**
- **`personal-cfo-lyd`** builds §6–§8: the capture ring, admission, preview and
  save, and the UI.
- **`personal-cfo-ryjx`** proves the production path, capture → admission →
  preview → packaging → parse, drops every seeded sensitive value and accounts
  for eviction.
- **`personal-cfo-fps`** covers crash paths: app-owned crash and error sinks
  stay redacted, there is no automatic crash report, and after a crash the
  session capture is simply gone (§6.1).
- **`personal-cfo-3cw`** lists "redacted export" as one of its privacy modes.
  That mode **is** the §8 exporter, reused and not a second path. 3cw's other
  modes (hide balances, blur, copy-safe) are display features for the main UI.
  They are not part of this policy and add no capture or export.

## 10. What ships today

| Surface | State |
|---|---|
| Rust logs | Redacted `tracing` output to **stdout only**. The app writes no log file. |
| Local capture ring | **Ships** (`personal-cfo-lyd`): `observability::diagnostics::Diagnostics`, owned by the unlocked `Kernel` (`Kernel::diagnostics`), so lock, switch and exit drop it. Sources today are durable-job durations (`job_duration`, timed around each execution), backup outcomes (`backup_outcome`, success or a fixed failure category), and **successful** restores (`restore_outcome`, recorded as the restored vault's first record; a failed restore has no session to record in). The other §7 metrics are wired by their per-area structured-logging beads. |
| Diagnostic bundle export | **Ships** (`lyd`): Settings → Diagnostics → Preview → Save. IPC: `diagnostics_preview` and `diagnostics_discard` (general set), and `diagnostics_save` (destructive set: `main` only). Save writes exactly the previewed bytes to an absolute `.json` path **outside the app-data and active-vault folders**. The parent is resolved with `..` and symlinks followed, and a symlinked target is refused. Save returns a fixed result. The bundle format is `dohflow-diagnostics` v1, parsed by `observability::diagnostics::parse_bundle`. The adversarial round-trip suite (`personal-cfo-ryjx`) is `apps/desktop/src-tauri/tests/diagnostics_export_roundtrip.rs`, run by CI's desktop `cargo test` job: a seeded synthetic corpus through real vault operations must not reach the preview, the saved file or any other artifact; every key and string must come from §7's closed sets (listed in the suite independently of the Rust enums); retained records match the approved `insta` snapshots; eviction and rejection are counted exactly. |
| Crash reporting | **None** (`personal-cfo-fps` covers crash-path redaction; ADR 0066-A rules out automatic crash reports) |
| Network egress for any of the above | **None**. The WebView CSP allows no remote origin (ADR 0010, `csp-egress`), and no Rust-side telemetry client exists. |
| WebView console | Not persisted or collected anywhere. But it sits **outside** the redactor: frontend `console.*` calls (failed IPC calls, a refused external link) must follow §1 at the call site. Logging a user-entered value or a financial field there is a policy violation. |

## 11. Changing logging

In the same PR as the change:

- **A new log field or span** follows §1 and §3.1. If it could carry user data,
  it is redacted at the call site (§2.1).
- **A new sink** routes through the one redactor (§2.2) and gets a
  known-positive round-trip test. It must fit §5's five kinds; a new kind of
  record is an ADR change first. Anything leaving the device follows §8, and ADR
  0066-A requires explicit approval **before** collection for any new egress or
  service log field.
- **A new capture metric or field** changes §7's table and is reviewed first,
  and extends the allowlist in `diagnostics_export_roundtrip.rs` in the same PR
  (that suite fails on anything outside it). Changing §6's limits updates this
  doc and `lyd`'s tests together.
- **A new sensitive class or secret format** extends both corpora (§4) and the
  pattern table (§2.2).
- **This document** is updated. Security-sensitive changes follow the full review
  path (AGENTS.md §16).

## 12. Related

- `docs/adr/0066-A-dohflow-endpoints-pinned-credential-not-unlock.md` §5: no
  client telemetry, the single diagnostic export, and service-log retention.
- `docs/adr/0074-dohflow-sync-architecture.md`: protocol records.
- `docs/security/threat-model.md`, row "Financial data leaks into logs".
- `docs/security/capability-audit-2026-09-27.md` and
  `docs/security/tauri-regression-suite.md`: the WebView egress controls.
- `docs/architecture/definition-of-done.md` §2: logging obligations per feature.
