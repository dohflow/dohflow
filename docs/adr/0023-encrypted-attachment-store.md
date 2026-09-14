# ADR 0023: Encrypted attachment store

- **Status:** Accepted
- **Date:** 2026-06-20
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-b7jk`](../../.beads/issues.jsonl)
- **Related plan sections:** §6.4, §6.6
- **Supersedes:** None

## Context

The app must let users attach documents (statements, receipts, pay stubs) to
financial records. ADR 0002 §5 already fixes the *key model* — "each attachment
has a randomly-generated content key wrapped by the DEK; filenames stored only as
encrypted metadata; on-disk filenames are opaque storage IDs" — and notes
attachments are encrypted **separately** from the SQLCipher DB because multi-
megabyte binaries don't fit SQLite pages. This ADR records the *storage model*
that realizes that key model, governing implementation bead `personal-cfo-bcj`
(and, transitively, the backup arc, since `ef3` bundles this store).

This is foundation for the Week-8 gate's "no plaintext attachment artifacts
outside the vault" criterion. It is **storage only**; in-app *processing* of
attachment contents (OCR, preview rendering, parsing) is governed by ADR 0022
(parser/document isolation) and deferred with its P3 consumers.

## Decision

### 1. Content-addressed encrypted blob files (not DB BLOBs)

Each attachment's bytes are stored as a **separate AEAD-encrypted file** under
`<vault>/blobs/`, never as a SQLCipher BLOB. The vault directory is:

```
<vault>/
  vault.db            SQLCipher: all metadata, refs, wrapped keys
  blobs/
    9f3a…  (one file per blob; opaque name = storage ID; AES-256-GCM ciphertext)
    4c20…
```

Keeping large binaries out of `vault.db` avoids DB bloat, keeps WAL/VACUUM cheap,
and decouples blob I/O (streamable) from the transactional read-model.

### 2. Storage ID = keyed content hash (dedup without a plaintext-guessable name)

The on-disk filename (the **storage ID**) is
`HMAC-SHA256(addr_subkey, plaintext_bytes)`, hex-encoded, where `addr_subkey` is a
content-addressing subkey derived from the DEK (HKDF, fixed `"attachment-addr"`
context). This gives **dedup within a vault** (identical bytes → identical
storage ID → one stored blob) while ensuring the on-disk name is **not** a bare
content hash an attacker could match against guessed plaintext. Storage IDs are
opaque: no original filename, extension, or size is derivable from them.

### 3. Per-blob key, wrapped by the DEK

Each blob is encrypted with a fresh random 256-bit content key using
**AES-256-GCM** (random 96-bit nonce; matches the AES-256 line SQLCipher uses).
The content key is wrapped by the DEK (per ADR 0002 §5) and the wrap — never the
raw key — is stored in the blob's metadata row. Dedup-identical blobs still get a
**single** stored ciphertext; the shared content key is wrapped once. Whole-blob
GCM is the MVP; chunked AEAD for very large files is a forward option (§Revisit).

### 4. Metadata lives in SQLCipher, not beside the blob

All attachment metadata is rows in `vault.db` (already SQLCipher-encrypted at
rest, which **is** the "encrypted metadata" ADR 0002 calls for — no second
encryption layer for filenames). An `attachments` table carries: `id` (UUIDv7),
`storage_id` (hex, = on-disk name), `wrapped_content_key`, `content_nonce`,
`content_alg`, `plaintext_size`, `mime_type`, `original_filename`, `created_at`,
`ref_count`. No metadata is written to the `blobs/` directory — the files are
pure opaque ciphertext.

### 5. References are many-to-many

Domain entities link to attachments through an `attachment_links` table
(`attachment_id`, `entity_kind`, `entity_id`, `created_at`) rather than a single
FK column, so one receipt can back several transactions and an entity can carry
several documents. `ref_count` on `attachments` is the number of live links.

### 6. No plaintext attachment bytes ever leave the vault

Import reads the source file, encrypts **in memory**, and writes only ciphertext
to `blobs/`. Plaintext attachment bytes are **never** written to the OS temp dir,
a cache, or any path outside `<vault>/`. Decryption is to memory only, for a
caller that has an explicit need; rendering/OCR that would need plaintext on disk
is out of scope until ADR 0022 (parser isolation) provides a sandbox. Enforced by
`bcj`'s test: plant a known PDF + PNG, then after import + a simulated crash
assert none of their plaintext bytes appear in `vault.db`, the OS temp dir, or the
macOS PreviewCache.

### 7. Deletion is crypto-shredding

Removing the last link to an attachment deletes its `attachments` row — which
holds the only wrapped copy of the content key — rendering the blob ciphertext
**permanently undecryptable** regardless of whether the file is reclaimed.
(Secure overwrite is unreliable on SSDs; crypto-shred is the durable guarantee.)
The now-orphaned `blobs/` file is then unlinked best-effort.

### 8. Ownership

A blob-store module owns `blobs/` read/write + per-blob AEAD, using `vault-crypto`
for HKDF/wrap/unwrap; `db-worker` (sole `rusqlite` owner) owns the `attachments` /
`attachment_links` rows. The store is exposed to the kernel as commands, never via
direct FS access from the frontend.

## Consequences

### Positive

- Large documents never bloat `vault.db`; WAL/VACUUM stay cheap; blob I/O streams.
- Dedup falls out of keyed content addressing; the on-disk name leaks nothing.
- Per-blob keys bound blast radius (ADR 0002 §Positive) and make deletion a
  cheap, durable crypto-shred.
- Generalizes cleanly to OCR text, thumbnails, and `document_extractions`
  (`personal-cfo-iwo` / `-p6uk`) — all just more encrypted rows/derived blobs.

### Negative

- Two on-disk artifacts (`vault.db` + `blobs/`) must stay consistent; backup
  (ADR 0024) and restore must move both atomically, and an orphan-GC sweep is
  needed for crash-interrupted writes.
- Keyed content addressing requires the DEK (vault unlocked) to compute a storage
  ID, so dedup/import only happens while unlocked — acceptable (import is an
  unlocked-only action anyway).

## Rejected alternatives

- **Attachment bytes as SQLCipher BLOBs.** ✗ Bloats the DB, makes WAL/VACUUM and
  the read-model pay for multi-MB binaries, couples blob size to transaction I/O.
- **Bare content-hash filenames (SHA-256 of plaintext).** ✗ Lets an attacker with
  a guessed document confirm its presence by matching the hash; keyed addressing
  removes that while keeping dedup.
- **Opaque random storage IDs (UUID), no dedup.** ✗ Simpler but stores duplicate
  copies of the same attachment; keyed content addressing is strictly better.
- **One shared attachment key (not per-blob).** ✗ Loses the per-blob blast-radius
  bound and the crypto-shred-on-delete property.
- **Plaintext temp file for preview/OCR.** ✗ Violates the no-plaintext-artifact
  gate criterion; processing waits for ADR 0022's sandbox.

## Revisit if

- A single attachment can exceed the AES-GCM safe single-shot limit → adopt
  chunked AEAD with per-chunk derived nonces.
- ADR 0022 (parser isolation) lands and needs a defined hand-off path for
  staging ciphertext into the sandbox.
- Cross-device sync (ADR 0017) requires blob-level sync envelopes.

## Test coverage

- `personal-cfo-bcj` plaintext-leak test (PDF + PNG, post-import + crash) — the
  gate's no-plaintext-artifact check.
- Cross-cutting redaction CI suite (`personal-cfo-zobt`) over attachment-path
  tracing spans.
- Dedup unit test: importing identical bytes twice yields one `blobs/` file and
  `ref_count == 2`; deleting one link leaves the blob, deleting both crypto-shreds
  it.

## Linked beads

- `personal-cfo-b7jk` (this ADR)
- `personal-cfo-bcj` (implementation: encrypted attachment store)
- `personal-cfo-ef3` / `-au3` (backup/restore bundle the blob store — ADR 0024)
- `personal-cfo-iwo` / `-p6uk` (documents + extractions schema — extensibility)
- `personal-cfo-v4l` (ADR 0022 parser/document isolation — attachment *processing*)
- `personal-cfo-2y8` (rekey re-wraps every per-blob content key)
