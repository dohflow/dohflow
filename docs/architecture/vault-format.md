# Vault format

This is the public format authority for DohFlow data. It has two kinds of
content, kept apart on purpose:

- **Part A** describes the formats the application writes and reads today: the
  local vault, its key hierarchy, the schema, and the `.pcfobk` backup.
- **Part B** specifies what the portable export and the Sync genesis snapshot
  carry. Both are **not implemented**.
- **Part C** specifies the cryptographic framing, the canonical encodings and the
  Sync wire formats behind them. It is **not implemented** either.

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

<!-- vault-format: spec-version=10 -->
<!-- vault-format: schema-version=59 -->
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

While a rotation runs, SQLite can leave `-wal`, `-shm` and `-journal` side files
beside `vault.db.rekey-new`, and the hard-link probe leaves a transient
`blobs/rekey-probe.tmp`. Rolling back an unfinished rotation removes the copy,
its side files, the probe and the new envelope. `vault.db.rekey-old` is only
renamed into place and later removed; SQLite never opens it. This table lists
the principal files, not every transient name.

A rotation journal present at startup is finished (`committed`) or undone
(`preparing`, `prepared`) before the vault is classified; contradictory files
are reported as needing recovery and nothing is deleted
([ADR 0083](../adr/0083-vault-key-rotation.md) §4). Vault deletion removes every
name in this file family, including the rotation side-file names.

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

- `spec-version = 2` adds Parts B and C, the planned portable and Sync content
  contract and wire formats, to the shipped formats of Part A.
- `spec-version = 3` tightens C.1.2 (canonical JSON) before any implementation
  shipped it: the integer grammar excludes `-0`, strings exclude U+0000, nesting is at
  most 128 levels, a receiver accepts only the canonical encoding of the value it decodes
  to, and C.14 gains the rejection vectors. New wire sections require a
  spec-version bump and their own reviewed vectors.
- `spec-version = 4` adds C.1.5, the canonical row encoding (declared-kind tags, primary
  key, row and deleted-row digests, and folded children) that the write-set hash of C.4
  and the replay-property hash refer to, with known-answer vectors.
- `spec-version = 5` defines `engine_version` (C.13): what the value means, who writes it
  and when it changes. No wire layout or vector changes; the vectors' `dohflow-0.3.0` stays
  an illustrative value.
- `spec-version = 6` makes two rules explicit before any implementation ships them, with no
  wire layout or vector change: C.4 receiver check 4 refuses a present manifest commitment
  for an empty delta manifest (a second valid encoding of one envelope), and C.13 freezes the
  prefix of every clear header and container so a newer protocol is reported as unsupported,
  not as a corrupt object.
- `spec-version = 7` makes one more rule explicit, with no wire layout or vector change: a
  present `result_ref` in the accepted-envelope record (C.10) is 1 to 64 bytes, because an
  empty one would be a second encoding of an absent one.
- `spec-version = 8` pins the exact key and signature acceptance rules of C.6 (strict Ed25519
  checks stated as checks, a screen for low-order X25519 keys), adds chain rules 8 to 11 (a
  device id is never reused; a revoke never leaves zero authorized members; at most 1,024
  authorized members; no two members share a public key), requires Ed25519 keys and signature
  `R` to lie in the prime-order subgroup (closing mixed-order points), fixes the check order,
  makes a conflicting record a fork only if it would itself have been valid, makes the recipient list canonical
  (ascending by fingerprint, with exact lengths), and requires every cutover and rotation to
  publish a key commitment never used before in the chain. C.14 gains the public membership-chain
  conformance vectors (`docs/architecture/vectors/membership-chain-v1.json`). No existing wire
  layout or vector changes.
- `spec-version = 9` adds the push and producer rules of C.4 (what the service MUST check at the
  door, and that a client MUST run the receiver checks on the exact bytes it uploads), binds the
  receiver-check rule set to `engine_version`, and adds the typed failure
  `AuthenticatedEnvelopeInvalid` to C.11. No wire layout or vector changes.
- `spec-version = 10` adds the void record (C.10 "Void records"), the member-signed record by
  which a vault recovers from a poisoned log position (ADR 0074 Addendum D), with its encoding,
  who may sign it, when a replica honors it, and the index marking; and the vectors `void_body`
  and `void_id` in C.14. No existing wire layout or vector changes.
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
| `vault_metadata` | 3 | fields | fields | Only `household_timezone` travels, as the logical class-1 field of the `household` record (B.3); it is stored here today and may move to the households row (`personal-cfo-339`) without a format change. `vault_id`, `schema_version`, `envelope_version`, `kdf_algorithm`, `kdf_memory_kib`, `kdf_time_cost`, `kdf_parallelism`, `manifest_pointer` and `created_at` never leave the vault; an imported vault gets its own. |
| `schema_migrations` | 3 | exclude | exclude | Device-local state. |
| `audit_events` | 3 | exclude | exclude | Device-local security evidence and replay-owned planned-command rows; not a user-owned financial record. |
| `settings` | 3 | fields | fields | Keys with the `user.` prefix and the six legacy unprefixed user keys travel; `device.*` keys never do. |
| `connector_connections` | 3 | exclude | exclude | Holds provider credentials. Excluded whole until the identity/secret split in `personal-cfo-07u` yields a replicable identity table. |
| `connector_account_links` | 3 | exclude | exclude | Holds local refresh watermarks. Excluded whole until the split in `personal-cfo-07u`. |
| `connector_attention` | 3 | exclude | exclude | Device-local re-authentication notices (personal-cfo-pxi.22); rebuilt by the next refresh. |
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
   Sync genesis ([ADR 0017](../adr/0017-sync-identity-and-clock.md)). This does
   **not** limit what a Sync device can see: the genesis carries every class-1
   ledger row of every age (C.8).
6. **The household timezone is user data, not a header.** `household_timezone`
   changes the forecast horizon boundary, so it is a class-1 logical field and is
   changed only through a logged command (`SetHouseholdTimezone`, a
   `WriteCommand` with an operation-log row and an audit event; the value is
   stored on the `vault_metadata` row today). It is never a manifest-only or
   genesis-header-only value, because a header would not propagate later changes.

## B.3 Portable logical model

The portable export is a logical bundle. Its encrypted container is specified
in C.12; this section defines the logical contents and is independent of the
container.

### Contents

| Logical path | Meaning |
| --- | --- |
| `manifest.json` | format identity and version, producer versions, the vault profile, per-table row counts and hashes, the artifact list, and the inventory version |
| `data/<table>.jsonl` | one file per table with disposition `include`, `fields` or `history`; one canonical JSON object per line, in primary-key order |
| `data/household.jsonl` | the logical `household` record: one row holding `household_timezone` (B.2 rule 6) |
| `artifacts/<artifact_id>` | the plaintext bytes of one document, addressed by the attachment's logical `id` |
| `exports/transactions.csv` | the existing plaintext transaction export (`personal-cfo-hbd8`), included unchanged |
| `RECOVERY.txt` | plain-language instructions for opening the bundle without DohFlow (B.6) |

`manifest.json` carries, at minimum:

- `format`, the constant `dohflow-portable-export`, and `format_version`, the
  decimal string `"1"` for this revision;
- `app_version` (text) and `schema_version` (a decimal string) of the producer;
- `inventory_version`, the decimal string `"1"` for the inventory of B.2;
- `object_count`, a decimal string equal to the container header's (C.12);
- `tables`: an array of objects with `name`, `class`, `row_count`, `sha256` (of
  the file's bytes) and `columns`, an array of objects with `name` and `kind`;
- `artifacts`: an array of objects with the document's logical `id`, `length`,
  `sha256` (of the plaintext), `mime_type` and `original_filename`;
- `objects`: an array of objects with `index`, `path`, `chunk`,
  `ciphertext_length` and `ciphertext_sha256`, one per object after the manifest
  (C.12).

`manifest.json` is canonical JSON (C.1.2): every integer in it, including counts
and lengths, is a decimal string.

The `artifacts` array is the **complete artifact manifest**. An importer
refuses a bundle whose list names a document that is absent, or that contains a
document the list does not name.

### Row encoding

Each line of a `data/<table>.jsonl` file is one JSON object keyed by the
table's column names. The manifest's `columns` array gives each column a kind
that fixes its encoding, so a reader needs no knowledge of the SQL schema:

- `int`: a decimal string for a signed 64-bit value, never a JSON number (C.1.2).
- `text`: a JSON string.
- `uuid`: a lowercase hyphenated UUID string, for the 16-byte identifier columns.
- `bytes`: a standard base64 string, for any other binary column.
- a SQL `NULL` is JSON `null` in any kind.

Money is always its integer minor units, as a decimal string, plus a separate
currency column; no floating-point value appears in any exported record. Every file is UTF-8 with
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
and instructions. Its wire encoding is in C.8.

**Full-history guarantee.** Every device that bootstraps from a genesis snapshot
can view all historical transactions, however old. The genesis carries every
class-1 ledger row of every age; only the pre-v2 edit audit trail
(`operation_log` and `provenance_links` history) is left out.

## B.6 Independent recovery

The portable export is only a recovery path if it can be opened without
DohFlow. The independent recovery tool, working name `dohflow-recover`, is
specified here and delivered and verified by `personal-cfo-vn91`:

- it takes a bundle and the vault password and writes the logical contents of
  B.3 into a directory the user selects, and writes nothing elsewhere;
- it depends on the published container specification (C.12) and its test
  vectors, not on the vault, the database or any DohFlow crate;
- it reports the typed outcomes above and a non-zero exit status on failure;
- it ships with a procedure a person can follow with the format specification
  alone.

DohFlow makes **no** claim that a bundle can be opened without DohFlow until a
synthetic no-application recovery test passes in `personal-cfo-vn91`.

# Part C — Cryptographic framing and Sync wire formats (specified, not implemented)

Nothing in this part is written or read by the shipped application. It fixes the
bytes behind the content contract of Part B and behind
[ADR 0074](../adr/0074-dohflow-sync-architecture.md), whose decisions it does not
change: ADR 0074 decides *what* Sync protects and how it behaves; this part
fixes the canonical encodings, derivations and records, as ADR 0074 and
[ADR 0017](../adr/0017-sync-identity-and-clock.md) delegate to
[ADR 0077](../adr/0077-wire-formats.md). The owner decisions it applies are
recorded on `personal-cfo-klr.4` (2026-10-18).

The byte-level rules here are checked by a test-only reference implementation,
`crates/vault-crypto/tests/sync_wire_vectors.rs`, which computes every vector in
C.14 from the rules below and rejects the malformed inputs listed there. Ed25519
signatures and HPKE sealing are **not** in that reference, because those
primitives are not workspace dependencies yet: this part pins every byte that
feeds them (signed bodies, HPKE `info`, AAD) and requires the implementing bead
(`personal-cfo-1cltt`) to add the RFC 8032 and RFC 9180 known-answer tests listed
in C.14.

## C.1 Conventions

### C.1.1 Canonical binary encoding (CBE1)

Every value that is hashed, signed, authenticated as AAD, or used as HKDF
`info` is encoded with one scheme, so there is exactly one byte string for each
logical value. One field table per structure drives both the encoder and the
parser.

| Kind | Encoding |
| --- | --- |
| `label` | `u16` length (1 to 64), then ASCII bytes from `[a-z0-9/_.-]`. Every structure starts with its own label, which is its domain separator. |
| `u8`, `u16`, `u32`, `u64` | fixed-width, big-endian |
| `i64` | fixed-width, big-endian, two's complement |
| `uuid` | 16 raw bytes (never text) |
| `digest`, `fingerprint`, `key32` | 32 raw bytes |
| `bytes(max)` | `u32` big-endian length no greater than `max`, then the bytes |
| `text(max)` | `bytes(max)` whose content is valid UTF-8 |
| `blob` | `u64` length, then the bytes; used only for large bodies with a stated bound |
| `opt(T)` | one presence byte, `0` for absent or `1` for present, then `T` if present; this is the canonical null marker |
| `list(T, max)` | `u32` count no greater than `max`, then the items |

A parser **must** reject: trailing bytes after the last field; a length above
its stated maximum; a presence byte other than `0` or `1`; invalid UTF-8 in a
`text`; an unknown or mismatched label; and truncation. It must not skip unknown
trailing fields. A receiver that supports a version rejects a structure of an
unsupported version rather than reading part of it.

### C.1.2 Canonical JSON (restricted RFC 8785)

Manifests and command payloads are JSON canonicalized under
[RFC 8785](https://www.rfc-editor.org/rfc/rfc8785.html) (JCS), restricted so
that JCS's IEEE-754 number rules never apply:

- **Every integer is a decimal string** (`"-1250"`, `"41"`), whatever its width, with
  syntax `0|-?[1-9][0-9]*` (zero is `"0"`, never `"-0"`, and there are no leading zeros,
  so every integer has exactly one spelling) and the range of its field. No JSON number
  appears in any hashed or committed document, and no floating-point value appears at
  all. Money is a minor-units string plus a currency string.
- Member names are ASCII `[a-z0-9_]`, sorted by UTF-16 code units (which is byte
  order for ASCII), with no insignificant whitespace.
- Strings use the JCS escaping rules. Invalid UTF-8, lone surrogate escapes and
  duplicate member names are rejected. No Unicode normalization is applied.
- A string contains no U+0000 (neither raw nor as `\u0000`), so a consumer in a language
  with NUL-terminated strings cannot truncate a document.
- Booleans, `null`, arrays, objects and strings are the only value types. Nesting is at
  most **128** levels of arrays and objects (the outermost container is level 1); a
  document nested deeper is refused on encoding and on parsing alike.
- A value that is *absent* and a value that is *present but null* are distinguishable
  only by the schema of the document: an encoder refuses a "some" whose content itself
  encodes as `null`, so no two distinct values share an encoding.
- **A receiver accepts a document only if its bytes equal the canonical encoding of the
  value it decodes to.** Accepting any other spelling of the same value (whitespace, a
  different member order, an alternative escape, an unknown member, `"-0"`) would let
  two byte strings carry one meaning. Hashes and signatures are always over the received
  bytes, never over a re-encoding of what was parsed. The documents below are well-formed
  text that a receiver must refuse; each is a rejection vector of C.14:

| Name | Document |
| --- | --- |
| `whitespace_between_members` | `{"a": "1"}` |
| `trailing_newline` | `{"a":"1"}\n` |
| `members_out_of_order` | `{"b":"1","a":"2"}` |
| `escaped_ordinary_character` | `{"a":"\u0041"}` |
| `escaped_solidus` | `{"a":"\/"}` |
| `uppercase_hex_escape` | `{"a":"\u00E9"}` |
| `escaped_non_ascii_character` | `{"a":"\u00e9"}` |
| `escaped_control_with_wrong_spelling` | `{"a":"\u000a"}` |
| `escaped_surrogate_pair` | `{"a":"\ud83d\ude00"}` |
| `a_json_number` | `{"a":1}` |
| `a_float` | `{"a":1.5}` |
| `duplicate_member` | `{"a":"1","a":"2"}` |
| `duplicate_member_escaped` | `{"a":"1","\u0061":"2"}` |
| `member_name_uppercase` | `{"A":"1"}` |
| `member_name_with_dot` | `{"a.b":"1"}` |
| `lone_high_surrogate` | `{"a":"\ud800"}` |
| `lone_low_surrogate` | `{"a":"\udc00"}` |
| `nul_escape` | `{"a":"\u0000"}` |
| `raw_control_character` | `{"a":"\n"}` |
| `leading_byte_order_mark` | `\ufeff{"a":"1"}` |
| `trailing_comma` | `{"a":"1",}` |
| `empty_member_name` | `{"":"1"}` |
| `invalid_utf8_in_a_string` | `{"a":"\xff"}` |
| `overlong_utf8_in_a_string` | `{"a":"\xc0\x80"}` |
| `nested_129_levels` | `[` x 129 then `]` x 129 |

<!-- cj-reject: whitespace_between_members=7b2261223a202231227d -->
<!-- cj-reject: trailing_newline=7b2261223a2231227d0a -->
<!-- cj-reject: members_out_of_order=7b2262223a2231222c2261223a2232227d -->
<!-- cj-reject: escaped_ordinary_character=7b2261223a225c7530303431227d -->
<!-- cj-reject: escaped_solidus=7b2261223a225c2f227d -->
<!-- cj-reject: uppercase_hex_escape=7b2261223a225c7530304539227d -->
<!-- cj-reject: escaped_non_ascii_character=7b2261223a225c7530306539227d -->
<!-- cj-reject: escaped_control_with_wrong_spelling=7b2261223a225c7530303061227d -->
<!-- cj-reject: escaped_surrogate_pair=7b2261223a225c75643833645c7564653030227d -->
<!-- cj-reject: a_json_number=7b2261223a317d -->
<!-- cj-reject: a_float=7b2261223a312e357d -->
<!-- cj-reject: duplicate_member=7b2261223a2231222c2261223a2232227d -->
<!-- cj-reject: duplicate_member_escaped=7b2261223a2231222c225c7530303631223a2232227d -->
<!-- cj-reject: member_name_uppercase=7b2241223a2231227d -->
<!-- cj-reject: member_name_with_dot=7b22612e62223a2231227d -->
<!-- cj-reject: lone_high_surrogate=7b2261223a225c7564383030227d -->
<!-- cj-reject: lone_low_surrogate=7b2261223a225c7564633030227d -->
<!-- cj-reject: nul_escape=7b2261223a225c7530303030227d -->
<!-- cj-reject: raw_control_character=7b2261223a220a227d -->
<!-- cj-reject: leading_byte_order_mark=efbbbf7b2261223a2231227d -->
<!-- cj-reject: trailing_comma=7b2261223a2231222c7d -->
<!-- cj-reject: empty_member_name=7b22223a2231227d -->
<!-- cj-reject: invalid_utf8_in_a_string=7b2261223a22ff227d -->
<!-- cj-reject: overlong_utf8_in_a_string=7b2261223a22c080227d -->
<!-- cj-reject: nested_129_levels=5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d -->

- Member names, when a map is encoded, follow the same `[a-z0-9_]` rule, so user data is
  never a map key (a key such as `user.theme` is refused), and an integer key is its
  decimal string (a negative key fails the name rule). Binary data in a payload is a
  base64 string newtype, never a byte sequence.
- Implementations pin one canonicalization crate and must produce identical
  bytes on the cross-implementation vectors of C.14.

### C.1.3 Registries

| Registry | Values |
| --- | --- |
| `protocol_version` | `1` |
| `suite_id` | `1`: see C.1.4 |
| `object_type` (AEAD objects) | `1` command envelope, `2` snapshot |
| `actor_type` | `1` user, `2` system, `3` importer, `4` agent |
| `membership_action` | `1` add, `2` revoke |
| `reason` (epoch record) | `1` genesis, `2` revocation cutover, `3` grant, `4` forced rotation |
| `kem_id` | `0x0020` DHKEM(X25519, HKDF-SHA256) |
| `artifact_kind` | `1` attachment, `2` source batch, `3` source record, `4` parser run |

### C.1.4 Cryptographic suite 1

| Role | Algorithm |
| --- | --- |
| Hash | SHA-256 |
| MAC | HMAC-SHA256 |
| Key derivation | HKDF-SHA256 in Expand-only use on a uniformly random 32-byte key ([RFC 5869](https://www.rfc-editor.org/rfc/rfc5869.html)) |
| AEAD | AES-256-GCM, 96-bit nonce, 128-bit tag |
| Key encapsulation | [HPKE](https://www.rfc-editor.org/rfc/rfc9180.html) base mode, DHKEM(X25519, HKDF-SHA256) `kem_id 0x0020`, HKDF-SHA256 `kdf_id 0x0001`, AES-256-GCM `aead_id 0x0002` |
| Signature | Ed25519 ([RFC 8032](https://www.rfc-editor.org/rfc/rfc8032.html)), always verified with strict verification (`verify_strict` semantics: canonical `S`, small-order keys and points rejected) |

**Post-quantum posture (owner decision).** Suite 1 is classical. Every sealed-key
blob, membership record and epoch record carries `suite_id` and `kem_id`, so a
hybrid KEM (for example X-Wing, or ML-KEM-768 with X25519) can be introduced at a
later key epoch as a new `suite_id` without a format break. A membership record
carries the suite's `kem_id` and a suite-typed, variable-length agreement public
key (`bytes(2048)`), and the device fingerprint covers `suite_id`, `kem_id` and
that key, so a hybrid KEM's larger public key enrolls with no new record layout.
All objects of one epoch use the single suite named in that epoch's record. A recipient that does
not implement the named suite fails with `UnsupportedSuite`; it never guesses.
Ciphertext already sealed under suite 1 is not retroactively protected against a
future quantum adversary; that limitation is accepted for v1.

### C.1.5 Canonical row encoding and primary key (`personal-cfo-t2s4b`)

One encoding of a database row is used everywhere a row is hashed: the write-set hash of
C.4, the replay-property hash that compares two replicas, and the simulator's equality
checks. It covers **every table of every class**; the write-set hash applies it to the
replicated rows only (class-2 rebuilds are excluded from that hash, not from the
encoding). It uses CBE1 (C.1.1).

**What the write set covers.** Exactly what Sync replicates, per the B.2 inventory:

- every table of Sync class 1, and each class-4 group that ADR 0074 Decision 5 routes to
  class 1 (the CLASS-0 manifest names them), with all of their columns, except the
  **folded children** below and `operation_log` (the command's own row);
- for each table whose B.2 disposition is `fields`, a **projection**: only the replicated
  fields B.2 names, as a logical record, so that a device-local or rebuilt column never
  enters a digest. These are `attachments` (`id`, `plaintext_size`, `mime_type`,
  `original_filename`, `created_at`; never `storage_id`, wrapped keys or nonces, and not
  the class-2 `ref_count`), `settings` (the `key` and `value` of the `user.` keys and the
  six legacy user keys, never `device.*`), and `vault_metadata`, whose
  `household_timezone` is the single field of the logical `household` record (B.3): its
  digest uses the table name `household`, the one column `household_timezone`, and the
  key `int 1`. The projection a command's apply writes is therefore never empty for
  `SetHouseholdTimezone`.

**Folded children.** A replicated table that declares no primary key cannot be a write-set
entry of its own, because a rowid is not the same on two devices. Such a table is **folded
into a parent**: its rows are part of the parent's digest, and the parent is the only
write-set entry. The column-kind manifest names the mapping:

| Child table | Parent table | Child column naming the parent |
| --- | --- | --- |
| `ledger_postings` | `ledger_transactions` | `transaction_id` (the parent's `id`) |
| `attachment_links` | `attachments` | `attachment_id` (the parent's `id`) |

A parent that has folded children appends a **child section** to its `row_digest` message,
after its own columns: `u16 child_table_count ‖ for each child table, in raw-name order:
text(64) child_table ‖ u32 count ‖ for each child row: bytes(2^16) child_row`. A
`child_row` is `u16 column_count ‖ for each column of the child table, over **all** its
declared columns and never a rowid, sorted by the raw bytes of the name: text(64) name ‖
tagged value`. The child rows of one parent are sorted **bytewise by the `child_row` bytes themselves,
excluding the `bytes(2^16)` length prefix, lexicographically, with a shorter row first when
it is a prefix of another (the rule of C.4), and duplicates are kept**, so two identical
postings count twice and the digest is a multiset. Sorting by the length-prefixed form
gives a different order and therefore a different digest; a vector below fixes the order. A parent with no folded children has no child section, so its message is
exactly the one defined below. A parent whose manifest declares children always carries the
section, with a count of 0 when it has no child rows now. An insert, update or delete of
any child row changes the parent's digest, so the parent's write-set entry is present
(the state-difference rule); a child row whose parent does not exist is an error, never an
entry.

**Folded children are bounded and unordered.** The 2^16 child-row bound is a schema-time
guarantee like the key bound (a schema test checks the folded children's columns). The
digest is a multiset, so two replicas may hold the same rows in different rowid order and
still agree: **no reader of a folded child table may depend on row order**, and every query
over one orders by its content columns.

**Deleting and moving.** A command that deletes a parent removes all its folded children
in the same transaction (a foreign-key cascade or explicit deletes), and the parent's absent
digest stands for the whole group. A child left without a parent after apply fails the
command before commit, deterministically on every replica; it is never discovered at hash
time. A child whose parent column changes is in both the old and the new parent's digest, so
both parents are write-set entries, and a capture maps a child write to `OLD` and `NEW`
parent. Both paths have tests.

`provenance_links` is **not** in the write set: B.2 gives it the disposition `exclude`, like
`operation_log`, and the write set is exactly what Sync replicates.

Each value is tagged by its column's **declared kind**, taken from the column-kind
manifest, which lists every column of every table with a kind and is aligned with the
portable export's kinds (B, "Row encoding"). The tag does not depend on SQLite's runtime
storage class or type affinity, so affinity can never make two devices encode one value
differently:

| Tag | Kind | Stored as | Encoding after the tag byte |
| --- | --- | --- | --- |
| `0` | any kind | NULL | nothing |
| `1` | `int` | INTEGER | `i64` |
| `2` | `text` | TEXT | `text(2^24)` (valid UTF-8; invalid UTF-8 is an error) |
| `3` | `uuid` | BLOB of exactly 16 bytes | the 16 raw bytes |
| `4` | `bytes` | BLOB | `bytes(2^24)` |

A stored value whose storage class does not match its declared kind (a TEXT in an `int`
column, a BLOB of another length in a `uuid` column, and so on) is an error, never
coerced. A REAL value has no tag and is an error: no table has a REAL column, and a test
proves it from the schema. A column absent from the manifest, or a manifest column absent
from the table, is an error. The digests are:

- `primary_key` is the concatenation, in the table's declared primary-key column order, of
  each key column's tagged value (tag byte, then the value). It is at most 256 bytes; a
  longer key is an error and is never truncated. Each tagged value is self-delimiting, so
  the concatenation is unambiguous for a table whose key columns are fixed.
- `row_digest` of a row that exists is `SHA-256(label "dohflow/sync-row/v1" ‖ text(64)
  table ‖ u16 column_count ‖ for each column: text(64) name ‖ tagged value)`, with the
  columns sorted by the **raw bytes of their names** (not by their position in the table,
  which a table rebuild may change). Every column the manifest lists for the table, or for
  its projection, is included; a parent with folded children then appends its child
  section (above). A class-1 table contains only class-1 columns (a schema
  test checks every class-1 table against CLASS-0), and a projection lists only replicated
  fields, so no device-local (class-3) or rebuild-timing (class-2) value ever enters a
  digest.
- `row_digest` of a row a command **deleted** (it existed before, it does not exist after)
  is `SHA-256(label "dohflow/sync-row-absent/v1" ‖ text(64) table ‖ bytes(256)
  primary_key)`, so a deletion changes the write-set hash and is distinguishable from a
  row that was not touched.
- **The write set is a state difference, not a record of writes.** It is exactly the
  class-1 rows whose final state differs from their state before the command: a row
  created, a row whose digest changed, and a row deleted. A no-op `UPDATE`, an `INSERT OR
  IGNORE` that hits an identical row, and a delete followed by a re-insert of identical
  columns are **not** in the set, so two engine builds, or a trigger-based and a
  diff-based capture, compute the same set. A row inserted and deleted inside the same
  command is not in the set, and a row touched several times appears once, with its final
  state. An update that changes a primary key is a deletion of the old key plus the
  presence of the new key. Rows changed by foreign-key actions (cascaded deletes and
  updates) are included exactly like direct writes, and a test covers a cascade.
- **Keys are never SQLite's to assign.** Every class-1 table declares an explicit primary
  key whose values the command supplies; none relies on an implicit rowid, and no
  `INTEGER PRIMARY KEY` or `AUTOINCREMENT` key is left for SQLite to allocate, because a
  rowid is not the same on two devices. Every class-1 key fits the 256-byte limit by
  construction (a bounded column width, checked by a schema test), so the limit is a
  schema-time guarantee and not a runtime failure that strands a command.
- A value that lives in a class-3 table and is not part of a B.2 projection is outside
  every write set: a divergence there is invisible to the fork check by design. The
  command that sets such a value carries it in its payload, and apply writes exactly the
  payload value, never something derived from local state.
- **Declared-kind validation is mandatory before Sync.** Before a vault's genesis (and
  as a local health check), the declared-kind check runs over every covered row. A stored
  value whose storage class disagrees with its kind blocks Sync as a typed state and is
  repaired by a reviewed migration that rewrites the value to its kind; it is never
  coerced at hash time. Otherwise a legacy mismatch would leave every later command that
  touches the row unhashable. The check is an acceptance criterion of the genesis bead
  (`personal-cfo-1cltt`) and is tested with one seeded mismatch per kind.
- The schema tests this text promises (no REAL column in a covered table, **every
  replicated table declares a supplied primary key or is listed as a folded child with its
  parent column**, key width of at most 256 bytes, class-1 tables hold only class-1
  columns, the manifest lists exactly each table's columns) are delivered by part 5b.
- The `operation_log` row of the command itself is not in the write set: it carries the
  write-set hash.
- The digests are bound to the schema version: the column set is the one the manifest
  lists for the `schema_version` in the envelope header. A receiver at another schema
  version does not apply the envelope and enters the ADR 0074 Decision 3
  update-required state; the hash is never skipped or recomputed over a different column
  set.

The two labels differ in their length prefix and in content, and neither can be parsed as
the other. Known-answer vectors, derived independently by the contract script
`scripts/tests/row-digest-contract.test.sh` (`table = tags`; `id` is a `uuid` column
holding the 16 bytes of `0198c2c1-5f0a-7c3d-8a4e-9b1d00000011`; `name` is a `text` column
holding `trip é`; `color` is a `text` column holding NULL; `sort_order` is an `int` column
holding `-7`; the first five markers below). The last four markers cover a parent with
folded children: the parent
is a `ledger_transactions` row with `id` the 16 bytes of
`0198c2c1-5f0a-7c3d-8a4e-9b1d00000021` and `memo` `split`; its children are three
`ledger_postings` rows over the same `transaction_id`, two identical (account
`...0031`, `-500`, `USD`, exponent `2`, `2026-10-17`) and one other (account `...0032`,
`1000`); `child_row_a` and `child_row_b` are the two distinct child encodings, and the
duplicate shows that the multiset keeps both copies (`parent_message` and `parent_digest`
are the parent's message, with its child section, and its digest). The last three markers
fix the child **sort order**: an `attachments` parent with two `attachment_links` rows
whose shorter encoding sorts after the longer one lexicographically (`entity_id` `...4a`
with entity kind `transaction_document`, and `entity_id` `...4b` with kind `tx`);
`order_digest` is the digest with the correct order (the longer row first),
`order_wrong_digest` the digest an implementation gets by sorting the length-prefixed
form, and the two differ:

<!-- row-vector: primary_key=030198c2c15f0a7c3d8a4e9b1d00000011 -->
<!-- row-vector: row_message=0013646f68666c6f772f73796e632d726f772f76310000000474616773000400000005636f6c6f7200000000026964030198c2c15f0a7c3d8a4e9b1d00000011000000046e616d6502000000077472697020c3a90000000a736f72745f6f7264657201fffffffffffffff9 -->
<!-- row-vector: row_digest=2f155b98841dc8b10bc71e369ee216022f9c15ef56a27b5165b36cb220d7b601 -->
<!-- row-vector: absent_message=001a646f68666c6f772f73796e632d726f772d616273656e742f7631000000047461677300000011030198c2c15f0a7c3d8a4e9b1d00000011 -->
<!-- row-vector: absent_digest=6d5ad824c2f6c542d09ce9250916c0135574503f89d8719c90292aac69347285 -->
<!-- row-vector: child_row_a=00060000000863757272656e637902000000035553440000001163757272656e63795f6578706f6e656e74010000000000000002000000116c65646765725f6163636f756e745f6964030198c2c15f0a7c3d8a4e9b1d000000310000000b6d696e6f725f756e69747301fffffffffffffe0c0000000c706f7374696e675f64617465020000000a323032362d31302d31370000000e7472616e73616374696f6e5f6964030198c2c15f0a7c3d8a4e9b1d00000021 -->
<!-- row-vector: child_row_b=00060000000863757272656e637902000000035553440000001163757272656e63795f6578706f6e656e74010000000000000002000000116c65646765725f6163636f756e745f6964030198c2c15f0a7c3d8a4e9b1d000000320000000b6d696e6f725f756e6974730100000000000003e80000000c706f7374696e675f64617465020000000a323032362d31302d31370000000e7472616e73616374696f6e5f6964030198c2c15f0a7c3d8a4e9b1d00000021 -->
<!-- row-vector: parent_message=0013646f68666c6f772f73796e632d726f772f7631000000136c65646765725f7472616e73616374696f6e730002000000026964030198c2c15f0a7c3d8a4e9b1d00000021000000046d656d6f020000000573706c697400010000000f6c65646765725f706f7374696e677300000003000000b400060000000863757272656e637902000000035553440000001163757272656e63795f6578706f6e656e74010000000000000002000000116c65646765725f6163636f756e745f6964030198c2c15f0a7c3d8a4e9b1d000000310000000b6d696e6f725f756e69747301fffffffffffffe0c0000000c706f7374696e675f64617465020000000a323032362d31302d31370000000e7472616e73616374696f6e5f6964030198c2c15f0a7c3d8a4e9b1d00000021000000b400060000000863757272656e637902000000035553440000001163757272656e63795f6578706f6e656e74010000000000000002000000116c65646765725f6163636f756e745f6964030198c2c15f0a7c3d8a4e9b1d000000310000000b6d696e6f725f756e69747301fffffffffffffe0c0000000c706f7374696e675f64617465020000000a323032362d31302d31370000000e7472616e73616374696f6e5f6964030198c2c15f0a7c3d8a4e9b1d00000021000000b400060000000863757272656e637902000000035553440000001163757272656e63795f6578706f6e656e74010000000000000002000000116c65646765725f6163636f756e745f6964030198c2c15f0a7c3d8a4e9b1d000000320000000b6d696e6f725f756e6974730100000000000003e80000000c706f7374696e675f64617465020000000a323032362d31302d31370000000e7472616e73616374696f6e5f6964030198c2c15f0a7c3d8a4e9b1d00000021 -->
<!-- row-vector: parent_digest=c31659d4006d3c4609c0b9d330c0d45627160732eddf048b749ac1ee4048ac1c -->
<!-- row-vector: order_message=0013646f68666c6f772f73796e632d726f772f76310000000b6174746163686d656e74730002000000026964030198c2c15f0a7c3d8a4e9b1d00000041000000096d696d655f747970650200000009696d6167652f706e670001000000106174746163686d656e745f6c696e6b73000000020000008700040000000d6174746163686d656e745f6964030198c2c15f0a7c3d8a4e9b1d000000410000000a637265617465645f6174020000000a323032362d31302d313700000009656e746974795f6964030198c2c15f0a7c3d8a4e9b1d0000004a0000000b656e746974795f6b696e6402000000147472616e73616374696f6e5f646f63756d656e740000007500040000000d6174746163686d656e745f6964030198c2c15f0a7c3d8a4e9b1d000000410000000a637265617465645f6174020000000a323032362d31302d313700000009656e746974795f6964030198c2c15f0a7c3d8a4e9b1d0000004b0000000b656e746974795f6b696e6402000000027478 -->
<!-- row-vector: order_digest=6b1d556830b76a329b6b35bcb605f5f4267d906bc62da24d9b085a269faf5cfb -->
<!-- row-vector: order_wrong_digest=a1ad1405a7e7474e666d7f5a2cc74945bdeae13bacd2864b8656555ff98c5f69 -->

## C.2 Keys and secrets

| Secret | Size | Lifetime and custody |
| --- | --- | --- |
| Sync epoch root `K_e` | 32 random bytes | One per key epoch; sealed only to enrolled devices; never used directly as an AEAD key; distinct from the vault DEK |
| Tag secret `S` | 32 random bytes | **Non-rotating**, created at Sync genesis, sealed to each device together with `K_e` and re-sealed unchanged at every epoch. It is an equality-tag key, not a confidentiality key. |
| Device signing key | Ed25519 | One per device; generated in platform-neutral Rust and wrapped at rest by the OS keystore |
| Device agreement key | the suite's KEM key pair (X25519 in suite 1) | One per device, independent of the signing key |

The two device keys are generated independently and are not derived from a
common seed. Hardware P-256 keys are not protocol keys. Key material lives in
zeroizing protected buffers in Rust, is loaded only when needed, and **never
crosses the IPC boundary** to the frontend.

**Device key custody (rules; platform behavior is unverified until
`personal-cfo-1zb` exists).** Device private keys and the nonce-counter state
never roam, sync or restore across devices, because two devices sharing a
fingerprint would reuse GCM nonces. The implementing bead must satisfy, per
platform: Apple, `kSecAttrSynchronizable` false and a `ThisDeviceOnly`
accessibility class; Windows, a local-machine or non-roaming credential store;
Android, a restored wrapped blob is detected and treated as `NonceStateLost`;
Linux, the Secret Service, failing with a typed error when it is absent, and
never falling back to plaintext storage. These requirements are carried into
the acceptance criteria of `personal-cfo-1zb`.

`device_key_fingerprint` is a 32-byte value, defined in C.3, that commits to
**both** public keys, the suite and the KEM id. No two enrollments may share a
fingerprint.

## C.3 Derivations

Every derived value is computed over CBE1 bytes. `‖` below means the fields are
encoded in the order shown with the kinds of C.1.1. `Expand(K, info)` is
HKDF-SHA256-Expand with PRK `K` and output length 32.

| Value | Derivation |
| --- | --- |
| `device_key_fingerprint` | `SHA-256(label "dohflow/device-fpr/v1" ‖ u16 suite_id ‖ u16 kem_id ‖ key32 ed25519_pk ‖ bytes(2048) kem_public_key)` |
| AEAD key `K_{e,d,t}` | `Expand(K_e, label "dohflow/sync-aead/v1" ‖ u16 protocol_version ‖ uuid vault_id ‖ u64 key_epoch ‖ fingerprint ‖ u8 object_type)` |
| `nonce_domain` | first 4 bytes of `SHA-256(label "dohflow/sync-nonce/v1" ‖ uuid vault_id ‖ u64 key_epoch ‖ fingerprint ‖ u8 object_type)` |
| GCM nonce | `nonce_domain` (4 bytes) then `u64 invocation_counter` (8 bytes) |
| Tag key | `Expand(S, label "dohflow/sync-idem-tag-key/v1" ‖ uuid vault_id)` |
| `idempotency_key_tag` | `HMAC-SHA256(tag key, label "dohflow/sync-idem-tag/v1" ‖ uuid canonical_idempotency_key)` |
| `immutable_metadata_commitment` | `SHA-256(label "dohflow/sync-meta-commit/v1" ‖ uuid command_id ‖ uuid correlation_id ‖ opt(uuid) causation_id ‖ u8 actor_type ‖ text(256) actor_id ‖ uuid idempotency_key ‖ i64 issued_at ‖ uuid device_id)` |
| Key commitments | `SHA-256(label "dohflow/sync-epoch-key-commit/v1" ‖ key32 K_e)` and `SHA-256(label "dohflow/sync-tag-key-commit/v1" ‖ key32 S)` |
| `sync_artifact_id_e` | `HMAC-SHA256(Expand(K_e, label "dohflow/sync-artifact-address/v1" ‖ uuid vault_id), storage_id_raw)`, where `storage_id_raw` is the 32 bytes behind the local `StorageId`'s hex form |
| Artifact object key | `Expand(K_e, label "dohflow/sync-artifact-object/v1" ‖ u16 protocol_version ‖ uuid vault_id ‖ u64 key_epoch ‖ digest sync_artifact_id)` |
| Artifact manifest commitment | `SHA-256(label "dohflow/sync-artifact-manifest/v1" ‖ list(digest, 4096) of the ids in strictly ascending byte order)` |
| `canonical_envelope_digest` | `SHA-256(label "dohflow/sync-envelope-digest/v1" ‖ u32 len ‖ header bytes ‖ u64 len ‖ ciphertext)` |

`issued_at` is a signed 64-bit count of microseconds since the Unix epoch, UTC.
`canonical_idempotency_key` is the 16 raw bytes of the key's UUID (C.5).
`K_e` and `S` are uniformly random, so Expand-only use is sound; the labelled,
versioned context provides domain separation.

## C.4 AEAD objects: envelopes and snapshots

### Clear header

The header is the AAD of the object's AEAD. Its fields, in order:

| # | Field | Kind | Notes |
| --- | --- | --- | --- |
| 1 | label `dohflow/sync-header/v1` | `label` | |
| 2 | `protocol_version` | `u16` | `1` |
| 3 | `vault_id` | `uuid` | |
| 4 | `key_epoch` | `u64` | |
| 5 | `object_type` | `u8` | `1` envelope or `2` snapshot |
| 6 | `object_id` | `uuid` | the immutable `command_id`, or the snapshot id |
| 7 | `idempotency_key_tag` | `opt(digest)` | present for an envelope, absent for a snapshot |
| 8 | `immutable_metadata_commitment` | `opt(digest)` | present for an envelope, absent for a snapshot |
| 9 | `device_id` | `uuid` | |
| 10 | `device_key_fingerprint` | `fingerprint` | |
| 11 | `nonce_domain` | 4 raw bytes | |
| 12 | `invocation_counter` | `u64` | at most `2^32 - 1` |
| 13 | `base_seq` | `u64` | |
| 14 | `payload_schema_version` | `u32` | |
| 15 | `engine_version` | `text(64)` | |
| 16 | `schema_version` | `u32` | the vault schema version of the producer |
| 17 | `artifact_manifest_commitment` | `opt(digest)` | an envelope: the delta manifest commitment, present **only when the delta manifest is non-empty** (absent otherwise); a snapshot: always present, the commitment of its full closure even when that closure is empty |

The service-assigned `seq` is **not** in the header: it does not exist until the
service accepts the already sealed object, and the service binds it to the
immutable `object_id` in its append-only log. Any epoch, type, fingerprint,
domain, counter, tag, commitment or field mismatch fails closed.

### Container

`magic "DFSY"` (4 ASCII bytes), `u16 container_version = 1`, `u32 header_len`,
the header, `u64 ciphertext_len`, then the ciphertext (the AES-256-GCM output
including its tag). A parser rejects any other length, any trailing byte, and an
unsupported container version.

### Encryption

The key is `K_{e,d,t}` for the object's epoch, device and type, and the nonce is
`nonce_domain ‖ invocation_counter`. The AAD is the header bytes. An exact-byte
retransmission reuses the saved ciphertext; a crash or a changed retry **burns**
the counter, and the next encryption uses the next counter value.

### Receiver checks

After authenticated decryption, and before any apply, rebase, queue or accepted-
index reconciliation, a receiver:

1. recomputes `idempotency_key_tag` from the plaintext's `idempotency_key` and
   `immutable_metadata_commitment` from the plaintext's complete metadata, and
   requires both to equal the header fields; a separately tampered clear field
   already fails AEAD authentication, because the header is the AAD, and the
   ciphertext and a diagnostic record are retained;
2. checks that `device_id` and `device_key_fingerprint` belong to the same member
   of the membership chain at the object's epoch;
3. for an envelope, requires a UUIDv4 idempotency key (C.5);
4. checks the manifest commitment rule of header field 17: the plaintext's delta
   manifest is strictly ascending and duplicate-free, and the header's commitment
   is its commitment, or both are empty and the header field is absent. A header that
   carries a commitment for an empty delta manifest (even the commitment of the empty
   list) is refused, because it would be a second valid encoding of the same envelope
   with a different `canonical_envelope_digest`;
5. checks that the header's `object_id` is the plaintext's `command_id`, that its
   `device_id` is the plaintext's `device_id`, and that its `nonce_domain` is the one
   derived (C.3) for its epoch, device fingerprint and object type.

### Envelope plaintext (`EnvelopePlaintextV1`)

`label "dohflow/sync-envelope-plaintext/v1" ‖ uuid command_id ‖ uuid
correlation_id ‖ opt(uuid) causation_id ‖ u8 actor_type ‖ text(256) actor_id ‖
uuid idempotency_key ‖ i64 issued_at ‖ uuid device_id ‖ digest write_set_hash ‖
list(digest, 4096) delta_manifest ‖ bytes(2^24) payload`. The delta manifest is
in strictly ascending byte order, like every manifest list.

`payload` is canonical JSON (C.1.2). Together with the header, the envelope
therefore carries the complete original `CommandMeta` (`command_id`,
`idempotency_key`, `issued_at`, `correlation_id`, `causation_id`, `device_id`,
plus `actor_type` and `actor_id`), the payload, schema and engine versions, the
class-1 write-set hash, `base_seq`, and the immutable `object_id`. The optional
post-acceptance `seq` is carried only by the service's response and index.

`write_set_hash` is `SHA-256(label "dohflow/sync-writeset/v1" ‖ u32 count ‖ for
each touched class-1 row: text(64) table ‖ bytes(256) primary_key ‖ digest
row_digest)`. The count is at most `2^20`, and each `(table, primary_key)` pair
appears at most once (a duplicate is `MalformedObject`). Rows sort by the **raw bytes** of the
table name, then by the raw bytes of the primary key, bytewise lexicographically
with a shorter value first when it is a prefix of the other; they do not sort by
their length-prefixed encodings. The primary key's bytes are the C.1.5 tagged
encoding, never the SQL value, so a negative integer key (two's complement) sorts after a
positive one; that order is deterministic and is the order. `row_digest`
is the SHA-256 of the row's one canonical binary encoding, which
is defined in C.1.5, as is the `primary_key` encoding and the digest of a deleted row.
The set of rows is the C.1.5 state difference: the class-1 rows whose final state differs
from their state before the command. Class-2 rebuilds are excluded, and so is the command's
own `operation_log` row.

### Push and producer rules

The service stores sealed objects it cannot read, so it can refuse a push for what the header
and the chain show, but not for anything that needs the plaintext. An envelope sealed
inconsistently by a buggy or partly failed producer would otherwise be accepted at a sequence
number and fail the receiver checks on every replica at that position, stopping the whole
vault. The two sides therefore share the work, and between them leave no class of inconsistency
unchecked.

The service MUST work in this order: **parse, then the replay lookup of C.10, then the rest**.
First it parses the container and header strictly (grammar, the dual-identity shape, the counter
cap, the `engine_version` grammar, a ciphertext of at least a GCM tag), refuses an object over the
envelope size cap and a snapshot pushed as a command envelope, requires `vault_id` to be the
routed vault, and computes `canonical_envelope_digest` itself from the received bytes. It then
does the replay lookup of C.10; an exact retry of an accepted envelope returns its original `seq`
even if its device has since been revoked or replaced (after `NonceStateLost`, or a restore) and
even with an old `key_epoch`. Only for an identity the index does **not** know does it go on to
require `device_id` and `device_key_fingerprint` to be one currently authorized member of the
chain, `nonce_domain` to be the one derived (C.3) for the epoch, fingerprint and object type,
`key_epoch` to be the epoch in force, and the nonce-reuse backstop above to hold.

The producer MUST, in this order: (1) apply the command locally and commit the op-log row
together with the counter advance, before encrypting; (2) seal; (3) before it uploads, open the
sealed object it is about to send through the
**receiver path** (the AEAD key derived from the expected vault id, key epoch and its own
fingerprint from the chain, the nonce built from the header's nonce domain and counter; never the
sealing code's key object and never the plaintext it still holds), decode the plaintext, and run
the receiver checks above on that header and plaintext. It MUST NOT upload on any failure; the
command stays in its outbox with a typed error and is not retried in a loop, because the failure
is a producer bug; the change is committed on this device only and cannot sync, and the user is
told exactly that. (4) It persists the verified object with its outbox entry, and (5) uploads by
that object only. The bytes verified are the bytes uploaded: the upload is by the verified object,
never by raw bytes plus an approval. A crash between (2) and (4) re-seals with a fresh counter and
never reuses the old one; after an application update the command is re-sealed with a new counter
and verified again.

**The check set is versioned with `engine_version`.** Every replica that applies an envelope has
the same `engine_version` (C.13), and the receiver checks are part of what that string names: a
change to what the checks accept is an `engine_version` change. Two replicas that both evaluate
a log position therefore run the same checks, which keeps "this envelope is invalid" a fact the
whole vault agrees on. (A device registry that never decrypts is not part of that agreement.)

A replica that finds an authenticated envelope at position `N` failing the receiver checks is in
the typed state `AuthenticatedEnvelopeInvalid` (C.11): it applies nothing at or past `N`, it does
not push (any push would need a base at or past `N`), it keeps its outbox, and it surfaces the
position, the command and the sealing device. It is neither a fork nor a retryable error. The
position is identified by `(seq, canonical_envelope_digest)`: the same position with a different
digest is a fork of the log (`ProtocolFork`).

### Counter register and nonce state

Each device owns a durable `next_counter` for every
`(key_epoch, device_key_fingerprint, object_type)` context. It persists the
advance **before** encrypting. The register has an independent anti-rollback
witness that a backup restore does not restore; a missing, restored or
inconsistent witness is `NonceStateLost`, and the client must not encrypt in
that context. It re-enrolls with a fresh device key pair and fingerprint, or
bootstraps a fresh epoch. Each derived key has a cap of `2^32` invocations;
before the next invocation the client completes a fresh-key epoch rotation. The
service records the first `(key_epoch, fingerprint, object_type, nonce)` it
receives and accepts only an exact-byte retry afterwards, as a duplicate-detection
backstop and never as the security source. The same independent witness also
protects the newest verified membership head and the accepted epoch chain (C.6).

## C.5 Idempotency keys and the tag

Every idempotency key that enters a Sync envelope is a UUID version 4 drawn from
the operating system's CSPRNG, enforced as follows:

- the backend mints idempotency keys with version 4, not version 7
  (`command_id` may remain version 7);
- the envelope builder rejects any other format with the typed error
  `InvalidIdempotencyKey`, covered by a negative test (a version 7 key and a
  wrong-variant key are the required cases);
- `canonical_idempotency_key` is the 16 raw UUID bytes.

With 122 random bits per key, a device holding `S` after revocation can only
recognize tags of keys it has already seen; it cannot confirm guessed keys. That
residual is accepted. `S` is shared by every enrolled device and is never
rotated, so the tag is stable across key epochs and across a vault key rotation
([ADR 0083](../adr/0083-vault-key-rotation.md)). A per-epoch tag key is rejected:
it would break the dual-identity lookup that ADR 0074 Decision 2 requires after
tail truncation.

`S` lives as long as the Sync vault. A second genesis for a known `vault_id` is
impossible (ADR 0074 Decision 9), so `S` never needs replacing. If every enrolled
device is lost, no device can sign membership changes or unseal keys, and the
vault cannot rejoin Sync (ADR 0074 Decision 10: no escrow in v1). The client
reports the typed outcome `NoAuthorizedDevice`; a local backup restores the local
vault, but its unsent outbox cannot be pushed.

## C.6 Membership and epoch records

### Signed record container

A signed record is `u32 body_len`, the body bytes, then a 64-byte Ed25519
signature over **the body bytes**. Each body starts with its own label, so a
signature made for one record type cannot be reinterpreted as another. A record
is identified by `SHA-256(body)` of its body bytes; `prev_id`, `membership_head` and
`prev_epoch_id` are record identifiers of this form. Verification uses strict Ed25519 verification
against the signer's `ed25519_pk` from the membership chain, found by
fingerprint.

### Membership record body

`label "dohflow/sync-membership/v1" ‖ u16 protocol_version ‖ u16 suite_id ‖ u16
kem_id ‖ uuid vault_id ‖ u64 version ‖ opt(digest) prev_id ‖ u8 action ‖
fingerprint signer_fingerprint ‖ uuid device_id ‖ key32 ed25519_pk ‖
bytes(2048) kem_public_key ‖ i64 issued_at`. In suite 1 the `kem_public_key` is
the 32-byte X25519 public key; any other length is `MalformedObject`, and so is an
HPKE `enc` that is not 32 bytes in suite 1.

Chain rules, all enforced by every client and by the service:

1. The first record has `version = 1`, an absent `prev_id`, `action = 1`, and is
   signed by the device it adds (`signer_fingerprint` equals that device's
   fingerprint). The first Sync device creates it locally.
2. Every later record has `version = previous + 1` and `prev_id` equal to the
   previous record's id, with the same `vault_id`.
3. The signer must be an authorized, unrevoked member in the state after
   `prev_id`. The service stores and enforces the chain but cannot add a
   recipient, replace a key or choose a fingerprint.
4. A fingerprint is never reused: an `add` of a fingerprint already in the chain
   is rejected, and a revoked device can never sign or be added again.
5. A device's fingerprint is recomputed from the `suite_id`, `kem_id`,
   `ed25519_pk` and `kem_public_key` of the record that adds it, and every later
   record's `signer_fingerprint` and every epoch-record recipient must equal that
   recomputed value.
6. A `revoke` record repeats the revoked device's `device_id`, `ed25519_pk` and
   `kem_public_key` exactly as its `add` record, and revokes a fingerprint that is
   currently authorized.
7. A replacement after `NonceStateLost` is a `revoke` of the old fingerprint and
   an `add` of a new device with fresh keys, signed by an existing member.
8. A `device_id` is never reused: an `add` of a `device_id` already in the chain,
   including one that was revoked, is rejected (a re-added device is a fresh enrollment,
   which has a new `device_id`).
9. A `revoke` that would leave no authorized member is rejected: its cutover could never
   be sealed, because the sealer must be authorized at the revoke's head. A device may
   revoke itself only when another member remains to seal the cutover.
10. An `add` that would leave more than 1,024 authorized members is rejected, because the
    next cutover must list every one of them.
11. An `add` whose `ed25519_pk` or `kem_public_key` equals one already in the chain, under
    a new fingerprint, is rejected (it would create an alias identity).

`issued_at` is informational: no chain rule depends on a clock (ADR 0017 decision 3).

### Key and signature acceptance

A signature, and every public key that enters the chain, is checked by these rules, stated
as checks so that implementations in other languages reproduce them. They are the strict
rules, not the permissive ones some libraries offer by default, and a record that one
implementation accepts and another rejects would split the views permanently.

1. An `ed25519_pk` is accepted only if it decompresses to a curve point, re-compresses to the
   same 32 bytes (a non-canonical encoding is rejected), lies in the prime-order subgroup
   (`[L]A` is the identity, which excludes every mixed-order point and every point of order 2,
   4 or 8), and is not the identity itself (the identity is in the subgroup but has order 1, and
   a key `A` = identity would let anyone forge). In short: `A` has order exactly `L`. This is
   checked when the key is added, so such a key never enters the chain.
2. A signature `R || S` is accepted only if `R` decompresses, re-compresses to the same 32
   bytes, lies in the prime-order subgroup and is not the identity (`R` has order exactly `L`);
   `S`, read as a little-endian integer, is below the group order `L`; and the cofactorless equation holds: the 32-byte encoding of
   `[S]B - [k]A` equals `R`, for `k = SHA-512(R || A || message)` reduced mod `L` (RFC 8032
   section 5.1.7, without cofactor multiplication). With `A` and `R` in the prime-order subgroup
   and `S` below `L`, the cofactored and cofactorless equations accept exactly the same signatures,
   so an implementation's choice of equation cannot make it disagree.
3. A suite-1 `kem_public_key` (X25519) is accepted only if, after clearing the unused top bit
   and reducing modulo `2^255 - 19`, it is none of the low-order values `0`, `1`, `2^255 - 20`,
   `325606250916557431795983626356110631294008115727848805560023387167927233504` and
   `39382357235489614581723060781553021112529911719440698176882885853963445705823`.
4. The signer's key for a record is always the key the **chain** holds for the signer's
   fingerprint; it is never taken from the record being verified (the genesis record, which
   has no earlier chain, is verified under the key it contains).

**Check order.** A record with more than one defect is refused for the first of these, so every
implementation reports the same C.11 failure: parse and protocol version; suite, KEM and vault;
position and linkage (`version`, `prev_id`, and for the genesis its required shape); signer and
signature (and, for the genesis, the key screen first, because its signature is verified under
the key it contains); then the rule checks (key screens for an `add`, reuse, the revoke rules,
the member cap). Epoch records: parse and protocol version; suite, KEM and vault; a conflict with
an accepted record; the named head; the sealer's authorization at that head and the signature;
then the sequence and reason table; then the recipients.

### Epoch record body

`label "dohflow/sync-epoch-record/v1" ‖ u16 protocol_version ‖ u16 suite_id ‖ u16
kem_id ‖ uuid vault_id ‖ u64 key_epoch ‖ u8 reason ‖ digest membership_head ‖
opt(digest) prev_epoch_id ‖ digest key_commitment_Ke ‖ digest key_commitment_S ‖
fingerprint sealer_fingerprint ‖ list(recipient, 1024)`.

This header (everything before the recipient list) has its own hash,
`epoch_header_hash = SHA-256(header bytes)`. Each `recipient` is `uuid device_id
‖ fingerprint ‖ u16 suite_id ‖ u16 kem_id ‖ bytes(2048) enc ‖ bytes(256)
ciphertext`. Epoch records form a chain: `prev_epoch_id` is the id of the
previous epoch record, absent only for the first.

The explicit `reason` says what the record is for, so a recipient never has to
infer it:

| `reason` | Meaning | `key_epoch` | Key | `membership_head` | Recipients |
| --- | --- | --- | --- | --- | --- |
| `1` | genesis | `1` | fresh `K_1` and fresh `S` | version 1, the genesis `add` | the founding device |
| `2` | revocation cutover | previous + 1 | fresh `K_{e+1}`; `S` unchanged | a `revoke` record newer than the previous record's head | exactly the authorized members at that head |
| `3` | grant | same as previous | unchanged `K_e` and `S`; both commitments equal the previous record's | an `add` record newer than the previous record's head | exactly the devices added between the previous record's head and this one |
| `4` | forced rotation (a derived key at its cap, or after `NonceStateLost`) | previous + 1 | fresh `K_{e+1}`; `S` unchanged | **the previous record's head**: a rotation changes no membership | exactly the authorized members at that head, the same as the previous record's |

### Coupling a membership change to an epoch record

Every membership record after the genesis is installed by the service in the
**same transaction** as exactly one epoch record that names it as its
`membership_head`: an `add` with a grant (`reason 3`) and a `revoke` with a
cutover (`reason 2`). The genesis membership record is installed with the genesis
epoch record. A membership record that no epoch record names is
`EpochRecordMissing`. An added device therefore receives the current `K_e` and `S`
from a grant, with **no key rotation and no new snapshot**: it unseals the grant
and bootstraps from the latest snapshot of the current epoch, whose AEAD key it
can derive. A revocation cutover and a forced rotation are the only events that
change the key, and each publishes a snapshot under the new key and re-seals the
live artifacts (C.7). Because every head has a coupled record, an
anchor exists whenever a head does.

**What a grant gives, and its conditions.**

- *Scope.* A grant gives the added device the current `K_e`, so it can read every
  object of the current epoch, including the change history the service stores
  since the last cutover. Removing it later stops it reading anything new; it does
  not stop it reading what it has already seen.
- *A grant never outlives its epoch (an add racing a cutover).* An `add` names the
  newest membership record as its `prev_id`, and the service installs a membership
  record only if that `prev_id` is still the newest (compare and swap). An `add`
  and its grant prepared before a revocation cutover therefore fail: the `add`'s
  `prev_id` is stale, and the grant would in any case seal the pre-cutover `K_e`,
  so it is rejected as `BadEpochSequence` (its `key_epoch` is not the epoch in
  force) or, if forged to claim it, `KeyCommitmentMismatch`. The adder retries from
  the new head: a fresh `add`, and a grant that seals the new `K_{e+1}` under the
  new epoch number. The service rule belongs to `personal-cfo-v98vd`, which owns the
  test; the reference model of this part tests the epoch-record side.
- *Optional same-epoch snapshot.* A grant transaction may also include a fresh
  snapshot published under the current epoch at the new head, so the added device
  need not rely on an older one. It is optional, and the snapshot is an ordinary
  type `2` object, so it needs no format change.
- *Re-adding is a fresh enrollment.* A removed device is never re-added: coming back
  means a new enrollment with new key pairs, a new fingerprint and a new
  `device_id` (chain rule 4).

### Sealing the key set (F1)

The sealed-key set travels **only** inside a signed epoch record. For each
recipient the sealer runs HPKE base mode `Seal` to the recipient's
`kem_public_key` with:

- `info` = `label "dohflow/sync-hpke-info/v1" ‖ u16 protocol_version ‖ u16
  suite_id ‖ uuid vault_id ‖ u64 key_epoch ‖ uuid recipient_device_id ‖
  fingerprint recipient_fingerprint ‖ fingerprint sealer_fingerprint ‖ digest
  membership_head ‖ digest epoch_header_hash`;
- `aad` = `label "dohflow/sync-hpke-aad/v1" ‖ digest epoch_header_hash ‖
  fingerprint recipient_fingerprint`;
- plaintext = `label "dohflow/sync-sealed-keys/v1" ‖ key32 K_e ‖ key32 S`.

`S` is sealed together with `K_e` in the same per-recipient payload. It is not
encrypted under `K_e`. The `reason` is inside the epoch header, so the header hash
that the `info` and `aad` bind also binds it.

A recipient list is in one canonical order, **strictly ascending by fingerprint** with no
duplicate, so one set of recipients has one encoding. In suite 1 each recipient's `enc` is
exactly 32 bytes and its `ciphertext` is exactly 109 bytes: the 93-byte sealed-keys plaintext
plus the 16-byte AES-GCM tag. A recipient's `device_id`, fingerprint, `suite_id` and `kem_id`
equal those of its `add` record. Reasons 2 and 4 must publish a `key_commitment_Ke` that no
earlier epoch record of the chain carried (a cutover that re-seals an old key is invalid),
reason 3 repeats both commitments of the previous record, and every record repeats the first
record's `key_commitment_S`.

A recipient accepts an epoch record only when it:

1. verifies the signature strictly and checks that the sealer is an authorized
   member of the chain at the record's `membership_head`;
2. checks that the recipient list matches the record's `reason` (the table above):
   exactly the authorized, unrevoked members at that head for reasons `1`, `2` and
   `4`, and exactly the newly added devices for reason `3`, with no extra
   recipient and none missing;
3. unseals its own entry, then requires `SHA-256(commitment label ‖ K_e)` and the
   corresponding value for `S` to equal the record's two key commitments, and for
   reason `3` requires both commitments to equal the previous record's;
4. for any record after the first, requires `key_commitment_S` to equal the first
   record's value (`S` never changes);
5. rejects a suite or `kem_id` it does not implement with `UnsupportedSuite`.

Any failure is `EpochRecordInvalid` or `KeyCommitmentMismatch`; no key from a
rejected record is installed.

### Freshness, sequencing and catch-up (F1)

A valid signature and consistent commitments are not enough: a member who has
since been revoked can still produce a well-formed record that names an older
membership head, where that member was authorized. A recipient therefore also
requires all of the following. The checks above are evaluated for each record
**at the head that record names**.

1. **The anchor names the newest head.** The newest epoch record a recipient is
   shown, its **anchor**, must name the newest membership record in the
   recipient's verified chain. An anchor naming an older head is rejected with
   `StaleMembershipHead`. An anchor naming a head the recipient has not verified
   makes it first fetch and verify the membership chain up to that record (an
   unverifiable chain is `MembershipChainInvalid`), and then apply this rule; a
   record is never accepted against an unverified head. This rule applies to the
   anchor only.
2. **The chain and sequence hold.** `prev_epoch_id` equals the previous record's
   id; `key_epoch` follows the `reason` table; heads never move backwards and
   strictly advance across reasons `2` and `3`, and a forced rotation names the same
   head as the previous record. A cutover must name a `revoke`
   record (`CutoverHeadNotRevoke` otherwise) and a grant an `add` record
   (`GrantHeadNotAdd` otherwise). Any other break is `BadEpochSequence`.
3. **Catch-up walks backward from the anchor.** A device that was offline across
   several epochs verifies the membership chain to its newest head, takes the epoch
   record that names it as the anchor, and walks `prev_epoch_id` back to its last
   accepted record. Each intermediate record must hash (as `SHA-256(body)`) to the
   id its successor links to; must follow rule 2; and must pass every check above
   **at its own named head**, with its sealer authorized there. The anchor's
   authorized sealer and the hash chain vouch for each intermediate, even if an
   intermediate's sealer was revoked later. The device installs an intermediate
   epoch's keys for reading only, and only if it was a recipient, and it encrypts
   only under the anchor's epoch. Nothing is installed if any link fails.
4. **Equivocation is detected at every position.** Two different records with the
   same `prev_epoch_id`, including a second genesis record, are
   `EpochEquivocation`; two different membership records for one `version` are
   `MembershipChainFork`. A conflicting record counts only if it would itself have been valid at
   that position: a membership record with the right `prev_id`, a signer authorized there and a
   valid signature under the chain's key (for a second genesis record, one signed by the
   accepted founding device: a self-signed genesis from anyone else has no authority over the
   chain), or an epoch record whose sealer is authorized at its named head with a valid
   signature. Anything else at that position is an ordinary invalid
   record, because unauthenticated bytes must never put a device into the halt state below. Both records are retained, nothing from either is
   installed, and the device halts as below. An exact duplicate is idempotent.
5. **Verified state is rollback-protected.** The newest verified membership head,
   the accepted epoch chain and the per-position record map are persisted under the
   same independent anti-rollback witness as the nonce counters (C.4). Older
   restored state is `StateRolledBack`, handled like `NonceStateLost` by
   re-enrollment, and is never silently accepted.

### Detection and recovery

When a device detects equivocation or a chain fork, **including after a forged
record was already installed**, it:

1. enters `SyncHalted`: it encrypts nothing for Sync and installs no further key;
2. quarantines every key it adopted from the offending record, flags every object
   it encrypted or sealed under such a key as exposed, and shows the window to the
   user;
3. keeps its outbox: envelopes produced on the forked branch are not accepted on
   the real chain, so they stay queued and are re-encrypted under the real epoch
   only after the index check of C.10 shows both their identities unknown;
4. retains both records for diagnosis;
5. recovers by re-enrolling through an honest member (chain rule 6) or by
   rebootstrapping from a snapshot of the real chain. Local data is never deleted.

**Residual.** The rules guarantee that a device that has verified the newer head
never accepts a stale-head record, and that no device accepts a record that
contradicts its own verified chain. They cannot give a device a view it was never
shown. A service that withholds membership or epoch records can keep a device on an
older view; without an external freshness anchor the strongest property available
is fork consistency, and a stale device cannot tell. Exploiting this takes a
revoked (or otherwise compromised, not yet revoked) member **together with** a
malicious or compromised service, because a revoked member's credential no longer
works and only the service can deliver its records. The forger can fork the
membership chain itself, not only an epoch record: a record signed by the since-
revoked member that adds an attacker-held device, or revokes an honest one, is a
valid extension of the stale device's chain. The exposure is then everything the
stale device encrypts under keys from the fork and everything it seals to
recipients the fork adds. The same holds for any compromised member before its
revocation. As soon as the device sees the real chain it detects the fork and
follows the steps above. An out-of-band membership code compared across devices
would narrow the window further and is **not** specified in this revision. The
epoch-position rule of C.7 is the narrowing this revision adopts; it binds a client
only to the log order it has observed, and a service that controls the order can
place a stale envelope before a cutover in history the client has not yet seen.

## C.7 Epoch cutover, rotation and compatibility

Revocation is a fresh-key, atomic epoch cutover (ADR 0074 Decision 8): an
authorized remaining device appends a `revoke` membership record, generates a new
`K_{e+1}`, publishes a signed epoch record (`reason 2`) sealed to the remaining
members, and publishes a new logical snapshot, all in one service transaction. `S` is carried
unchanged.

- A device reads an object of epoch `e'` only if it holds `K_{e'}`. A device
  enrolled at epoch `e` bootstraps from the snapshot at the current head and
  never needs older tails.
- **Key-changing epochs.** A revocation cutover (`reason 2`) and a forced rotation
  (`reason 4`) both start a new key epoch. In one service transaction each installs
  its epoch record, publishes a snapshot under `K_{e+1}` at the current head, and
  re-addresses and re-seals every live artifact, and the service then rejects
  old-epoch writes. They differ only in membership: a cutover also appends the
  `revoke` record it names, while a rotation changes no membership and names the
  previous record's head.
- Every live artifact is re-addressed and re-sealed at each key-changing epoch,
  because its `sync_artifact_id_e` and object key depend on `K_e` (C.9).
- **Retry order (F5).** The service performs the dual accepted-index lookup
  (C.10) **before** it rejects an old-epoch write. An exact retry of an already
  accepted old-epoch envelope returns its original `seq` and result. A client
  re-encrypts an outbox entry under a new epoch only after the index confirms
  that both its `command_id` and its tag are unknown, and a request that matches
  one identity but not the other is `ProtocolFork`.
- **Epoch position.** The **position** of a key-changing epoch record (`reason 2`
  or `4`) is the log head `h` at which the compare-and-swap that installs it, with
  its snapshot, is accepted; that snapshot's header `base_seq` equals `h`. The first
  epoch (`reason 1`) has position `0`. A client learns `h` from the service's log
  entry for the record and checks it against the authenticated `base_seq` of that
  epoch's first snapshot; a mismatch is `EpochRecordInvalid`. The boundary is
  exact: an envelope with `seq <= h` belongs to the earlier epoch and one with
  `seq > h` to the new epoch. A client rejects with `EpochRejected` an envelope
  whose `key_epoch` is not the epoch in force at its `seq` by this rule: an older
  key after a key-changing epoch, or an epoch not yet installed at that position.
  The service applies the same rule to new pushes. An envelope accepted at
  `seq <= h` keeps its place. **The guarantee is relative to the log order the
  client has observed:** the service assigns both an envelope's `seq` and the
  epoch's position, so a malicious service can still place a stale envelope before
  a cutover in history the client has not yet seen (the residual of C.6). This adds
  nothing to the AAD of ADR 0074.
- **Cutover and grant records.** A revocation cutover is an epoch record with
  `reason 2` naming the `revoke` record; an added device is brought in by a grant
  with `reason 3` naming the `add` record and sealing the current keys to it
  without a rotation. Both are checked under the freshness and sequencing rules of
  C.6, and two records at one chain position are equivocation.
- A future suite is introduced at a new key epoch (C.1.4).

**Requirements on clients (not enforced by this format).**

1. When a device is added, the client says, in plain language: "A device you add
   can read everything in this vault, including the change history stored on the
   server since a device was last removed. Removing it later stops it reading
   anything new, not what it has already seen."
2. When a device is revoked, the client lists every device that the revoked device
   added (the `add` records it signed) and offers to revoke those as well, because
   a compromised member can enroll devices it controls.
These requirements are carried by the client and settings beads, not by this
format.

## C.8 The logical genesis snapshot

The snapshot is an AEAD object of type `2` whose plaintext is the logical model
of Part B restricted to the Sync dispositions of the B.2 inventory.

**Full-history guarantee.** The genesis carries the complete current canonical
state, which includes **every class-1 ledger row of every age**
(`ledger_transactions`, `ledger_postings`, `transaction_details`, `split_lines`
and the rest of the class-1 tables), so a device that bootstraps from it, a phone
included, can view all historical transactions, not only recent ones. What the
genesis does **not** carry is the pre-v2 edit audit trail (`operation_log` and
`provenance_links` history). That trail stays in the portable export and on the
originating device, and each replica keeps its own local history from the point
it joins.

### Plaintext framing (`SnapshotPlaintextV1`)

`label "dohflow/sync-snapshot-plaintext/v1" ‖ u32 file_count` (at most 1024), then per file
`text(255) path ‖ blob content`. The first file is `manifest.json`; the other
files are the `data/<table>.jsonl` files in ascending path order. The manifest
(canonical JSON) lists every table file with its row count and SHA-256, and the
**complete closure manifest** of `sync_artifact_id_e` values; its commitment is
the header's `artifact_manifest_commitment`. The set of files must equal the set
the manifest names, with none missing and none extra.

The household timezone travels as the logical class-1 field `household_timezone`
of the `household` record in `data/household.jsonl` (Part B), never as a header
field, so later changes to it propagate by command.

A snapshot's plaintext is at most `2^30` bytes (1 GiB). A larger vault fails with
the typed error `SnapshotTooLarge` until a chunked framing is specified.
Publication follows ADR 0074 Decision 1: the service accepts a snapshot only when
`base_seq` equals the current head and the snapshot, its artifact closure and
every closure pin are durable in one compare-and-swap transaction.

## C.9 Artifact transport

A class-1b artifact (an attachment's bytes, or an import provenance artifact) is
published as an immutable, epoch-scoped object.

**Plaintext (`SyncArtifactTransportV1`).** `label
"dohflow/sync-artifact-transport/v1" ‖ digest sync_artifact_id ‖ u8
artifact_kind ‖ u64 plaintext_length ‖ digest plaintext_digest ‖ bytes(65536)
logical_metadata ‖ blob bytes`. `logical_metadata` is canonical JSON and carries
only logical fields such as `mime_type` and `original_filename`. The plaintext
digest and metadata exist **only** inside the encrypted object.

**Sealing.** The key is the per-artifact object key of C.3; the nonce is twelve
zero bytes; the AAD is `label "dohflow/sync-artifact-aad/v1" ‖ u16
protocol_version ‖ uuid vault_id ‖ u64 key_epoch ‖ digest sync_artifact_id`. The
nonce is safe because the key is unique per artifact and is invoked exactly once;
there is exactly one canonical ciphertext per address. An exact retry sends the
same bytes, and a different byte string for the same address fails closed.

**Fixed plaintext (normative).** The zero nonce is safe only if the transport
plaintext is a fixed function of `(vault_id, key_epoch, sync_artifact_id)` for the
life of the epoch. A client therefore fixes `logical_metadata` at the artifact's
first publication in that epoch and persists those exact transport bytes (or the
descriptor that rebuilds them byte for byte), and it **never seals a second,
different plaintext under the same `(key_epoch, sync_artifact_id)`**. It refuses
before encryption, with the typed error `ArtifactDescriptorChanged`. A later
change to a row's filename or MIME type updates only the class-1 attachment row
(which Sync already replicates as a field) and never re-seals the artifact; the
object keeps its original descriptor until the next epoch re-seals it under a new
key. The container is `magic "DFSA"`, `u16 container_version = 1`, `u64 ciphertext_len`,
then the ciphertext.

**Never service-visible.** The origin `StorageId`, wrapped or content-key fields,
content nonces, blob paths, and any bare plaintext digest never appear in a
service-visible field. The service sees only vault routing metadata, the opaque
`sync_artifact_id`, the key epoch, the ciphertext length, and the ciphertext; the
artifact kind and logical metadata stay inside the encrypted object.

**Receiver materialization.** A receiver verifies the outer AEAD, then the
descriptor's length and plaintext digest, **before** it materializes anything. It
then derives its own `StorageId_B` under its DEK, generates a fresh content key,
re-encrypts the bytes, wraps the key under its DEK, writes the blob and row
atomically, and records only a device-local mapping from `sync_artifact_id_e` to
the local identity. A failure is `ArtifactMaterializationFailed`, leaves the live
vault unchanged, and discards temporary plaintext and ciphertext. A received
canonical reference keeps the origin's `sync_artifact_id_e`.

**Receipts and pins.** `ArtifactReceiptV1` is `label "dohflow/sync-receipt/v1" ‖
uuid vault_id ‖ u64 key_epoch ‖ digest sync_artifact_id ‖ u64 ciphertext_length ‖
digest ciphertext_sha256`. It is a durability acknowledgement bound to the
address and ciphertext, with **no service signature** in v1: its integrity comes
from the client re-verifying the outer AEAD whenever it fetches. A pin is the
service's record that a retained snapshot or envelope references an artifact; the
opaque manifest sidecar the service stores is the ordered id list whose
commitment the header carries, and a client rejects a sidecar whose commitment
differs from the decrypted manifest. Upload precedes reference, and reachability,
retention and garbage collection follow ADR 0074 Decision 1; `personal-cfo-v98vd`
implements them.

**Key-changing epochs.** At a revocation cutover or a forced rotation a client
re-addresses and re-seals every live artifact under the new epoch before the
replacement snapshot is accepted.

## C.10 Accepted-envelope index and retry protocol

The service's accepted-envelope index is authoritative and recovery-durable, and
survives tail truncation, snapshot bootstrap and restart. Each record is
`label "dohflow/sync-accepted/v1" ‖ uuid vault_id ‖ uuid command_id ‖ digest
idempotency_key_tag ‖ digest canonical_envelope_digest ‖ digest
immutable_metadata_commitment ‖ u64 key_epoch ‖ u64 seq ‖ opt(bytes(64))
result_ref`. It holds no plaintext key and no payload, and the exact original
command metadata is bound by `immutable_metadata_commitment`. A present `result_ref` is 1 to
64 bytes of opaque, service-chosen data (an empty one is refused), and the index holds at most
one record per `command_id` and one per tag. The service computes `canonical_envelope_digest`
itself from the received header bytes and ciphertext; a pusher's claimed digest is never
trusted.

On a push the service decides in this order:

1. look up **both** `command_id` and the tag in the index;
2. if both match one record with the same envelope digest and commitment, return
   that record's `seq` and `result_ref`, even when the request's `base_seq` is
   stale and even when its `key_epoch` is old;
3. if either lookup finds a different bound command, tag, digest or commitment, or
   the two lookups disagree, reject with `ProtocolFork`;
4. only if both identities are unknown, reject an old `key_epoch` (`EpochRejected`),
   then apply the compare-and-swap rule on `base_seq`.

A client that sees a `seq` for a command it still holds in its outbox binds the
outbox record to that `seq` and result and never replays or requeues it. A
same-identifier, different-envelope response is `ProtocolFork`, and both
envelopes are retained for diagnosis.

### Void records

A log position can be *poisoned*: an envelope that authenticates but fails the receiver checks
(C.4), or that passes them and then fails to apply deterministically. The service cannot see
this, so every replica stops there (`AuthenticatedEnvelopeInvalid`). A **void record** is how a
vault moves on. It is a signed record in the style of C.6, not a sealed object: it carries no
payload, only identifiers the service already holds, so the service can order it, verify its
claims against its own index, and mark the index.

Body: `label "dohflow/sync-void/v1" ‖ u16 protocol_version ‖ u16 suite_id ‖ uuid vault_id ‖
u64 seq ‖ uuid command_id ‖ digest envelope_digest ‖ u8 reason ‖ fingerprint signer_fingerprint ‖
i64 issued_at`. `seq` is the poisoned position `N`; `command_id` and `envelope_digest` are those
of the envelope the signer saw there; `reason` is `1` (receiver-check failure) or `2`
(deterministic apply failure). The record is `u32 body_len ‖ body ‖ 64-byte Ed25519 signature`,
identified by `SHA-256(body)`, with the strict acceptance rules and the check order of C.6. The
log position `V` the service assigns when it appends the void is not in the body, exactly as a
`seq` is not in an envelope's header.

- **Who may sign.** An authorized, unrevoked member of the chain at the epoch in force, verified
  under the key the chain holds for `signer_fingerprint`. The signature is **not authority over
  the outcome**: it makes the decision attributable, gives replicas a rollback trigger, and keeps
  spam from non-members out. What makes a replica skip a position is its own rejection of it.
- **What the service checks.** The signature and the signer (an authorized member at the
  membership head in force at the position `V` it assigns); that the vault matches; and that `seq`
  is an existing command-envelope position and that `command_id` and `envelope_digest` equal the
  index's record for it. It dedupes only an identical void (the same body id) or a second void for
  the exact same `seq`. It cannot check that the envelope is actually invalid (that needs the
  plaintext) and does not try. **It never refuses a void because an earlier void's range would
  cover it**: whether a void is honored, and so what a range covers, is decided only by replicas.
  A malicious member can append a void for a *valid* envelope; replicas ignore it, and a later
  genuine void for a poisoned position inside that range must still be accepted and honored.
- **When a replica honors it.** Only if the replica has itself evaluated position `seq`, in log
  order, found it invalid, and the void's `command_id` and `envelope_digest` equal what it saw
  there. The `reason` byte is for audit and diagnosis: a replica that found `seq` invalid for any
  deterministic reason honors the void, because it cannot apply `seq` either way. Otherwise the
  void is inert (retained for diagnosis, nothing skipped). So a malicious member cannot void a
  valid envelope, and spam does nothing. A replica stopped at `N` has evaluated `N` before it can
  read a void positioned after it. The signer must be an authorized member at the membership head
  in force at the void's position `V` (C.7 gives each position its epoch).
- **Range.** An honored void at position `V` skips every command envelope at positions `seq`
  through `V - 1`: later objects were built on a base that included `seq`. Membership and epoch
  records are never skipped (C.6 verifies them independently); an epoch record inside the range
  still takes effect for keys. A **snapshot** inside the range (including the snapshot coupled to
  a key-changing epoch) was built on poisoned state: it is never a valid bootstrap root, and a
  bootstrapping device uses one before `seq` or one at or after `V` that records the void. A void
  whose `seq` an earlier **honored** void already covers is inert, so the range is exactly the
  first honored void's; an earlier void that was itself inert (it named a valid envelope, or one
  the replica had not yet evaluated) covers nothing. A void for a position at or after `V` is an
  independent poisoning and is honored on its own.
- **No snapshot and no cutover while halted.** A replica stopped at a poisoned position publishes
  no snapshot, performs no key-epoch cutover and pushes nothing.
- **The index.** The accepted-envelope index (above) keeps the record of every position a void
  names and records **"void claimed by `<void id>`"** against that position only. The service
  cannot know whether a void was honored, so it never says a position *is* voided: an exact replay
  returns the original `seq` plus any void claims, and the client decides from its own verdict,
  rolling back and re-issuing only if it honored that void (after an inert void on a valid
  envelope, which every replica applied, the claim is just a claim). A replica never re-applies a
  position it skipped. A claimed position's `command_id` and idempotency-key tag are never
  reusable, either way.
- **Re-issue.** The issuer of every voided command rolls it back through rebase (ADR 0074
  Decision 2) and re-issues it as a **new command with a new `command_id` and a new idempotency
  key** (reusing the key is a `ProtocolFork`), through the producer rule of C.4.
- **Evidence.** The poisoned ciphertext and the void are retained, not garbage-collected before a
  snapshot that supersedes the range, and a snapshot taken past a void records the voided ranges
  and the void ids in its metadata.

## C.11 Typed Sync failures

`NonceStateLost`, `ProtocolFork`, `InvalidIdempotencyKey`, `MalformedObject`
(any CBE1 or container violation), `UnsupportedProtocol`, `UnsupportedSuite`,
`EpochRejected`, `EpochRecordInvalid`, `KeyCommitmentMismatch`,
`MembershipChainInvalid`, `NoAuthorizedDevice`, `SnapshotTooLarge`,
`StaleMembershipHead`, `BadEpochSequence`, `CutoverHeadNotRevoke`,
`GrantHeadNotAdd`, `EpochRecordMissing`, `EpochEquivocation`,
`MembershipChainFork`, `SyncHalted`, `StateRolledBack`, `ArtifactDescriptorChanged`,
`ArtifactMaterializationFailed`, `ArtifactRebootstrapRequired`,
`AuthenticatedEnvelopeInvalid` (an envelope that authenticates but fails the receiver checks at a
log position, C.4 "Push and producer rules"; neither a fork nor retryable) and
`ArtifactMissing` (a closure that names an absent object). Each fails closed,
mutates nothing, and retains diagnostic evidence.

## C.12 Portable bundle container

The portable export of Part B is protected by the vault password alone, with no
separate export secret, as an encrypted container. This applies the owner's
decision on the bundle, including the conditions of the adversarial review.

### Header

`magic "DFPX"` (4 ASCII bytes), `u16 container_version = 1`, `u32 header_len`,
then the header, which is also the manifest's authenticated context:
`label "dohflow/export-header/v1" ‖ u16 container_version ‖ u8 kdf_profile ‖ u32
memory_kib ‖ u32 time_cost ‖ u32 parallelism ‖ 32 raw bytes salt ‖ u32
object_count ‖ u32 chunk_size`. `chunk_size` is `1048576`.

- **KDF allowlist.** `kdf_profile` is exactly `1` (Argon2id, 65,536 KiB, time 3,
  parallelism 1) or `2` (Argon2id, 262,144 KiB, time 4, parallelism 1). A writer
  uses at least profile `1` and **never** the legacy 19 MiB profile. A reader
  accepts only those two, requires the stored three values to equal the profile's
  exact values, and rejects anything else **before** it spends memory on the KDF.
- **Fresh salt.** The 32-byte salt is random and fresh for every export, never the
  vault envelope's salt. A retry draws a new salt and therefore a new header hash.

### Keys

`KEK = Argon2id` (version `0x13`) over the password, the salt and the profile
parameters, with a 32-byte output. With
`header_hash = SHA-256(header bytes)`, object `i` (counting the manifest as
object 0) uses the key `Expand(KEK, label ‖ digest header_hash ‖ u32 i)`, where
the label is `"dohflow/export-manifest-key/v1"` for `i = 0` and
`"dohflow/export-object-key/v1"` for `i >= 1`. **Each derived key seals exactly
one plaintext** under the fixed zero nonce.

### Objects

Every logical file is split into chunks of at most `chunk_size` plaintext bytes;
each chunk is one object, so a large document is chunked rather than sealed whole.
A zero-length logical file is one object: an empty final chunk (`chunk_index` 0,
`final` 1, `plaintext_length` 0).
After the header the container holds `object_count` objects, each `u32 length`
then ciphertext. `object_count` is at least 1 and at most `2^20`. Object 0 is the
canonical-JSON `manifest.json` of Part B: it is a single object, never chunked,
with a plaintext of at most `2^28` bytes. The AAD of object `i` is `label
"dohflow/export-object-aad/v1" ‖ digest header_hash ‖ text(255) path ‖ u32 index ‖
u32 chunk_index ‖ u8 final ‖ u64 plaintext_length`, which binds the header, path,
position and length. `chunk_index` counts the chunks of one path from 0, and
`final` is `1` for the last chunk of a path and `0` otherwise. The manifest's
own AAD uses path `manifest.json`, index `0`, chunk index `0` and `final` `1`.

The manifest **commits the object set**: it lists the object count and, for every
object after the manifest, its index, path, chunk, ciphertext length and
ciphertext SHA-256, and a reader enforces the exact set (an object not listed, or
a listed object absent, is `Corrupt`). The manifest's `object_count` equals the
header's, and the manifest lists exactly the objects `1` to `object_count - 1`.

### Outcomes

A malformed header or container is `Corrupt`. A failure to authenticate object 0
is `WrongPassword` (it can also mean a tampered header, which the AAD binds). A
failure on any later object, or a mismatch with the manifest, is `Corrupt`. The
typed outcomes of Part B apply unchanged.

## C.13 Compatibility and change control

- A reader supports an explicit, documented range of `protocol_version`,
  `container_version` and `suite_id` values and refuses the rest with a typed
  error. No version promises that every future version opens.
- Any new field, label, kind or registry value is a new version of its structure,
  and a changed derivation is a new label version (`/v2`), never a silent edit.
- The first bytes of every clear header and container are frozen across all protocol
  versions: a header starts with `label "dohflow/sync-header/v1" ‖ u16 protocol_version`
  and a container with `magic "DFSY" ‖ u16 container_version`. A newer layout changes
  `protocol_version` or `container_version` and leaves those prefixes alone, so an older
  reader reports `UnsupportedProtocol` or `UnsupportedContainer` (an update-required
  state) instead of `MalformedObject`. A reader checks the version before it trusts any
  later field.
- `engine_version` is checked as it is parsed: a header whose value is not 1 to 64
  bytes of printable ASCII is `MalformedObject`.
- A change to this part bumps `spec-version` and requires new reviewed vectors.
- `engine_version` and `schema_version` skew is handled by ADR 0074 (R40): the
  client asks the user to update before continuing.
- `engine_version` is opaque printable ASCII (`0x21..=0x7e`), 1 to 64 bytes, compared byte
  for byte for equality and never ordered. It names the **apply semantics**, not the
  release: two builds with the same value produce the same rows and the same write-set hash
  from the same payload, metadata and base state. DohFlow writes `dohflow-engine-N` for a
  positive integer N, and increments N, in the same change, whenever a change to `apply`
  alters what it writes for an unchanged payload and base. A replica that receives an
  envelope or snapshot whose `engine_version` it does not equal is in the update-required
  state; it never guesses compatibility. (A vault reading back its own operation-log rows
  is not gated by this.) The value
  an operation-log row stores is the one the producing build wrote, unchanged. (The first
  value is `dohflow-engine-1`.)

## C.14 Conformance tests and vectors

The membership chain and its epoch records have their own **public conformance vectors**,
`docs/architecture/vectors/membership-chain-v1.json` (`vectors_version` 1): valid chains
(genesis only, add with a grant, revoke with a cutover, forced rotation, incremental extension,
idempotent duplicates) and invalid ones (unsigned and mis-signed records, signer not a member or
revoked, wrong version or `prev_id`, forks, fingerprint, device-id and public-key reuse, revoke
that does not repeat its add, revoke of the last member, small-order and non-canonical keys and
signature parts, missing, mis-ordered or equivocating epoch records, wrong recipients, a reused
key, a changed `S`, unsupported protocol and suite). Each case is a list of steps and each step
states the outcome as the C.11 failure name and the position of the offending record, never an
implementation's own error names. The file holds only public data; the seeds that produced it live
only in the Rust generator (`crates/sync-core/tests/membership_vectors.rs`), which fails if the file
drifts. A change to the file is a change to this part and follows C.13.

The reference test recomputes every vector below and fails if the document and
the code differ. Every input is synthetic and listed here, so a third
implementation can reproduce each vector from this part alone. A repeated byte is
written `N x 0xNN`.

| Input | Value |
| --- | --- |
| `vault_id` | 16 x `0x11` |
| Protocol constants | `protocol_version` 1, `suite_id` 1, `kem_id` `0x0020`, `key_epoch` 1 |
| Device A | `device_id` 16 x `0xa1`; Ed25519 public key 32 x `0xe1`; KEM (X25519) public key 32 x `0xc1` |
| Device B | `device_id` 16 x `0xb1`; Ed25519 public key 32 x `0xe2`; KEM (X25519) public key 32 x `0xc2` |
| `K_e` (epoch 1) | the 32 bytes `00 01 02 ... 1f` |
| `K_e` (epoch 2, used only by `artifact_id_epoch2`) | the 32 bytes `40 41 42 ... 5f` |
| `S` | the 32 bytes `20 21 22 ... 3f` |
| Idempotency key | `4a3b2c1d-9e8f-4a7b-8c6d-5e4f3a2b1c0d` as 16 raw bytes |
| Command metadata | `command_id` `01938f2e-7c1a-7b3d-9a4e-123456789abc`; `correlation_id` 16 x `0x22`; `causation_id` absent; `actor_type` 1; `actor_id` `device-a-user`; `issued_at` 1_790_000_000_000_000; `device_id` = device A |
| Sample header fields | `base_seq` 41; `payload_schema_version` 1; `engine_version` `dohflow-0.3.0`; `schema_version` 55; `invocation_counter` 0; the snapshot's `object_id` 16 x `0x33`; `nonce_domain` and fingerprint as derived for device A |
| Envelope plaintext | `write_set_hash` 32 x `0x44`; payload `{"amount_minor":"-1250","currency":"USD"}`; the delta manifest is the two sample artifact ids in ascending byte order |
| Local storage ids | 32 x `0x5a` and 32 x `0x6b`; the manifest commitment covers the two epoch-1 artifact ids |
| Sample artifact | `artifact_kind` 1; plaintext `synthetic document` (18 bytes); `logical_metadata` `{"mime_type":"text/plain"}`; the address and keys of the first storage id at epoch 1 |
| Void | `seq` 42; `command_id` and `envelope_digest` those of the sample envelope; `reason` 1; signer device A; `issued_at` 1_790_000_003_000_000 |
| Membership | record 1: `version` 1, `prev_id` absent, `action` 1 (add), signer and added device A, `issued_at` 1_790_000_001_000_000; record 2: `version` 2, `prev_id` the id of record 1, `action` 1, signer device A, added device B, `issued_at` 1_790_000_002_000_000 |
| Epoch records | genesis sample (`epoch_header_genesis`): `reason` 1, `key_epoch` 1, `membership_head` the id of membership record 1, `prev_epoch_id` absent, sealer device A. Grant sample (`epoch_header`, `hpke_info_recipient_b`, `hpke_aad_recipient_b`): `reason` 3, `key_epoch` 1, `membership_head` the id of membership record 2, `prev_epoch_id` 32 x `0x99` (the id of a genesis record whose body no vector computes), sealer device A, recipient device B |
| Write-set sample | rows `(ledger_transactions, 16 x 0x01, 32 x 0x51)`, `(accounts, 16 x 0x02, 32 x 0x52)`, `(accounts, 16 x 0x01, 32 x 0x53)` |
| Export | password `synthetic-export-password`; profile 1; salt 32 x `0x77`; `object_count` 2; `chunk_size` 1048576 |
| Export objects | object 0 path `manifest.json`, index 0, chunk 0, final 1; object 1 path `data/accounts.jsonl`, index 1, chunk 0, final 1, plaintext `{"id":"1","name":"Checking"}` followed by a line feed (29 bytes) |
| Export manifest values | `app_version` `dohflow-0.3.0`; `schema_version` `"55"`; `inventory_version` `"1"`; `object_count` `"2"`; `artifacts` empty; one `tables` entry with `name` `accounts`, `class` `"1"`, `row_count` `"1"`, `sha256` the SHA-256 of the row file's 29 bytes, and `columns` `id` with `kind` `text` and `name` with `kind` `text`; one `objects` entry for object 1 (`index` `"1"`, `chunk` `"0"`, `path` `data/accounts.jsonl`, with the ciphertext's length and SHA-256) |

The vectors `aead_key_a_envelope` and `aead_key_b_envelope`, and
`nonce_domain_a_envelope` and `nonce_domain_b_envelope`, are computed from the
**same** `K_e` and two device fingerprints: they differ, so two devices sealed the
same epoch key by a grant never share an AEAD key or a nonce domain.

The 199-byte HPKE `info` should be confirmed against the HPKE crate that
`personal-cfo-1cltt` selects. The grant sample's `prev_epoch_id` is a declared
placeholder, so the epoch-chain link has no byte vector here; the implementing
bead's real-hash tests cover it.

Beyond the byte vectors, the reference test enforces: rejection of trailing
bytes, truncation, a wrong label, an oversize length, a bad presence byte, invalid
UTF-8, an unsupported version and a counter above the cap; the dual-identity field
rules per object type; the UUIDv4-only idempotency rule; tamper of the clear tag
or commitment failing authentication; distinct AEAD keys for devices with equal
counters even when a hostile service offers overlapping nonce domains;
`NonceStateLost` on a register rollback and a forced rotation at the cap; the
service backstop accepting only exact retries; unlinkable artifact addresses
across epochs and vaults; an order-independent, exact manifest commitment and its
presence rule (absent for an envelope with no artifacts, always present for a
snapshot); strictly ascending manifest lists; the raw-byte ordering of
`write_set_hash`; a fingerprint that covers the suite, the KEM id and a
variable-length key; the freshness, sequencing and catch-up rules of C.6 (a stale-head anchor from a
since-revoked member; catch-up across two cutovers and across a cutover followed by
an add; a forged intermediate and a non-newest anchor; the reason and head
coupling; a membership record with no epoch record; a forced rotation; equivocation
at the newest position and at a genesis position; a forked chain with its recovery;
a rolled-back state, an add racing a cutover whose grant seals the pre-cutover key,
and its retry at the new head; a forced rotation that moves the membership head
(refused, so an add or revoke cannot bypass its coupled grant or cutover); the
epoch-position rule with its exact boundary (an envelope at `seq` equal to the
position belongs to the old epoch); exact 32-byte suite-1 key lengths with a layout
that still carries longer keys; and duplicate write-set rows (refused);
refusal, before encryption, of a changed descriptor for a published artifact
address; rejection of an altered artifact object or a wrong address; HPKE `info`
binding every recipient, epoch, sealer, membership head and record header; and the
export header's exact KDF allowlist, object-count bound and per-object AAD binding.

The implementing beads must add, with the crates they choose, these tests that
the reference cannot run: the RFC 8032 and RFC 9180 known-answer vectors;
`verify_strict` rejection of small-order and non-canonical signatures; a forged,
unsigned sealed-key set; a replayed old-epoch sealed blob; a record that seals
different keys to different recipients; a membership chain with a gap, a fork, a
reused fingerprint and a revoked signer; the stale-head record signed by a
since-revoked member, the catch-up walk and the equivocating epoch records, with
real signatures and real hashes of full bodies; an add whose `prev_id` is stale
refused by the service, and the grant that seals the pre-cutover key refused by a
recipient (`personal-cfo-v98vd` and `personal-cfo-1cltt`);
a changed artifact descriptor refused before encryption, end to end;
cross-implementation canonical-JSON vectors for 64-bit integer strings, duplicate
keys and lone surrogates; and the revocation-cutover simulator suite extended with
the F5 retry-order case.

## Vector listing

Each line below is one vector as lowercase hex, in the order the reference test
computes them. A name's meaning is the derivation of the same name in C.3 to
C.12: for example `header_envelope` is the clear header of the sample envelope,
`envelope_container` is the complete wire object, and `export_manifest_json` is
the canonical manifest whose bytes the export vectors seal.

<!-- sync-vector: fingerprint_a=d3804ba59759f53af251f158f18364a228fa5c5db0f31edd8fcb5d4fde71c608 -->
<!-- sync-vector: fingerprint_b=388483da46033bcf0263edfdbb306bb56114a32c56ebb500cc52813227fdff17 -->
<!-- sync-vector: aead_key_a_envelope=0c5c10840d720abfc446d20b9f510c3c064583c62fd2ace6a6911347a3e7b5f6 -->
<!-- sync-vector: aead_key_b_envelope=98327c534570f357b1ddffb66ef9ea12fea8e53420dc4067a3d013ffb452dde0 -->
<!-- sync-vector: aead_key_a_snapshot=fc9bee5ab472d6afc355b6f6e943f0877b7bc3799b9e9e8836a106bc89e21f8c -->
<!-- sync-vector: nonce_domain_a_envelope=03f7cf26 -->
<!-- sync-vector: nonce_domain_b_envelope=7051fcce -->
<!-- sync-vector: nonce_a_envelope_counter0=03f7cf260000000000000000 -->
<!-- sync-vector: nonce_a_envelope_counter1=03f7cf260000000000000001 -->
<!-- sync-vector: idempotency_tag_key=4404ee3c509bd231d87b0adc99c1e40ad3da12199898bb827e34d6f21c890e3f -->
<!-- sync-vector: idempotency_tag=06fa9d92e5203243401aa8c29e960bb67d25c7f9b7d148279ce2ad2fea1a5e1b -->
<!-- sync-vector: immutable_metadata_commitment=885a1199f22775ad785e88bac193d96ef2c4ba0d84665d83c77ae72035b68053 -->
<!-- sync-vector: key_commitment_ke=c7c2f442f9197f13cd691a7ced1edf498d263a16fd0d0604091294e616fb9eb0 -->
<!-- sync-vector: key_commitment_s=4576446220850b7883789181f26831271e57bc082d0a24074466a53a21405644 -->
<!-- sync-vector: artifact_id_epoch1=d250092c040686d891c21151a01b44a593e22451b71f872fbf367c9327fd90de -->
<!-- sync-vector: artifact_id_epoch2=b8532c2b8f48fc07410d0fcaab93293882b32a0dc4cbb6d9263ddd4d5b708273 -->
<!-- sync-vector: artifact_manifest_commitment=e9f048d0e627a918249c19bd2df94315be7373469a5af23869cd32c65cd88631 -->
<!-- sync-vector: write_set_hash_sample=5e2ef15d19742bcf1f39b0071f115a9c5b1f427f11eb3eb3468cf08bbdfe3f11 -->
<!-- sync-vector: artifact_object_key=57441986f4b6610011f5efa0aa280eb3dd5f415d6275d4e823e03048851944e1 -->
<!-- sync-vector: artifact_aad=001c646f68666c6f772f73796e632d61727469666163742d6161642f76310001111111111111111111111111111111110000000000000001d250092c040686d891c21151a01b44a593e22451b71f872fbf367c9327fd90de -->
<!-- sync-vector: artifact_object_ciphertext=f1f50189e96ae26161ba2f22e85de9e62984dcab08765606008cca460cc1fb9196f03d2e5812253729b9deb7ea32b967dba157284121dd8874b4594124d7db5a4b935776569576d048cbd279d2086033abafa954db60a99b90f37fcdc750299eaccf4906b3a9c7203ba567abfbd4c32d2e550266430b335d6c540338abfbb4f6a8e8bf06eec43c19e20b7446e292e714976718297e6deecfe928162de967c50141ffd6f4e76eccf2a7425b7b6f2765dd684badf75a -->
<!-- sync-vector: header_envelope=0016646f68666c6f772f73796e632d6865616465722f763100011111111111111111111111111111111100000000000000010101938f2e7c1a7b3d9a4e123456789abc0106fa9d92e5203243401aa8c29e960bb67d25c7f9b7d148279ce2ad2fea1a5e1b01885a1199f22775ad785e88bac193d96ef2c4ba0d84665d83c77ae72035b68053a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1d3804ba59759f53af251f158f18364a228fa5c5db0f31edd8fcb5d4fde71c60803f7cf2600000000000000000000000000000029000000010000000d646f68666c6f772d302e332e300000003701e9f048d0e627a918249c19bd2df94315be7373469a5af23869cd32c65cd88631 -->
<!-- sync-vector: header_snapshot=0016646f68666c6f772f73796e632d6865616465722f7631000111111111111111111111111111111111000000000000000102333333333333333333333333333333330000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1d3804ba59759f53af251f158f18364a228fa5c5db0f31edd8fcb5d4fde71c608fc89840d00000000000000000000000000000029000000010000000d646f68666c6f772d302e332e300000003701e9f048d0e627a918249c19bd2df94315be7373469a5af23869cd32c65cd88631 -->
<!-- sync-vector: envelope_plaintext=0022646f68666c6f772f73796e632d656e76656c6f70652d706c61696e746578742f763101938f2e7c1a7b3d9a4e123456789abc2222222222222222222222222222222200010000000d6465766963652d612d757365724a3b2c1d9e8f4a7b8c6d5e4f3a2b1c0d00065bfeda25e000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a144444444444444444444444444444444444444444444444444444444444444440000000233a46ad663dbac24a2db72929a54f8d144a68eb05f092048e4952a187c02d0b3d250092c040686d891c21151a01b44a593e22451b71f872fbf367c9327fd90de000000297b22616d6f756e745f6d696e6f72223a222d31323530222c2263757272656e6379223a22555344227d -->
<!-- sync-vector: envelope_ciphertext=269513d4bfcfeed5008ee354fe9fb09a4c87f5fb50f5ad2dfdf339b51f68d87fe7c4bc4b71e2df9aa4a90c49dd0235908081d9552773d5b820257dccca1abdf35763f16fbe8313bded923fe001c9b413b7ebbfcb39b2d7a97720992fca3b2f62c886f6854422267face5c2c68598cdafba37d9ec070967180e3c12baaa495002c9bb98c824c03a52fb8e3820f9d712cd1fa2633fbd38c02390cb80bac93f8192a0ada76eb353e300b5f9239c99c9987e8d369f0c3bc5351283db45172723d044efa9554ecf8ee6a3b519142aa991f2ac2b497786e22216f035e8763d699d4db37f57493990bbd86ce94c1fd1ca31a07f9b243c4072559eb112dcf5b03332e108da208543c1ab02ebc4f76fabd720246f597a557f04e71b3ef35ead78b5834efd -->
<!-- sync-vector: envelope_digest=fb917cb47e6be62c9e6858395376a24807af5af3151b998fabc0c12db03e80a5 -->
<!-- sync-vector: envelope_container=444653590001000001030016646f68666c6f772f73796e632d6865616465722f763100011111111111111111111111111111111100000000000000010101938f2e7c1a7b3d9a4e123456789abc0106fa9d92e5203243401aa8c29e960bb67d25c7f9b7d148279ce2ad2fea1a5e1b01885a1199f22775ad785e88bac193d96ef2c4ba0d84665d83c77ae72035b68053a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1d3804ba59759f53af251f158f18364a228fa5c5db0f31edd8fcb5d4fde71c60803f7cf2600000000000000000000000000000029000000010000000d646f68666c6f772d302e332e300000003701e9f048d0e627a918249c19bd2df94315be7373469a5af23869cd32c65cd886310000000000000120269513d4bfcfeed5008ee354fe9fb09a4c87f5fb50f5ad2dfdf339b51f68d87fe7c4bc4b71e2df9aa4a90c49dd0235908081d9552773d5b820257dccca1abdf35763f16fbe8313bded923fe001c9b413b7ebbfcb39b2d7a97720992fca3b2f62c886f6854422267face5c2c68598cdafba37d9ec070967180e3c12baaa495002c9bb98c824c03a52fb8e3820f9d712cd1fa2633fbd38c02390cb80bac93f8192a0ada76eb353e300b5f9239c99c9987e8d369f0c3bc5351283db45172723d044efa9554ecf8ee6a3b519142aa991f2ac2b497786e22216f035e8763d699d4db37f57493990bbd86ce94c1fd1ca31a07f9b243c4072559eb112dcf5b03332e108da208543c1ab02ebc4f76fabd720246f597a557f04e71b3ef35ead78b5834efd -->
<!-- sync-vector: void_body=0014646f68666c6f772f73796e632d766f69642f76310001000111111111111111111111111111111111000000000000002a01938f2e7c1a7b3d9a4e123456789abcfb917cb47e6be62c9e6858395376a24807af5af3151b998fabc0c12db03e80a501d3804ba59759f53af251f158f18364a228fa5c5db0f31edd8fcb5d4fde71c60800065bfeda53a6c0 -->
<!-- sync-vector: void_id=15c06220b78a3158a8375eaaaa8f38d02a15af57bbc9a787359fa78546df1beb -->
<!-- sync-vector: membership_body_1=001a646f68666c6f772f73796e632d6d656d626572736869702f76310001000100201111111111111111111111111111111100000000000000010001d3804ba59759f53af251f158f18364a228fa5c5db0f31edd8fcb5d4fde71c608a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e100000020c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c100065bfeda352240 -->
<!-- sync-vector: membership_id_1=453a0ac4429a0e0a7d7e8b425defb17fe6f56833361f1a07e9290725bf9e82a0 -->
<!-- sync-vector: membership_body_2=001a646f68666c6f772f73796e632d6d656d626572736869702f763100010001002011111111111111111111111111111111000000000000000201453a0ac4429a0e0a7d7e8b425defb17fe6f56833361f1a07e9290725bf9e82a001d3804ba59759f53af251f158f18364a228fa5c5db0f31edd8fcb5d4fde71c608b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e200000020c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c200065bfeda446480 -->
<!-- sync-vector: membership_id_2=1e1545a55b67c5e70ab2a771b9a6d090f53ba38dc4f2b001bae9443697e73a3e -->
<!-- sync-vector: epoch_header_genesis=001c646f68666c6f772f73796e632d65706f63682d7265636f72642f763100010001002011111111111111111111111111111111000000000000000101453a0ac4429a0e0a7d7e8b425defb17fe6f56833361f1a07e9290725bf9e82a000c7c2f442f9197f13cd691a7ced1edf498d263a16fd0d0604091294e616fb9eb04576446220850b7883789181f26831271e57bc082d0a24074466a53a21405644d3804ba59759f53af251f158f18364a228fa5c5db0f31edd8fcb5d4fde71c608 -->
<!-- sync-vector: epoch_header=001c646f68666c6f772f73796e632d65706f63682d7265636f72642f7631000100010020111111111111111111111111111111110000000000000001031e1545a55b67c5e70ab2a771b9a6d090f53ba38dc4f2b001bae9443697e73a3e019999999999999999999999999999999999999999999999999999999999999999c7c2f442f9197f13cd691a7ced1edf498d263a16fd0d0604091294e616fb9eb04576446220850b7883789181f26831271e57bc082d0a24074466a53a21405644d3804ba59759f53af251f158f18364a228fa5c5db0f31edd8fcb5d4fde71c608 -->
<!-- sync-vector: epoch_header_hash=7ad4d10215dae6361f512d0f37da58bd87344f957c75ba0af0e2522aedaee206 -->
<!-- sync-vector: hpke_info_recipient_b=0019646f68666c6f772f73796e632d68706b652d696e666f2f763100010001111111111111111111111111111111110000000000000001b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1388483da46033bcf0263edfdbb306bb56114a32c56ebb500cc52813227fdff17d3804ba59759f53af251f158f18364a228fa5c5db0f31edd8fcb5d4fde71c6081e1545a55b67c5e70ab2a771b9a6d090f53ba38dc4f2b001bae9443697e73a3e7ad4d10215dae6361f512d0f37da58bd87344f957c75ba0af0e2522aedaee206 -->
<!-- sync-vector: hpke_aad_recipient_b=0018646f68666c6f772f73796e632d68706b652d6161642f76317ad4d10215dae6361f512d0f37da58bd87344f957c75ba0af0e2522aedaee206388483da46033bcf0263edfdbb306bb56114a32c56ebb500cc52813227fdff17 -->
<!-- sync-vector: sealed_keys_plaintext=001b646f68666c6f772f73796e632d7365616c65642d6b6579732f7631000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f -->
<!-- sync-vector: export_header=0018646f68666c6f772f6578706f72742d6865616465722f763100010100010000000000030000000177777777777777777777777777777777777777777777777777777777777777770000000200100000 -->
<!-- sync-vector: export_header_hash=d4f9f4348e70fd109ac7c4fd76ff4ad83bf4744c0f42544904ecaf017544d67b -->
<!-- sync-vector: export_kek=2fe8f3c867e80cbdc7461e1b03743c20dab7db956f8bd4eef8f822953ee3b84b -->
<!-- sync-vector: export_manifest_key=bf5c481f4725ae2c0610cd1b9d5eb9e92e4b7f438f4661e9c3bb274bf96c8674 -->
<!-- sync-vector: export_object_key_1=481ef4530c89ba2764bc0d8e71e817c91921b928f126adcf4938cc15d9f54a0a -->
<!-- sync-vector: export_object_aad_1=001c646f68666c6f772f6578706f72742d6f626a6563742d6161642f7631d4f9f4348e70fd109ac7c4fd76ff4ad83bf4744c0f42544904ecaf017544d67b00000013646174612f6163636f756e74732e6a736f6e6c000000010000000001000000000000001d -->
<!-- sync-vector: export_object_ciphertext_1=e16256a8851d0f66d0d8360b4064c104cb83cb0242e4850f900b98a927230e2beec4c55f1c4be66873e4a82b69 -->
<!-- sync-vector: export_manifest_json=7b226170705f76657273696f6e223a22646f68666c6f772d302e332e30222c22617274696661637473223a5b5d2c22666f726d6174223a22646f68666c6f772d706f727461626c652d6578706f7274222c22666f726d61745f76657273696f6e223a2231222c22696e76656e746f72795f76657273696f6e223a2231222c226f626a6563745f636f756e74223a2232222c226f626a65637473223a5b7b226368756e6b223a2230222c22636970686572746578745f6c656e677468223a223435222c22636970686572746578745f736861323536223a2261366133396434316139656666333262653639626365376563373865376564666332376433383761396130376262366663653336666131356238646133316136222c22696e646578223a2231222c2270617468223a22646174612f6163636f756e74732e6a736f6e6c227d5d2c22736368656d615f76657273696f6e223a223535222c227461626c6573223a5b7b22636c617373223a2231222c22636f6c756d6e73223a5b7b226b696e64223a2274657874222c226e616d65223a226964227d2c7b226b696e64223a2274657874222c226e616d65223a226e616d65227d5d2c226e616d65223a226163636f756e7473222c22726f775f636f756e74223a2231222c22736861323536223a2266383134323961336333383632666137653463646361656439323462666339303439303931633466666566376537626230636364383439363065353233356562227d5d7d -->
<!-- sync-vector: export_manifest_ciphertext=ff7c9d0f8f9d38240cc360284be24fb83451202bbb6d0980af71e19cea06f1a518130d91aee2c3aa3546cf7a3f5e73f99ac01d83d59749450e1c8987e4619b790695149664649d88470b509912a26c6856d733c0a1a4376f2c0e0c19cde5274ddd21fa985a419210256dd739b4ae6baf14ad0d4438876baefa46439eb38b9f1e7d62bc3c7b3f813076b69151cd5520359c929660e13385896ea03532ed126d5cf9f5eb2da291f52bf8575653033fe8237a9605e1224969cab2808d280b05f90e881e909843a5fb4e03a3b8d3ff496af8c0937eec8563b2fed4f0bf3bb789b6221f743a111b0fa9fd7dc69cde6a2dd0ce63c0a69fa3dcad09f37e980d4e0e340aed9bcd9c1399b3d72e9b475661f7e3a836c7f768e32fece07186c55932876b7823c30d096369262cfa73347d0f42d6af0439553ce5483b4adac8ef0f6e6f6a2f68e8b56a89d3c49cf0ba26d267f6977ff2c567f84aa90f272d3ba412f7812ffa444b1a899a7d8f2fb63f0a2d73520a8cc45f4be1f8efc1c22eaf8e4e5e592f0fe1b5aebba7898c39cf729725b8ae63eff7f217908339c27f96d79876868d52121a04ba3d993b3cec88c695d628d709a684a7a038717907f95b9957193965b718ab72c30878b925118eecde79f33f13c94eb0949d63903e55db9e4099eb39fa1247ecf8e169f4a577c71208a619bc2b96895b0d0fbd2b1eb9fd4f393bde7e597e4fe4edba70eb99be4ce97ed90501c7e269782d65c9def1727dc69c592925437acad2d54a12f818a6f133b33911c3262fc3c7de671ba611 -->
