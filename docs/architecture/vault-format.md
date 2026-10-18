# Vault format

This is the public format authority for DohFlow data. It has two kinds of
content, kept apart on purpose:

- **Part A** describes the formats the application writes and reads today: the
  local vault, its key hierarchy, the schema, and the `.pcfobk` backup.
- **Part B** specifies the portable export and the Sync genesis snapshot, which
  are **not implemented**. Part B fixes what each format carries.

# Part A — Local vault and backup (shipped)

Part A describes only formats that the current application writes or reads. Do
not treat a SQLCipher database image or a `.pcfobk` backup as a Sync snapshot or
as a portable export.

The version pins below are checked against production code by
`crates/db-worker/tests/vault_format_spec.rs` and the backup integration tests.
Changing a persisted schema or envelope without updating this specification
fails CI. The [table-class manifest](../../crates/db-worker/table_classes.toml)
and its real-SQLCipher test are the exhaustive table and field inventory; this
document defines the meaning and compatibility of those bytes.

<!-- vault-format: spec-version=2 -->
<!-- vault-format: schema-version=55 -->
<!-- vault-format: envelope-version=1 -->
<!-- vault-format: backup-write-version=2 -->
<!-- vault-format: backup-read-versions=1,2 -->
<!-- vault-format: backup-manifest-version=1 -->
<!-- vault-format: engine-sqlcipher=4.14.0 community -->
<!-- vault-format: engine-sqlite=3.51.3 -->

## Local vault files and key hierarchy

The app stores a SQLCipher-encrypted `vault.db`, its plaintext
`vault.db.envelope` sidecar, and separately encrypted attachment blobs in
`blobs/` ([ADR 0002](../adr/0002-local-encrypted-vault.md),
[ADR 0023](../adr/0023-encrypted-attachment-store.md)). The sidecar contains
public KDF parameters and the wrapped vault key, not financial records or a
password. Blob filenames are opaque keyed storage IDs; original filenames and
other document metadata live inside SQLCipher. A raw copy of these files is not
the supported backup/export procedure.

The user's password and the sidecar's 16-byte salt/Argon2id parameters derive
a 256-bit key-encrypting key (KEK). AES-256-GCM unwraps a random 256-bit vault
data-encrypting key (DEK); an incorrect password fails authentication. The DEK
keys SQLCipher directly. Each attachment has its own random content key,
wrapped by the vault DEK. The password and KEK are never persisted. Password
change rewraps the DEK rather than rewriting the whole database. The current
creation profile is Argon2id with 65,536 KiB memory, time cost 3, parallelism
1, and parameter-shape version 1; the serialized values, not a presumed
profile, control unlock. The envelope is authoritative: the `kdf_*` columns in
`vault_metadata` are a creation-time stamp from build constants, are not kept in
sync with the envelope (a password change or key rotation does not update them),
and are never used to open a vault. See [encryption design](../security/encryption-design.md)
for the supported profile floor and engine pin.

**Key rotation** ([ADR 0083](../adr/0083-vault-key-rotation.md)) replaces the
DEK itself, unlike a password change, which only re-wraps the same DEK. A
rotation re-encrypts the database into a sibling file, re-wraps every
attachment's content key under the new DEK, re-addresses every blob (below),
writes a new envelope with a fresh salt at the current Argon2id profile, and
commits with one atomic rename of a journal. The vault's *meaning* is unchanged
by a rotation; every byte of `vault.db`, the sidecar and the blob names is
not. Rotation does not touch backups, which carry their own envelope and stay
openable with the password that was current when each was made.

**Attachment addressing.** A blob's name is the hex of
`HMAC-SHA256(addr_subkey, plaintext)`, where `addr_subkey` is
`HKDF-SHA256(DEK, salt = none, info = "personal-cfo/attachment-addressing/v1")`
(`crates/vault-crypto/src/attachment.rs`). Names are therefore deterministic
per vault and DEK, which gives local deduplication, and they change when the
DEK changes. A storage ID is a local, DEK-bound address: it is never a portable
or service-facing identifier.

### Principal files beside the vault

| File | Meaning |
| --- | --- |
| `vault.db` | SQLCipher database, keyed with the raw DEK |
| `vault.db.envelope` | plaintext sidecar (layout below) |
| `blobs/` | encrypted attachment blobs under opaque storage IDs |
| `vault.db.runner.lock` | advisory exclusive lock held while a vault is unlocked; a second opener fails with a typed vault-in-use error |
| `vault.db-wal`, `vault.db-shm`, `vault.db-journal` | SQLite side files: the WAL files while the vault is open, and a rollback journal if the database briefly uses that mode |
| `vault.db.rekey`, `vault.db.rekey.tmp`, `vault.db.rekey-new`, `vault.db.envelope.rekey-new`, `vault.db.rekey-old` | transient files of an in-progress or interrupted key rotation: the journal (`preparing`, `prepared`, `committed`), its temp file, the re-encrypted copy, the new envelope and the old database |

A rotation also leaves the same `-wal`, `-shm` and `-journal` side files beside
`vault.db.rekey-new` and `vault.db.rekey-old` while they exist, and a transient
`blobs/rekey-probe.tmp` file from its hard-link probe; recovery removes strays
of all of these. This table lists the principal files, not every transient name.

A rotation journal present at startup is finished (`committed`) or undone
(`preparing`, `prepared`) before the vault is classified; contradictory files
are reported as needing recovery and nothing is deleted
([ADR 0083](../adr/0083-vault-key-rotation.md) §4). Vault deletion removes this file
family too.

### Envelope byte layout (v1)

`crates/vault-crypto/src/envelope.rs` is the encoder/parser. Integers are
unsigned, big-endian; lengths count bytes. The fields are contiguous with no
padding or trailing bytes:

| Order | Field | Width |
| --- | --- | --- |
| 1 | ASCII magic `PCFOVLT` | 7 bytes |
| 2 | envelope format version `1` | u16 |
| 3 | KDF algorithm ID `1` = Argon2id | u8 |
| 4 | Argon2 memory KiB, time cost, parallelism, parameter-shape version | four u32 values |
| 5 | per-vault salt | 16 bytes |
| 6 | AES-256-GCM DEK-wrap nonce | 12 bytes |
| 7 | wrapped-DEK ciphertext length, then ciphertext including GCM tag | u32 + that many bytes |

There is no separate AAD in the current DEK-wrap call. Each wrap samples a
fresh random 96-bit nonce. Unknown envelope versions, algorithm IDs, malformed
lengths, and trailing bytes are rejected rather than guessed. An envelope
layout change requires a new envelope version and a new test vector; merely
changing Argon2 *values* stored in existing fields does not.

The deterministic **serialization-only** vector below uses a synthetic
three-byte wrapped ciphertext, so it is not a decryptable vault or a crypto
known-answer test. It pins field order and widths. Salt is sixteen `00` bytes;
nonce is twelve `11` bytes; memory is 65,536 KiB, time 3, parallelism 1,
parameter version 1, and ciphertext is `aa bb cc`.

<!-- vault-format: envelope-v1-vector=5043464f564c54000101000100000000000300000001000000010000000000000000000000000000000011111111111111111111111100000003aabbcc -->

## SQLCipher schema and migrations

The current fully migrated schema is the version pinned by the `schema-version`
marker above. The ordered migrations
in `crates/db-worker/src/migrations.rs` start from a consolidated v1 baseline;
subsequent versions are actual forward changes. `DbWorker::open` applies them
before exposing a usable worker. Each migration runs transactionally, records
its SQL content hash in `schema_migrations`, and advances SQLite
`PRAGMA user_version`. `vault_metadata.schema_version` advances on older
vaults; it is not lowered when a stored value is newer.

**Opening an existing vault fails closed on a layout this build does not
know** (`personal-cfo-g3m.4`). Before any writable setup, migration or unlock
side effect, the app inspects three independent version witnesses read-only:
SQLite `PRAGMA user_version`, the versions recorded in `schema_migrations`, and
`vault_metadata.schema_version`. If the highest of them exceeds the schema this
build knows, the open is refused with a typed newer-schema error carrying only
the observed and supported numbers. A missing or unreadable witness, a recorded
version this build has no migration for, a recorded migration hash that differs
from this build's, or a non-contiguous applied set is refused as an unsupported
layout. The inspection runs again on the writer handle after the file identity
(device and inode) is re-checked, with the runner lock held throughout. A
missing witness never hides newer evidence in another. An older build therefore
refuses a newer vault; it does not open it and does not stamp the schema
version downward.
The migration tests require contiguous versions, reversible post-baseline
migrations, and upgrade/rollback behavior; the table-class test compares the
freshly migrated `sqlite_master` set and selected `PRAGMA table_info` fields
to the manifest. A new migration bumps the schema pin above and requires
review of this document and the manifest; a migration must not silently
reinterpret an old column.

The [CLASS-0 manifest](../../crates/db-worker/table_classes.toml) names each
table, its owning bead, and its class. Class 1 holds canonical user intent;
1b holds document/import artifacts; 2 is rebuildable derived state; 3 is
device-local operational state; 4 is not yet assigned a final Sync route.
These are **Sync planning classifications**, not a claim that every class-1
table is already synced or that class-4 user data may be silently omitted from
a future portable export. `attachments` and `attachment_links` are currently
class 4; the manifest splits their user metadata from local blob keys, storage
IDs, and paths. Settings still include documented legacy unprefixed keys.

### Canonical state and audit history

The vault is a hybrid model: normalized domain tables are current financial
truth, and `operation_log` records successful command history in the same
SQLCipher transaction ([ADR 0011](../adr/0011-hybrid-ledger-operation-log.md)).
`audit_events` is a separate append-only security/event table. The current
operation-log schema and implementation, not ADR 0011's early illustrative
column sketch, govern stored fields. In particular,
[ADR 0017](../adr/0017-sync-identity-and-clock.md) clarifies that the persisted
`hlc_timestamp` is an advisory per-process monotonic counter, **not** a
distributed hybrid logical clock or Sync ordering authority. Pre-Sync command
history is audit evidence, not a replayable Sync tail. The detailed operation
log/projection lifecycle is separately owned by `personal-cfo-822b` and will
cross-link this authority.

## Encrypted `.pcfobk` backup (not portable logical export)

The app writes backup container **v2** and still reads historical **v1**
([ADR 0024](../adr/0024-encrypted-backup-restore-format.md), Addendum A).
This is an encrypted copy of the existing vault artifacts, not a logical JSON
dump. Because it contains the whole database, it includes device-local secrets
such as stored connector credentials; the portable export and Sync formats
exclude them by construction. Manual and scheduled export run only while the vault is unlocked and do
not request or retain a second password; the original vault password opens
the backup later.

`crates/finance-kernel/src/backup.rs` writes v2 as one file. The plaintext
header, in order, is ASCII `PCFOBK` (6 bytes), `format_version = u16(2)`,
`vault_envelope_len = u32` followed by the v1 envelope bytes, a fresh 16-byte
HKDF salt, a 12-byte wrapped-backup-DEK nonce, a u32 length and wrapped key
ciphertext, a 12-byte payload nonce, then a u64 length and AES-256-GCM payload
ciphertext. All lengths are big-endian. The outer payload contains a
length-framed JSON manifest, a byte-identical envelope copy, the checkpointed
SQLCipher `vault.db`, then a count and framed ciphertext blob entries. The
manifest schema version is **1**; it records app/schema/envelope versions and
SHA-256 checks of each *ciphertext* component. Unknown v2 manifest fields or
versions fail closed. The header envelope must match the authenticated payload
copy byte-for-byte before restore continues.

V2 derives a distinct backup KEK from the unlocked vault DEK via HKDF-SHA256
with the fresh header salt and ASCII info `dohflow-backup-kek-v2`; it wraps a
random backup DEK, which seals the payload with AES-256-GCM. On restore, the
password first unlocks the header vault envelope, then the recovered DEK
derives that backup KEK. Historical v1 instead derives the backup KEK directly
from the supplied password and the v1 header's Argon2id salt/parameters. Both
versions validate KDF parameters before expensive derivation and reject a
wrong password, corrupted ciphertext, or unsupported format. The checked-in
synthetic v1 fixture and v2 round-trip tests keep the read path covered.

Each backup is its own key epoch: it carries the envelope that existed when it
was made, so a later key rotation or password change does not alter an earlier
backup, and opening it needs the password that was current then. Restore
verifies the manifest and all component hashes before installing a
fresh vault; it never overwrites an existing vault. The app's named-vault
restore route preserves the original vault and reports interrupted, unregistered
staging for diagnosis ([recovery guide](../user-guide/recover-a-vault.md)).
The backup currently needs DohFlow's restore implementation. Do **not** claim
that a user can decrypt this backup without a DohFlow binary or recover from a
lost password. The independent recovery tool/procedure for the future portable
logical export is a `personal-cfo-vn91` deliverable and must pass its separate
synthetic no-app recovery test before any such public claim.

## Compatibility and change control

- `spec-version = 2` adds Part B, the planned portable and Sync content
  contract, to the shipped formats of Part A. New wire sections require a
  spec-version bump and their own reviewed vectors.
- Envelope v1 is read/written today; a future layout must use a new envelope
  version and retain an explicit reader/migration policy.
- Backup v2 is written; backup v1 and v2 are read. The restore path refuses a
  newer backup schema before installation, and opening an existing live vault
  refuses a newer or unrecognized schema as described above. No version of the
  app promises to open a vault written by a later one.
- The SQLCipher/SQLite engine pair is pinned in
  [stack.md](stack.md) (SQLCipher 4.14.0 community, SQLite 3.51.3, which carry
  the WAL-reset fix from `personal-cfo-g3m.5`), mirrored by the markers above,
  and exercised by the cross-version vault test. A compatible reader must honor
  the exact engine and migration policy, not just parse the outer envelope.
- No real financial records, vault paths, credentials, or keys appear in this
  specification or its vectors.

# Part B — Portable export and Sync genesis (specified, not implemented)

Nothing in this part is written or read by the shipped application. It is the
contract that `personal-cfo-vn91` (portable export and import) and
`personal-cfo-1cltt` (Sync genesis snapshot) implement. Owner approval of the
separation below is recorded on `personal-cfo-klr.4` (2026-09-27). The outer
encryption framing and the Sync wire formats are specified in Part C; this part
fixes *what* is carried and *what the importer promises*.

## B.1 Three formats, three jobs

DohFlow has three distinct data formats. They are not interchangeable, and none
is derived by post-processing another.

| | `.pcfobk` backup | Portable export | Sync genesis snapshot |
| --- | --- | --- | --- |
| Job | restore the same vault on this or another machine | personal migration and data ownership | bootstrap a Sync replica |
| Status | shipped (v2 written, v1 read) | specified, not implemented | specified, not implemented |
| Content | encrypted copy of `vault.db`, its envelope and blobs | logical JSON records, document bytes, CSV, recovery instructions | class-1 logical rows plus the class-1b artifact closure |
| Protected by | the vault password (through the vault DEK; v1 directly) | the vault password and no other secret | Sync epoch keys ([ADR 0074](../adr/0074-dohflow-sync-architecture.md)); never the password |
| Carries credentials and device keys | credentials yes (whole database) | no | no |
| Needs a DohFlow binary to read | yes | no: the independent recovery tool (B.6) | yes |
| Historical records | the whole database | read-only imported history | none: state, not history |

Giving another person a portable bundle and its password exposes the vault
password. The bundle is for the owner's own migration, not for sharing.

## B.2 Inclusion and exclusion inventory

The [CLASS-0 manifest](../../crates/db-worker/table_classes.toml) assigns every
table a Sync class. This inventory states, for **every** manifest row, what the
portable export and the Sync genesis do with it, so nothing is omitted under the
word "Everything". Dispositions:

- `include`: the rows travel as data.
- `fields`: only the named fields travel; the rest are named as excluded.
- `history`: the rows travel as read-only imported history (portable only).
- `artifact`: bytes travel as artifacts (B.3); under Sync, as opaque, epoch-scoped
  objects under the closure rule in ADR 0074 Decision 1.
- `rebuild`: not shipped; rebuilt locally from the shipped rows.
- `exclude`: never leaves the vault.

A manifest row with no line here, a line with no manifest row, a class that
disagrees with the manifest, or a disposition outside this list fails
`crates/db-worker/tests/portable_inventory_spec.rs`. Reclassifying a table
therefore requires reviewing this inventory.

<!-- inventory:start -->
| Table | Class | Portable | Sync genesis | Rule |
| --- | --- | --- | --- | --- |
| `operation_log` | 1 | history | exclude | Portable export carries the rows as read-only imported history, never as a replayable stream. The Sync genesis carries resulting state, not pre-v2 history; the tail carries envelopes issued after op-log v2. |
| `ledger_accounts` | 1 | include | include | Canonical user intent. |
| `accounts` | 1 | include | include | Canonical user intent. |
| `ledger_transactions` | 1 | include | include | Canonical user intent. |
| `ledger_postings` | 1 | include | include | Canonical user intent. |
| `provenance_links` | 1 | history | exclude | Travels with the history it indexes, on the same terms as `operation_log`. |
| `categories` | 1 | include | include | Canonical user intent. |
| `category_aliases` | 1 | include | include | Canonical user intent. |
| `income_sources` | 1 | include | include | Canonical user intent. |
| `recurring_events` | 1 | include | include | Canonical user intent. |
| `recurring_event_instances` | 1 | include | include | Canonical user intent. |
| `recurring_event_tags` | 1 | include | include | Canonical user intent. |
| `recurring_transfers` | 1 | include | include | Canonical user intent. |
| `bill_contracts` | 1 | include | include | Canonical user intent. |
| `confirmed_obligations` | 1 | include | include | Canonical user intent. |
| `transaction_details` | 1 | include | include | Canonical user intent. |
| `transaction_reviews` | 1 | include | include | Canonical user intent. |
| `tags` | 1 | include | include | Canonical user intent. |
| `transaction_tags` | 1 | include | include | Canonical user intent. |
| `split_lines` | 1 | include | include | Canonical user intent. |
| `split_line_tags` | 1 | include | include | Canonical user intent. |
| `debt_terms` | 1 | include | include | Canonical user intent. |
| `credit_card_cycles` | 1 | include | include | Canonical user intent. |
| `credit_card_statements` | 1 | include | include | Canonical user intent. |
| `change_journal_entries` | 1 | include | include | Canonical user intent. |
| `attachments` | 4 | fields | fields | `id`, `plaintext_size`, `mime_type`, `original_filename` and `created_at` travel; `content_alg` and `ref_count` are rebuilt; `storage_id`, `wrapped_content_key`, `content_key_nonce` and `content_nonce` never travel. |
| `attachment_links` | 4 | include | include | Class-4 group routed to class 1 by ADR 0074 Decision 5. |
| `source_batches` | 1b | include | artifact | Import provenance rows travel as data in the portable export; Sync carries them as opaque artifacts under the closure rule. |
| `source_records` | 1b | include | artifact | Import provenance rows travel as data in the portable export; Sync carries them as opaque artifacts under the closure rule. |
| `parser_runs` | 1b | include | artifact | Import provenance rows travel as data in the portable export; Sync carries them as opaque artifacts under the closure rule. |
| `source_provenance_links` | 1b | include | artifact | Import provenance rows travel as data in the portable export; Sync carries them as opaque artifacts under the closure rule. |
| `commitments` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `transaction_display_rows_read_model` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `read_model_checksums` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `forecast_input_snapshots` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `forecast_runs` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `forecast_rows` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `forecast_actuals` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `forecast_diffs` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `forecast_dirty_ranges` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `forecast_quality_scores` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `forecast_backtest_results` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `money_inbox_read_model` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `model_registry` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `risk_flags` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `recurring_suggestion_suppressions` | 2 | rebuild | rebuild | Derived; rebuilt locally after import or bootstrap, never shipped. |
| `command_idempotency_keys` | 3 | exclude | exclude | Device-local state. |
| `vault_meta` | 3 | exclude | exclude | Device-local state. |
| `vault_metadata` | 3 | fields | fields | Only `household_timezone` travels, in the manifest vault profile (portable) and the genesis header (Sync). `vault_id`, `schema_version`, `envelope_version`, `kdf_algorithm`, `kdf_memory_kib`, `kdf_time_cost`, `kdf_parallelism`, `manifest_pointer` and `created_at` never leave the vault; an imported vault gets its own. |
| `schema_migrations` | 3 | exclude | exclude | Device-local state. |
| `audit_events` | 3 | exclude | exclude | Device-local security evidence and replay-owned planned-command rows; not a user-owned financial record. |
| `settings` | 3 | fields | fields | Keys with the `user.` prefix and the six legacy unprefixed user keys travel; `device.*` keys never do. |
| `connector_connections` | 3 | exclude | exclude | Holds provider credentials. Excluded whole until the identity/secret split in `personal-cfo-07u` yields a replicable identity table. |
| `connector_account_links` | 3 | exclude | exclude | Holds local refresh watermarks. Excluded whole until the split in `personal-cfo-07u`. |
| `staged_accounts` | 3 | exclude | exclude | Device-local state. |
| `staged_balances` | 3 | exclude | exclude | Device-local state. |
| `staged_transactions` | 3 | exclude | exclude | Device-local state. |
| `system_ledger_accounts` | 3 | exclude | exclude | Device-local state. |
| `projection_cursors` | 3 | exclude | exclude | Device-local state. |
| `dedupe_decisions` | 4 | include | include | Class-4 group routed to class 1 by ADR 0074 Decision 5. |
| `balance_observations` | 4 | include | include | Class-4 group routed to class 1 by ADR 0074 Decision 5. |
| `scenarios` | 4 | include | include | Class-4 group routed to class 1 by ADR 0074 Decision 5. |
| `forecast_assumption_events` | 4 | include | include | Class-4 group routed to class 1 by ADR 0074 Decision 5. |
| `forecast_dependency_edges` | 4 | include | include | Class-4 group routed to class 1 by ADR 0074 Decision 5. |
| `merchant_aliases` | 4 | include | include | Class-4 group routed to class 1 by ADR 0074 Decision 5. |
| `merchant_identities` | 4 | include | include | Class-4 group routed to class 1 by ADR 0074 Decision 5. |
| `manual_entry_links` | 4 | include | include | Class-4 group routed to class 1 by ADR 0074 Decision 5. |
| `transaction_categorizations` | 4 | include | include | Class-4 group routed to class 1 by ADR 0074 Decision 5. |
| `settings_user_keys` | 4 | fields | fields | Key-scope marker for the `user.*` keys carried from `settings`. |
| `attachment_blobs` | 1b | artifact | artifact | Document bytes: plaintext inside the portable bundle; an opaque, epoch-scoped artifact object under Sync. |
| `durable_jobs` | 3 | exclude | exclude | Device-local state. |
| `backup_history` | 3 | exclude | exclude | Device-local state. |
<!-- inventory:end -->

Rules that apply to the whole inventory:

1. **Credentials, device keys, endpoint overrides, caches and runtime queues
   never travel in either format.** Provider credentials and refresh watermarks
   (`connector_connections`, `connector_account_links`), idempotency memos,
   durable jobs, staged import rows, backup receipts and the vault's
   key-derivation parameters are excluded by the rows above.
2. **No raw database image and no recovery key or packet** is part of the
   portable export. Emergency-access recovery is separate future work
   (`personal-cfo-6zf`).
3. **A document's wrapped content key, content nonce and local storage ID are
   never exported.** They are bound to the vault DEK that wrote them. The
   importer re-encrypts the document bytes under the new vault's DEK and a
   fresh content key (B.4).
4. **The nine intent groups ADR 0074 Decision 5 promotes from class 4 to class
   1 are exported as class 1.** The manifest still lists them as class 4 until
   `personal-cfo-5ymg8` routes their writes; the inventory follows the
   decision, so the portable export is complete before that routing lands.
5. **Pre-Sync operation history is audit evidence.** It is portable as history
   and is never presented as a replayable command stream or carried into a
   Sync genesis ([ADR 0017](../adr/0017-sync-identity-and-clock.md)).

## B.3 Portable logical model

The portable export is a logical bundle. Its encrypted container is specified
in Part C; this section defines the logical contents and is independent of the
container.

### Contents

| Logical path | Meaning |
| --- | --- |
| `manifest.json` | format identity and version, producer versions, the vault profile, per-table row counts and hashes, the artifact list, and the inventory version |
| `data/<table>.jsonl` | one file per table with disposition `include`, `fields` or `history`; one JSON object per line, in primary-key order |
| `artifacts/<artifact_id>` | the plaintext bytes of one document, addressed by the attachment's logical `id` |
| `exports/transactions.csv` | the existing plaintext transaction export (`personal-cfo-hbd8`), included unchanged |
| `RECOVERY.txt` | plain-language instructions for opening the bundle without DohFlow (B.6) |

`manifest.json` carries, at minimum:

- `format`, the constant `dohflow-portable-export`, and `format_version`, an
  integer that is `1` for this revision;
- the producing app version and `schema_version`;
- the vault profile: `household_timezone` and nothing else from
  `vault_metadata`;
- the inventory version;
- a `tables` array: name, class, row count, the SHA-256 of the file's bytes, and
  a `columns` array giving each column's name and kind;
- an `artifacts` array: the document's logical `id`, byte length, SHA-256 of the
  plaintext, `mime_type` and `original_filename`.

The `artifacts` array is the **complete artifact manifest**. An importer
refuses a bundle whose list names a document that is absent, or that contains a
document the list does not name.

### Row encoding

Each line of a `data/<table>.jsonl` file is one JSON object keyed by the
table's column names. The manifest's `columns` array gives each column a kind
that fixes its encoding, so a reader needs no knowledge of the SQL schema:

- `int`: a JSON integer, read as a signed 64-bit value.
- `text`: a JSON string.
- `uuid`: a lowercase hyphenated UUID string, for the 16-byte identifier columns.
- `bytes`: a standard base64 string, for any other binary column.
- a SQL `NULL` is JSON `null` in any kind.

Money is always its integer minor units plus a separate currency column; no
floating-point value appears in any exported record. Every file is UTF-8 with
`\n` line endings.

### Preserved, rebuilt and not preserved

Importing a bundle into a fresh vault **preserves**:

- every primary and foreign key of every exported table, so references hold;
- every `created_at` and other timestamp, and every user-entered field;
- imported history rows with their original identifiers and times, marked by
  the absence of an op-log-v2 payload, which is what makes them audit history.

It **rebuilds**, by the same projection code used after any import, every table
with disposition `rebuild`, so forecasts and the transaction display model are
derived from the imported canonical rows rather than copied.

It does **not** preserve the source vault's identity: the importer mints a new
`vault_id`, `node_id`, data-encrypting key, salt and envelope. The new vault's
password is chosen at import and may equal the bundle password; the importer
never assumes it does.

### Import and export semantics

- **Fresh vault only.** An import creates a new named, app-managed vault through
  the existing vault lifecycle. It never imports over an existing vault and
  never modifies the bundle.
- **Verify, then register.** The importer authenticates the container, then
  checks every table file and every artifact against the manifest hashes before
  the new vault is registered or activated. A failure at any point removes the
  new vault's files and leaves no usable partial vault.
- **No plaintext temporary files.** Neither exporter nor importer writes
  decrypted data to disk outside the user-selected destination or the new
  encrypted vault. Decryption streams into memory and the new vault.
- **Atomic, non-overwriting publication.** The exporter writes the encrypted
  bundle to an owned temporary name beside the destination, flushes it, and
  renames it into place only when complete. It never replaces an existing
  destination, and it removes only the partial file it created.
- **Source vault unchanged.** Export reads a consistent snapshot; neither
  operation modifies the source vault.

### Typed failures

Every failure below is a distinct, typed outcome and never a partial success.

| Outcome | When |
| --- | --- |
| `WrongPassword` | the container does not authenticate under the supplied password |
| `Corrupt` | the container, a table file or an artifact fails its integrity check |
| `UnsupportedVersion` | `format_version` is below the oldest version this reader supports |
| `NewerFormat` | `format_version` or the producer's `schema_version` is newer than this reader knows |
| `MissingArtifact` | the artifact list and the bundle disagree |
| `DestinationExists` | the export destination already exists |
| `DiskFull` | the destination or the new vault cannot be written |
| `Cancelled` | the user cancelled; owned partial output is removed |
| `Interrupted` | the process ended early; no success is reported and owned partial output is removed on the next run |

### Compatibility

A reader supports an explicit, documented range of `format_version` values and
refuses the rest; DohFlow does not promise that every future version opens. A
bundle with an unknown field at a supported version is `Corrupt`. A column
absent from an older bundle takes the importing schema's default; a column the
importer's schema does not know is `NewerFormat`.

## B.4 Documents

An attachment's bytes are exported as plaintext inside the encrypted bundle and
are verified against the manifest's plaintext SHA-256. On import, the importer
derives the new vault's storage ID, generates a fresh content key, encrypts the
bytes, wraps that key under the new DEK, and writes the new blob and row. No
exported value is a local storage ID or key material. The same receiver-side
rule governs the Sync genesis (ADR 0074 Decision 1).

## B.5 What the Sync genesis reuses

The genesis snapshot reuses only the logical model above, restricted to the
rows with a Sync disposition of `include`, `fields` or `artifact`. It does not
reuse the user-facing password wrapper, the history rows or the plaintext CSV
and instructions. Its wire encoding is in Part C.

## B.6 Independent recovery

The portable export is only a recovery path if it can be opened without
DohFlow. The independent recovery tool, working name `dohflow-recover`, is
specified here and delivered and verified by `personal-cfo-vn91`:

- it takes a bundle and the vault password and writes the logical contents of
  B.3 into a directory the user selects, and writes nothing elsewhere;
- it depends on the published container specification (Part C) and its test
  vectors, not on the vault, the database or any DohFlow crate;
- it reports the typed outcomes above and a non-zero exit status on failure;
- it ships with a procedure a person can follow with the format specification
  alone.

DohFlow makes **no** claim that a bundle can be opened without DohFlow until a
synthetic no-application recovery test passes in `personal-cfo-vn91`.
