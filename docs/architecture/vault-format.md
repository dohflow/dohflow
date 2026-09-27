# Vault format — version 1 (shipped local vault and backup)

This is the public format authority for DohFlow data. Version 1 describes only
formats that the current application writes or reads. The portable logical
export and Sync wire formats are **not implemented**; their approved contracts
will be added in a separately reviewed version-2 revision under
`personal-cfo-klr.4`. Do not treat a SQLCipher database image or a `.pcfobk`
backup as a Sync snapshot.

The version pins below are checked against production code by
`crates/db-worker/tests/vault_format_spec.rs` and the backup integration tests.
Changing a persisted schema or envelope without updating this specification
fails CI. The [table-class manifest](../../crates/db-worker/table_classes.toml)
and its real-SQLCipher test are the exhaustive table and field inventory; this
document defines the meaning and compatibility of those bytes.

<!-- vault-format: spec-version=1 -->
<!-- vault-format: schema-version=52 -->
<!-- vault-format: envelope-version=1 -->
<!-- vault-format: backup-write-version=2 -->
<!-- vault-format: backup-read-versions=1,2 -->
<!-- vault-format: backup-manifest-version=1 -->

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
profile, control unlock. See [encryption design](../security/encryption-design.md)
for the supported profile floor and engine pin.

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

The current fully migrated schema is version **52**. The ordered migrations
in `crates/db-worker/src/migrations.rs` start from a consolidated v1 baseline;
subsequent versions are actual forward changes. `DbWorker::open` applies them
before exposing a usable worker. Each migration runs transactionally, records
its SQL content hash in `schema_migrations`, and advances SQLite
`PRAGMA user_version`. `vault_metadata.schema_version` advances on older
vaults; it is not lowered when a stored value is newer. Do not assume an older
app build safely opens a vault written by a newer one.
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
dump. Manual and scheduled export run only while the vault is unlocked and do
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

Restore verifies the manifest and all component hashes before installing a
fresh vault; it never overwrites an existing vault. The app's named-vault
restore route preserves the original vault and reports interrupted, unregistered
staging for diagnosis ([recovery guide](../user-guide/recover-a-vault.md)).
The backup currently needs DohFlow's restore implementation. Do **not** claim
that a user can decrypt this backup without a DohFlow binary or recover from a
lost password. The independent recovery tool/procedure for the future portable
logical export is a `personal-cfo-vn91` deliverable and must pass its separate
synthetic no-app recovery test before any such public claim.

## Compatibility and change control

- `spec-version = 1` covers current local and backup formats, not the planned
  portable or Sync formats. New wire sections require a spec-version bump and
  their own reviewed vectors.
- Envelope v1 is read/written today; a future layout must use a new envelope
  version and retain an explicit reader/migration policy.
- Backup v2 is written; backup v1 and v2 are read. The restore path refuses a
  newer backup schema before installation. Opening an existing live vault
  with an older app build has a separate unresolved downgrade-safety risk.
- The SQLCipher/SQLite engine pair is pinned in
  [stack.md](stack.md) and exercised by the cross-version vault test. A
  compatible reader must honor the exact engine and migration policy, not just
  parse the outer envelope.
- No real financial records, vault paths, credentials, or keys appear in this
  specification or its vectors.
