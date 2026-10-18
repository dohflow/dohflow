//! Reference vectors for the Sync wire formats and the portable bundle
//! container (Part C of `docs/architecture/vault-format.md`, personal-cfo-klr.4).
//!
//! This file is a **test-only reference**: it implements the canonical binary
//! encoding and every derivation in the specification, using only primitives
//! `vault-crypto` already depends on. It is not a production exporter or Sync
//! client. The hex vectors in the specification are checked against it, and its
//! negative and scenario tests pin the fail-closed rules the spec states.
//!
//! Ed25519 and HPKE operations are deliberately absent: those crates are not
//! workspace dependencies yet. The specification pins every byte that feeds
//! them (signed bodies, HPKE `info`, AAD), and the implementing beads add the
//! RFC 8032 and RFC 9180 known-answer tests.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");

// ---------------------------------------------------------------------------
// Canonical binary encoding (CBE1)
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum WireError {
    Truncated,
    TrailingBytes,
    TooLong,
    BadUtf8,
    BadPresence,
    LabelMismatch,
    BadLabel,
    Semantic(&'static str),
}

/// Encoder. Every structure starts with a `label`; fixed-width integers are
/// big-endian; variable fields carry a `u32` length and a stated maximum.
#[derive(Default)]
struct Enc(Vec<u8>);

impl Enc {
    fn new(label: &str) -> Self {
        let mut e = Self::default();
        assert!(
            (1..=64).contains(&label.len())
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"/_.-".contains(&b)),
            "bad label {label}"
        );
        e.0.extend_from_slice(&(label.len() as u16).to_be_bytes());
        e.0.extend_from_slice(label.as_bytes());
        e
    }
    fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn u16(mut self, v: u16) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u32(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u64(mut self, v: u64) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn i64(mut self, v: i64) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn fixed(mut self, v: &[u8]) -> Self {
        self.0.extend_from_slice(v);
        self
    }
    fn bytes(mut self, v: &[u8], max: u32) -> Self {
        assert!(v.len() as u64 <= u64::from(max), "field over its maximum");
        self.0.extend_from_slice(&(v.len() as u32).to_be_bytes());
        self.0.extend_from_slice(v);
        self
    }
    fn text(self, v: &str, max: u32) -> Self {
        self.bytes(v.as_bytes(), max)
    }
    fn blob(mut self, v: &[u8]) -> Self {
        self.0.extend_from_slice(&(v.len() as u64).to_be_bytes());
        self.0.extend_from_slice(v);
        self
    }
    fn opt_fixed(self, v: Option<&[u8]>) -> Self {
        match v {
            None => self.u8(0),
            Some(b) => self.u8(1).fixed(b),
        }
    }
    fn list_digests(self, items: &[[u8; 32]], max: u32) -> Self {
        assert!(items.len() as u64 <= u64::from(max));
        let mut e = self.u32(items.len() as u32);
        for i in items {
            e = e.fixed(i);
        }
        e
    }
    fn done(self) -> Vec<u8> {
        self.0
    }
}

struct Dec<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Dec<'a> {
    fn new(buf: &'a [u8], expect_label: &str) -> Result<Self, WireError> {
        let mut d = Self { buf, pos: 0 };
        let len = usize::from(u16::from_be_bytes(d.take(2)?.try_into().unwrap()));
        if !(1..=64).contains(&len) {
            return Err(WireError::BadLabel);
        }
        if d.take(len)? != expect_label.as_bytes() {
            return Err(WireError::LabelMismatch);
        }
        Ok(d)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.pos.checked_add(n).ok_or(WireError::Truncated)?;
        let out = self.buf.get(self.pos..end).ok_or(WireError::Truncated)?;
        self.pos = end;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    fn bytes(&mut self, max: u32) -> Result<&'a [u8], WireError> {
        let len = self.u32()?;
        if len > max {
            return Err(WireError::TooLong);
        }
        self.take(len as usize)
    }
    fn text(&mut self, max: u32) -> Result<&'a str, WireError> {
        std::str::from_utf8(self.bytes(max)?).map_err(|_| WireError::BadUtf8)
    }
    fn opt_fixed<const N: usize>(&mut self) -> Result<Option<[u8; N]>, WireError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.fixed()?)),
            _ => Err(WireError::BadPresence),
        }
    }
    fn finish(self) -> Result<(), WireError> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            Err(WireError::TrailingBytes)
        }
    }
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

fn hkdf_expand(prk: &[u8; 32], info: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::from_prk(prk).expect("32-byte PRK");
    let mut out = [0u8; 32];
    hk.expand(info, &mut out).expect("32 bytes");
    out
}

fn hmac256(key: &[u8; 32], msg: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap();
    mac.update(msg);
    mac.finalize().into_bytes().into()
}

fn seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key))
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .unwrap()
}

fn open(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ct: &[u8]) -> bool {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key))
        .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad })
        .is_ok()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Protocol constants and derivations (the specification, in code)
// ---------------------------------------------------------------------------

const PROTOCOL_VERSION: u16 = 1;
const SUITE_ID: u16 = 1;
/// RFC 9180 `kem_id` of DHKEM(X25519, HKDF-SHA256), the KEM of suite 1.
const KEM_ID_X25519: u16 = 0x0020;
const MAX_KEM_PUBLIC_KEY: u32 = 2048;
const MAX_OBJECTS: u32 = 1 << 20;
const MAX_FILES: u32 = 1024;
const MAX_ROWS: u32 = 1 << 20;
const OBJ_ENVELOPE: u8 = 1;
const OBJ_SNAPSHOT: u8 = 2;
const MAX_COUNTER: u64 = (1 << 32) - 1;

fn device_fingerprint(suite: u16, kem_id: u16, ed_pk: &[u8; 32], kem_pk: &[u8]) -> [u8; 32] {
    sha256(
        &Enc::new("dohflow/device-fpr/v1")
            .u16(suite)
            .u16(kem_id)
            .fixed(ed_pk)
            .bytes(kem_pk, MAX_KEM_PUBLIC_KEY)
            .done(),
    )
}

fn aead_key(k_e: &[u8; 32], vault: &[u8; 16], epoch: u64, fpr: &[u8; 32], obj: u8) -> [u8; 32] {
    hkdf_expand(
        k_e,
        &Enc::new("dohflow/sync-aead/v1")
            .u16(PROTOCOL_VERSION)
            .fixed(vault)
            .u64(epoch)
            .fixed(fpr)
            .u8(obj)
            .done(),
    )
}

fn nonce_domain(vault: &[u8; 16], epoch: u64, fpr: &[u8; 32], obj: u8) -> [u8; 4] {
    let h = sha256(
        &Enc::new("dohflow/sync-nonce/v1")
            .fixed(vault)
            .u64(epoch)
            .fixed(fpr)
            .u8(obj)
            .done(),
    );
    h[..4].try_into().unwrap()
}

fn nonce(domain: [u8; 4], counter: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..4].copy_from_slice(&domain);
    n[4..].copy_from_slice(&counter.to_be_bytes());
    n
}

fn tag_key(s: &[u8; 32], vault: &[u8; 16]) -> [u8; 32] {
    hkdf_expand(
        s,
        &Enc::new("dohflow/sync-idem-tag-key/v1").fixed(vault).done(),
    )
}

/// Only a CSPRNG UUIDv4 may enter a Sync envelope as an idempotency key.
fn validate_idempotency_key(key: &[u8; 16]) -> Result<(), WireError> {
    if key[6] >> 4 != 4 {
        return Err(WireError::Semantic("idempotency key is not UUID version 4"));
    }
    if key[8] >> 6 != 0b10 {
        return Err(WireError::Semantic("idempotency key has a non-RFC variant"));
    }
    Ok(())
}

fn idempotency_tag(s: &[u8; 32], vault: &[u8; 16], key: &[u8; 16]) -> [u8; 32] {
    hmac256(
        &tag_key(s, vault),
        &Enc::new("dohflow/sync-idem-tag/v1").fixed(key).done(),
    )
}

struct Meta {
    command_id: [u8; 16],
    correlation_id: [u8; 16],
    causation_id: Option<[u8; 16]>,
    actor_type: u8,
    actor_id: &'static str,
    idempotency_key: [u8; 16],
    issued_at: i64,
    device_id: [u8; 16],
}

fn meta_commitment(m: &Meta) -> [u8; 32] {
    sha256(
        &Enc::new("dohflow/sync-meta-commit/v1")
            .fixed(&m.command_id)
            .fixed(&m.correlation_id)
            .opt_fixed(m.causation_id.as_ref().map(<[u8; 16]>::as_slice))
            .u8(m.actor_type)
            .text(m.actor_id, 256)
            .fixed(&m.idempotency_key)
            .i64(m.issued_at)
            .fixed(&m.device_id)
            .done(),
    )
}

fn key_commitment_ke(k_e: &[u8; 32]) -> [u8; 32] {
    sha256(
        &Enc::new("dohflow/sync-epoch-key-commit/v1")
            .fixed(k_e)
            .done(),
    )
}

fn key_commitment_s(s: &[u8; 32]) -> [u8; 32] {
    sha256(&Enc::new("dohflow/sync-tag-key-commit/v1").fixed(s).done())
}

fn artifact_id(k_e: &[u8; 32], vault: &[u8; 16], storage_id: &[u8; 32]) -> [u8; 32] {
    let addr_key = hkdf_expand(
        k_e,
        &Enc::new("dohflow/sync-artifact-address/v1")
            .fixed(vault)
            .done(),
    );
    hmac256(&addr_key, storage_id)
}

fn artifact_object_key(k_e: &[u8; 32], vault: &[u8; 16], epoch: u64, id: &[u8; 32]) -> [u8; 32] {
    hkdf_expand(
        k_e,
        &Enc::new("dohflow/sync-artifact-object/v1")
            .u16(PROTOCOL_VERSION)
            .fixed(vault)
            .u64(epoch)
            .fixed(id)
            .done(),
    )
}

fn artifact_aad(vault: &[u8; 16], epoch: u64, id: &[u8; 32]) -> Vec<u8> {
    Enc::new("dohflow/sync-artifact-aad/v1")
        .u16(PROTOCOL_VERSION)
        .fixed(vault)
        .u64(epoch)
        .fixed(id)
        .done()
}

/// A manifest or delta list is strictly ascending: sorted and without duplicates.
fn validate_manifest_list(ids: &[[u8; 32]]) -> Result<(), WireError> {
    if ids.windows(2).all(|w| w[0] < w[1]) {
        Ok(())
    } else {
        Err(WireError::Semantic(
            "manifest list is not strictly ascending",
        ))
    }
}

/// The header commitment rule: an envelope carries the commitment of its delta
/// manifest only when that manifest is non-empty; a snapshot always carries the
/// commitment of its (possibly empty) full closure.
fn manifest_presence_ok(is_snapshot: bool, ids: &[[u8; 32]], commit: Option<[u8; 32]>) -> bool {
    match (is_snapshot, ids.is_empty(), commit) {
        (false, true, None) => true,
        (false, false, Some(c)) | (true, _, Some(c)) => c == manifest_commitment(ids),
        _ => false,
    }
}

/// `row_digest` is owned by `personal-cfo-t2s4b`; the container is fixed here.
/// Rows sort by the raw bytes of the table name, then of the primary key (not by
/// their length-prefixed encodings).
fn write_set_hash(rows: &[(&str, &[u8], [u8; 32])]) -> [u8; 32] {
    assert!(rows.len() as u64 <= u64::from(MAX_ROWS));
    let mut sorted = rows.to_vec();
    sorted.sort_by(|a, b| (a.0.as_bytes(), a.1).cmp(&(b.0.as_bytes(), b.1)));
    let mut e = Enc::new("dohflow/sync-writeset/v1").u32(sorted.len() as u32);
    for (table, pk, digest) in sorted {
        e = e.text(table, 64).bytes(pk, 256).fixed(&digest);
    }
    sha256(&e.done())
}

fn manifest_commitment(ids: &[[u8; 32]]) -> [u8; 32] {
    let mut sorted = ids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "duplicate artifact id");
    sha256(
        &Enc::new("dohflow/sync-artifact-manifest/v1")
            .list_digests(&sorted, 4096)
            .done(),
    )
}

struct Header {
    object_type: u8,
    key_epoch: u64,
    object_id: [u8; 16],
    tag: Option<[u8; 32]>,
    meta_commit: Option<[u8; 32]>,
    device_id: [u8; 16],
    fpr: [u8; 32],
    nonce_domain: [u8; 4],
    counter: u64,
    base_seq: u64,
    payload_schema_version: u32,
    engine_version: &'static str,
    schema_version: u32,
    manifest_commit: Option<[u8; 32]>,
}

const VAULT: [u8; 16] = [0x11; 16];

fn encode_header(h: &Header) -> Vec<u8> {
    Enc::new("dohflow/sync-header/v1")
        .u16(PROTOCOL_VERSION)
        .fixed(&VAULT)
        .u64(h.key_epoch)
        .u8(h.object_type)
        .fixed(&h.object_id)
        .opt_fixed(h.tag.as_ref().map(<[u8; 32]>::as_slice))
        .opt_fixed(h.meta_commit.as_ref().map(<[u8; 32]>::as_slice))
        .fixed(&h.device_id)
        .fixed(&h.fpr)
        .fixed(&h.nonce_domain)
        .u64(h.counter)
        .u64(h.base_seq)
        .u32(h.payload_schema_version)
        .text(h.engine_version, 64)
        .u32(h.schema_version)
        .opt_fixed(h.manifest_commit.as_ref().map(<[u8; 32]>::as_slice))
        .done()
}

/// Parse and validate a header the way a receiver must: strict grammar, then
/// the object-type rules for the two dual-identity fields.
fn parse_header(buf: &[u8]) -> Result<(u8, u64, Option<[u8; 32]>), WireError> {
    let mut d = Dec::new(buf, "dohflow/sync-header/v1")?;
    if d.u16()? != PROTOCOL_VERSION {
        return Err(WireError::Semantic("unsupported protocol version"));
    }
    let _vault: [u8; 16] = d.fixed()?;
    let _epoch = d.u64()?;
    let object_type = d.u8()?;
    let _object_id: [u8; 16] = d.fixed()?;
    let tag: Option<[u8; 32]> = d.opt_fixed()?;
    let commit: Option<[u8; 32]> = d.opt_fixed()?;
    let _device: [u8; 16] = d.fixed()?;
    let _fpr: [u8; 32] = d.fixed()?;
    let _domain: [u8; 4] = d.fixed()?;
    let counter = d.u64()?;
    let _base = d.u64()?;
    let _psv = d.u32()?;
    let _engine = d.text(64)?;
    let _schema = d.u32()?;
    let manifest: Option<[u8; 32]> = d.opt_fixed()?;
    d.finish()?;
    match (object_type, tag.is_some(), commit.is_some()) {
        (OBJ_ENVELOPE, true, true) | (OBJ_SNAPSHOT, false, false) => {}
        (OBJ_ENVELOPE | OBJ_SNAPSHOT, _, _) => {
            return Err(WireError::Semantic(
                "dual-identity fields do not match type",
            ))
        }
        _ => return Err(WireError::Semantic("unknown object type")),
    }
    if counter > MAX_COUNTER {
        return Err(WireError::Semantic("counter beyond the per-key cap"));
    }
    Ok((object_type, counter, manifest))
}

fn envelope_digest(header: &[u8], ct: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(Enc::new("dohflow/sync-envelope-digest/v1").done());
    h.update((header.len() as u32).to_be_bytes());
    h.update(header);
    h.update((ct.len() as u64).to_be_bytes());
    h.update(ct);
    h.finalize().into()
}

fn container(header: &[u8], ct: &[u8]) -> Vec<u8> {
    let mut out = b"DFSY".to_vec();
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&(header.len() as u32).to_be_bytes());
    out.extend_from_slice(header);
    out.extend_from_slice(&(ct.len() as u64).to_be_bytes());
    out.extend_from_slice(ct);
    out
}

// Membership / epoch record bodies (the bytes Ed25519 signs; HPKE info).
#[allow(clippy::too_many_arguments)]
fn membership_body(
    version: u64,
    prev: Option<[u8; 32]>,
    action: u8,
    signer_fpr: &[u8; 32],
    device_id: &[u8; 16],
    ed_pk: &[u8; 32],
    kem_pk: &[u8],
    issued_at: i64,
) -> Vec<u8> {
    Enc::new("dohflow/sync-membership/v1")
        .u16(PROTOCOL_VERSION)
        .u16(SUITE_ID)
        .u16(KEM_ID_X25519)
        .fixed(&VAULT)
        .u64(version)
        .opt_fixed(prev.as_ref().map(<[u8; 32]>::as_slice))
        .u8(action)
        .fixed(signer_fpr)
        .fixed(device_id)
        .fixed(ed_pk)
        .bytes(kem_pk, MAX_KEM_PUBLIC_KEY)
        .i64(issued_at)
        .done()
}

fn epoch_header(
    epoch: u64,
    membership_head: &[u8; 32],
    prev_epoch: Option<[u8; 32]>,
    commit_ke: &[u8; 32],
    commit_s: &[u8; 32],
    sealer_fpr: &[u8; 32],
) -> Vec<u8> {
    Enc::new("dohflow/sync-epoch-record/v1")
        .u16(PROTOCOL_VERSION)
        .u16(SUITE_ID)
        .u16(KEM_ID_X25519)
        .fixed(&VAULT)
        .u64(epoch)
        .fixed(membership_head)
        .opt_fixed(prev_epoch.as_ref().map(<[u8; 32]>::as_slice))
        .fixed(commit_ke)
        .fixed(commit_s)
        .fixed(sealer_fpr)
        .done()
}

fn hpke_aad(epoch_header_hash: &[u8; 32], recipient_fpr: &[u8; 32]) -> Vec<u8> {
    Enc::new("dohflow/sync-hpke-aad/v1")
        .fixed(epoch_header_hash)
        .fixed(recipient_fpr)
        .done()
}

#[allow(clippy::too_many_arguments)]
fn hpke_info(
    epoch: u64,
    recipient_id: &[u8; 16],
    recipient_fpr: &[u8; 32],
    sealer_fpr: &[u8; 32],
    membership_head: &[u8; 32],
    epoch_header_hash: &[u8; 32],
) -> Vec<u8> {
    Enc::new("dohflow/sync-hpke-info/v1")
        .u16(PROTOCOL_VERSION)
        .u16(SUITE_ID)
        .fixed(&VAULT)
        .u64(epoch)
        .fixed(recipient_id)
        .fixed(recipient_fpr)
        .fixed(sealer_fpr)
        .fixed(membership_head)
        .fixed(epoch_header_hash)
        .done()
}

// Portable bundle container (Export C with the F7 conditions).
const EXPORT_PROFILE_INTERACTIVE: u8 = 1;
const EXPORT_PROFILE_HIGH: u8 = 2;

fn export_profile(id: u8) -> Result<(u32, u32, u32), WireError> {
    match id {
        EXPORT_PROFILE_INTERACTIVE => Ok((65_536, 3, 1)),
        EXPORT_PROFILE_HIGH => Ok((262_144, 4, 1)),
        _ => Err(WireError::Semantic("kdf profile is not on the allowlist")),
    }
}

fn export_header(profile: u8, salt: &[u8; 32], object_count: u32) -> Vec<u8> {
    let (m, t, p) = export_profile(profile).unwrap();
    Enc::new("dohflow/export-header/v1")
        .u16(1)
        .u8(profile)
        .u32(m)
        .u32(t)
        .u32(p)
        .fixed(salt)
        .u32(object_count)
        .u32(1_048_576)
        .done()
}

/// A reader validates the stored parameters against the allowlist **before**
/// it spends memory on the KDF.
fn parse_export_header(buf: &[u8]) -> Result<(u8, [u8; 32], u32), WireError> {
    let mut d = Dec::new(buf, "dohflow/export-header/v1")?;
    if d.u16()? != 1 {
        return Err(WireError::Semantic("unsupported container version"));
    }
    let profile = d.u8()?;
    let (m, t, p) = (d.u32()?, d.u32()?, d.u32()?);
    if export_profile(profile)? != (m, t, p) {
        return Err(WireError::Semantic(
            "kdf parameters differ from the profile",
        ));
    }
    let salt: [u8; 32] = d.fixed()?;
    let count = d.u32()?;
    if d.u32()? != 1_048_576 {
        return Err(WireError::Semantic("unsupported chunk size"));
    }
    d.finish()?;
    if count == 0 || count > MAX_OBJECTS {
        return Err(WireError::Semantic("object count is outside 1..=2^20"));
    }
    Ok((profile, salt, count))
}

fn export_object_key(kek: &[u8; 32], header_hash: &[u8; 32], index: u32) -> [u8; 32] {
    let label = if index == 0 {
        "dohflow/export-manifest-key/v1"
    } else {
        "dohflow/export-object-key/v1"
    };
    hkdf_expand(kek, &Enc::new(label).fixed(header_hash).u32(index).done())
}

fn export_object_aad(
    header_hash: &[u8; 32],
    path: &str,
    index: u32,
    chunk: u32,
    last: bool,
    plaintext_len: u64,
) -> Vec<u8> {
    Enc::new("dohflow/export-object-aad/v1")
        .fixed(header_hash)
        .text(path, 255)
        .u32(index)
        .u32(chunk)
        .u8(u8::from(last))
        .u64(plaintext_len)
        .done()
}

fn argon2id_kek(password: &[u8], salt: &[u8; 32], profile: u8) -> [u8; 32] {
    use argon2::{Algorithm, Argon2, Params, Version};
    let (m, t, p) = export_profile(profile).unwrap();
    let mut out = [0u8; 32];
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(m, t, p, Some(32)).unwrap(),
    )
    .hash_password_into(password, salt, &mut out)
    .unwrap();
    out
}

// ---------------------------------------------------------------------------
// The vector set
// ---------------------------------------------------------------------------

fn counting(start: u8) -> [u8; 32] {
    core::array::from_fn(|i| start.wrapping_add(i as u8))
}

fn vectors() -> Vec<(&'static str, Vec<u8>)> {
    let k_e = counting(0x00);
    let k_e2 = counting(0x40);
    let s = counting(0x20);
    let (dev_a, dev_b) = ([0xa1u8; 16], [0xb1u8; 16]);
    let (ed_a, x_a, ed_b, x_b) = ([0xe1u8; 32], [0xc1u8; 32], [0xe2u8; 32], [0xc2u8; 32]);
    let fpr_a = device_fingerprint(SUITE_ID, KEM_ID_X25519, &ed_a, &x_a);
    let fpr_b = device_fingerprint(SUITE_ID, KEM_ID_X25519, &ed_b, &x_b);
    let idem: [u8; 16] = [
        0x4a, 0x3b, 0x2c, 0x1d, 0x9e, 0x8f, 0x4a, 0x7b, 0x8c, 0x6d, 0x5e, 0x4f, 0x3a, 0x2b, 0x1c,
        0x0d,
    ];
    validate_idempotency_key(&idem).unwrap();
    let meta = Meta {
        command_id: [
            0x01, 0x93, 0x8f, 0x2e, 0x7c, 0x1a, 0x7b, 0x3d, 0x9a, 0x4e, 0x12, 0x34, 0x56, 0x78,
            0x9a, 0xbc,
        ],
        correlation_id: [0x22; 16],
        causation_id: None,
        actor_type: 1,
        actor_id: "device-a-user",
        idempotency_key: idem,
        issued_at: 1_790_000_000_000_000,
        device_id: dev_a,
    };
    let commit = meta_commitment(&meta);
    let tag = idempotency_tag(&s, &VAULT, &idem);

    let storage_1 = [0x5au8; 32];
    let storage_2 = [0x6bu8; 32];
    let art_1 = artifact_id(&k_e, &VAULT, &storage_1);
    let art_1_e2 = artifact_id(&k_e2, &VAULT, &storage_1);
    let art_2 = artifact_id(&k_e, &VAULT, &storage_2);
    let man_commit = manifest_commitment(&[art_1, art_2]);
    let mut delta = vec![art_1, art_2];
    delta.sort_unstable();
    validate_manifest_list(&delta).unwrap();

    let nd_a = nonce_domain(&VAULT, 1, &fpr_a, OBJ_ENVELOPE);
    let nd_b = nonce_domain(&VAULT, 1, &fpr_b, OBJ_ENVELOPE);
    let key_a = aead_key(&k_e, &VAULT, 1, &fpr_a, OBJ_ENVELOPE);

    let header_env = encode_header(&Header {
        object_type: OBJ_ENVELOPE,
        key_epoch: 1,
        object_id: meta.command_id,
        tag: Some(tag),
        meta_commit: Some(commit),
        device_id: dev_a,
        fpr: fpr_a,
        nonce_domain: nd_a,
        counter: 0,
        base_seq: 41,
        payload_schema_version: 1,
        engine_version: "dohflow-0.3.0",
        schema_version: 55,
        manifest_commit: Some(man_commit),
    });
    parse_header(&header_env).unwrap();
    let nd_snap = nonce_domain(&VAULT, 1, &fpr_a, OBJ_SNAPSHOT);
    let header_snap = encode_header(&Header {
        object_type: OBJ_SNAPSHOT,
        key_epoch: 1,
        object_id: [0x33; 16],
        tag: None,
        meta_commit: None,
        device_id: dev_a,
        fpr: fpr_a,
        nonce_domain: nd_snap,
        counter: 0,
        base_seq: 41,
        payload_schema_version: 1,
        engine_version: "dohflow-0.3.0",
        schema_version: 55,
        manifest_commit: Some(man_commit),
    });
    parse_header(&header_snap).unwrap();

    let plaintext_env = Enc::new("dohflow/sync-envelope-plaintext/v1")
        .fixed(&meta.command_id)
        .fixed(&meta.correlation_id)
        .opt_fixed(None)
        .u8(meta.actor_type)
        .text(meta.actor_id, 256)
        .fixed(&meta.idempotency_key)
        .i64(meta.issued_at)
        .fixed(&meta.device_id)
        .fixed(&[0x44; 32])
        .list_digests(&delta, 4096)
        .bytes(br#"{"amount_minor":"-1250","currency":"USD"}"#, 1 << 24)
        .done();
    let ct_env = seal(&key_a, &nonce(nd_a, 0), &header_env, &plaintext_env);
    assert!(open(&key_a, &nonce(nd_a, 0), &header_env, &ct_env));

    // Membership chain and epoch record.
    let m1 = membership_body(
        1,
        None,
        1,
        &fpr_a,
        &dev_a,
        &ed_a,
        &x_a,
        1_790_000_001_000_000,
    );
    let id_m1 = sha256(&m1);
    let m2 = membership_body(
        2,
        Some(id_m1),
        1,
        &fpr_a,
        &dev_b,
        &ed_b,
        &x_b,
        1_790_000_002_000_000,
    );
    let id_m2 = sha256(&m2);
    let commit_ke = key_commitment_ke(&k_e);
    let commit_s = key_commitment_s(&s);
    let eh = epoch_header(1, &id_m2, None, &commit_ke, &commit_s, &fpr_a);
    let eh_hash = sha256(&eh);
    let info_b = hpke_info(1, &dev_b, &fpr_b, &fpr_a, &id_m2, &eh_hash);
    let sealed_plaintext = Enc::new("dohflow/sync-sealed-keys/v1")
        .fixed(&k_e)
        .fixed(&s)
        .done();

    // Artifact object.
    let obj_key = artifact_object_key(&k_e, &VAULT, 1, &art_1);
    let art_ad = artifact_aad(&VAULT, 1, &art_1);
    let transport = Enc::new("dohflow/sync-artifact-transport/v1")
        .fixed(&art_1)
        .u8(1)
        .u64(18)
        .fixed(&sha256(b"synthetic document"))
        .bytes(br#"{"mime_type":"text/plain"}"#, 65_536)
        .blob(b"synthetic document")
        .done();
    let art_ct = seal(&obj_key, &[0u8; 12], &art_ad, &transport);

    // Portable bundle.
    let salt = [0x77u8; 32];
    let hdr = export_header(EXPORT_PROFILE_INTERACTIVE, &salt, 2);
    parse_export_header(&hdr).unwrap();
    let hdr_hash = sha256(&hdr);
    let kek = argon2id_kek(
        b"synthetic-export-password",
        &salt,
        EXPORT_PROFILE_INTERACTIVE,
    );
    let k0 = export_object_key(&kek, &hdr_hash, 0);
    let k1 = export_object_key(&kek, &hdr_hash, 1);
    let row = b"{\"id\":\"1\",\"name\":\"Checking\"}\n";
    let aad1 = export_object_aad(
        &hdr_hash,
        "data/accounts.jsonl",
        1,
        0,
        true,
        row.len() as u64,
    );
    let ct1 = seal(&k1, &[0u8; 12], &aad1, row);
    let manifest_json = format!(
        "{{\"app_version\":\"dohflow-0.3.0\",\"artifacts\":[],\"format\":\"dohflow-portable-export\",\
         \"format_version\":\"1\",\"inventory_version\":\"1\",\"object_count\":\"2\",\
         \"objects\":[{{\"chunk\":\"0\",\"ciphertext_length\":\"{}\",\"ciphertext_sha256\":\"{}\",\
         \"index\":\"1\",\"path\":\"data/accounts.jsonl\"}}],\"schema_version\":\"55\",\
         \"tables\":[{{\"class\":\"1\",\"columns\":[{{\"kind\":\"text\",\"name\":\"id\"}},\
         {{\"kind\":\"text\",\"name\":\"name\"}}],\"name\":\"accounts\",\"row_count\":\"1\",\
         \"sha256\":\"{}\"}}]}}",
        ct1.len(),
        hex(&sha256(&ct1)),
        hex(&sha256(row))
    );
    let aad0 = export_object_aad(
        &hdr_hash,
        "manifest.json",
        0,
        0,
        true,
        manifest_json.len() as u64,
    );
    let ct0 = seal(&k0, &[0u8; 12], &aad0, manifest_json.as_bytes());

    vec![
        ("fingerprint_a", fpr_a.to_vec()),
        ("fingerprint_b", fpr_b.to_vec()),
        ("aead_key_a_envelope", key_a.to_vec()),
        (
            "aead_key_b_envelope",
            aead_key(&k_e, &VAULT, 1, &fpr_b, OBJ_ENVELOPE).to_vec(),
        ),
        (
            "aead_key_a_snapshot",
            aead_key(&k_e, &VAULT, 1, &fpr_a, OBJ_SNAPSHOT).to_vec(),
        ),
        ("nonce_domain_a_envelope", nd_a.to_vec()),
        ("nonce_domain_b_envelope", nd_b.to_vec()),
        ("nonce_a_envelope_counter0", nonce(nd_a, 0).to_vec()),
        ("nonce_a_envelope_counter1", nonce(nd_a, 1).to_vec()),
        ("idempotency_tag_key", tag_key(&s, &VAULT).to_vec()),
        ("idempotency_tag", tag.to_vec()),
        ("immutable_metadata_commitment", commit.to_vec()),
        ("key_commitment_ke", commit_ke.to_vec()),
        ("key_commitment_s", commit_s.to_vec()),
        ("artifact_id_epoch1", art_1.to_vec()),
        ("artifact_id_epoch2", art_1_e2.to_vec()),
        ("artifact_manifest_commitment", man_commit.to_vec()),
        (
            "write_set_hash_sample",
            write_set_hash(&[
                ("ledger_transactions", &[0x01; 16], [0x51; 32]),
                ("accounts", &[0x02; 16], [0x52; 32]),
                ("accounts", &[0x01; 16], [0x53; 32]),
            ])
            .to_vec(),
        ),
        ("artifact_object_key", obj_key.to_vec()),
        ("artifact_aad", art_ad),
        ("artifact_object_ciphertext", art_ct),
        ("header_envelope", header_env.clone()),
        ("header_snapshot", header_snap),
        ("envelope_plaintext", plaintext_env),
        ("envelope_ciphertext", ct_env.clone()),
        (
            "envelope_digest",
            envelope_digest(&header_env, &ct_env).to_vec(),
        ),
        ("envelope_container", container(&header_env, &ct_env)),
        ("membership_body_1", m1),
        ("membership_id_1", id_m1.to_vec()),
        ("membership_body_2", m2),
        ("membership_id_2", id_m2.to_vec()),
        ("epoch_header", eh),
        ("epoch_header_hash", eh_hash.to_vec()),
        ("hpke_info_recipient_b", info_b),
        ("hpke_aad_recipient_b", hpke_aad(&eh_hash, &fpr_b)),
        ("sealed_keys_plaintext", sealed_plaintext),
        ("export_header", hdr),
        ("export_header_hash", hdr_hash.to_vec()),
        ("export_kek", kek.to_vec()),
        ("export_manifest_key", k0.to_vec()),
        ("export_object_key_1", k1.to_vec()),
        ("export_object_aad_1", aad1),
        ("export_object_ciphertext_1", ct1),
        ("export_manifest_json", manifest_json.into_bytes()),
        ("export_manifest_ciphertext", ct0),
    ]
}

fn spec_vectors() -> Vec<(String, String)> {
    SPEC.lines()
        .filter_map(|line| {
            let rest = line
                .strip_prefix("<!-- sync-vector: ")?
                .strip_suffix(" -->")?;
            let (name, value) = rest.split_once('=')?;
            Some((name.to_owned(), value.to_owned()))
        })
        .collect()
}

/// Run `cargo test -p vault-crypto --test sync_wire_vectors -- --ignored
/// --nocapture print_vectors` and paste the output into the specification.
#[test]
#[ignore = "generator: prints the vector markers for the specification"]
fn print_vectors() {
    for (name, bytes) in vectors() {
        println!("<!-- sync-vector: {name}={} -->", hex(&bytes));
    }
}

#[test]
fn spec_vectors_match_the_reference() {
    let documented = spec_vectors();
    let computed = vectors();
    assert_eq!(
        documented.len(),
        computed.len(),
        "the specification must list exactly the reference vectors"
    );
    for ((doc_name, doc_hex), (name, bytes)) in documented.iter().zip(&computed) {
        assert_eq!(doc_name, name, "vector order or name differs");
        assert_eq!(doc_hex, &hex(bytes), "vector {name} differs");
    }
}

// ---------------------------------------------------------------------------
// Fail-closed rules
// ---------------------------------------------------------------------------

fn sample_header() -> Vec<u8> {
    vectors()
        .into_iter()
        .find(|(n, _)| *n == "header_envelope")
        .unwrap()
        .1
}

#[test]
fn parser_rejects_every_malformed_header() {
    let good = sample_header();
    assert!(parse_header(&good).is_ok());

    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(parse_header(&trailing), Err(WireError::TrailingBytes));

    assert_eq!(
        parse_header(&good[..good.len() - 1]),
        Err(WireError::Truncated)
    );

    // Wrong domain label.
    let mut relabelled = good.clone();
    relabelled[2] ^= 0x01;
    assert_eq!(parse_header(&relabelled), Err(WireError::LabelMismatch));

    // A presence byte other than 0 or 1 (the tag's presence byte).
    let label_len = 2 + "dohflow/sync-header/v1".len();
    let tag_presence = label_len + 2 + 16 + 8 + 1 + 16;
    let mut bad_presence = good.clone();
    bad_presence[tag_presence] = 2;
    assert_eq!(parse_header(&bad_presence), Err(WireError::BadPresence));

    // Unsupported protocol version fails closed, not best-effort.
    let mut future = good.clone();
    future[label_len + 1] = 2;
    assert!(matches!(parse_header(&future), Err(WireError::Semantic(_))));
}

#[test]
fn text_fields_reject_oversize_and_invalid_utf8() {
    let good = sample_header();
    // engine_version length field set above its 64-byte maximum.
    let needle = b"dohflow-0.3.0";
    let at = good
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap();
    let mut long = good.clone();
    long[at - 4..at].copy_from_slice(&65u32.to_be_bytes());
    assert_eq!(parse_header(&long), Err(WireError::TooLong));
    let mut bad = good;
    bad[at] = 0xff;
    assert_eq!(parse_header(&bad), Err(WireError::BadUtf8));
}

#[test]
fn dual_identity_fields_must_match_the_object_type() {
    let base = Header {
        object_type: OBJ_SNAPSHOT,
        key_epoch: 1,
        object_id: [3; 16],
        tag: Some([9; 32]), // a snapshot must not carry an envelope tag
        meta_commit: None,
        device_id: [1; 16],
        fpr: [2; 32],
        nonce_domain: [0; 4],
        counter: 0,
        base_seq: 0,
        payload_schema_version: 1,
        engine_version: "e",
        schema_version: 55,
        manifest_commit: None,
    };
    assert!(matches!(
        parse_header(&encode_header(&base)),
        Err(WireError::Semantic(_))
    ));
    let envelope_without_tag = Header {
        object_type: OBJ_ENVELOPE,
        tag: None,
        ..base
    };
    assert!(matches!(
        parse_header(&encode_header(&envelope_without_tag)),
        Err(WireError::Semantic(_))
    ));
}

#[test]
fn counter_beyond_the_per_key_cap_is_rejected() {
    let mut h = Header {
        object_type: OBJ_SNAPSHOT,
        key_epoch: 1,
        object_id: [3; 16],
        tag: None,
        meta_commit: None,
        device_id: [1; 16],
        fpr: [2; 32],
        nonce_domain: [0; 4],
        counter: MAX_COUNTER,
        base_seq: 0,
        payload_schema_version: 1,
        engine_version: "e",
        schema_version: 55,
        manifest_commit: None,
    };
    assert!(parse_header(&encode_header(&h)).is_ok());
    h.counter = MAX_COUNTER + 1;
    assert!(matches!(
        parse_header(&encode_header(&h)),
        Err(WireError::Semantic(_))
    ));
}

#[test]
fn only_a_csprng_uuid_v4_may_be_an_idempotency_key() {
    let v4 = [
        0x4a, 0x3b, 0x2c, 0x1d, 0x9e, 0x8f, 0x4a, 0x7b, 0x8c, 0x6d, 0x5e, 0x4f, 0x3a, 0x2b, 0x1c,
        0x0d,
    ];
    assert!(validate_idempotency_key(&v4).is_ok());
    // A UUIDv7 (the backend's old minting scheme) is rejected with a typed error.
    let mut v7 = v4;
    v7[6] = 0x7b;
    assert!(validate_idempotency_key(&v7).is_err());
    // A wrong variant is rejected too.
    let mut bad_variant = v4;
    bad_variant[8] = 0x0c;
    assert!(validate_idempotency_key(&bad_variant).is_err());
}

#[test]
fn tag_and_commitment_tamper_fail_authentication() {
    // The receiver recomputes both from the decrypted plaintext (ADR 0074 §8).
    // Changing either clear field changes the AAD, so the ciphertext no longer
    // authenticates, and the receiver never reaches apply or index lookup.
    let header = sample_header();
    let all = vectors();
    let ct = &all
        .iter()
        .find(|(n, _)| *n == "envelope_ciphertext")
        .unwrap()
        .1;
    let key: [u8; 32] = all
        .iter()
        .find(|(n, _)| *n == "aead_key_a_envelope")
        .unwrap()
        .1
        .clone()
        .try_into()
        .unwrap();
    let nd: [u8; 4] = all
        .iter()
        .find(|(n, _)| *n == "nonce_domain_a_envelope")
        .unwrap()
        .1
        .clone()
        .try_into()
        .unwrap();
    assert!(open(&key, &nonce(nd, 0), &header, ct));
    let tag_at = 2 + "dohflow/sync-header/v1".len() + 2 + 16 + 8 + 1 + 16 + 1;
    let mut tampered_tag = header.clone();
    tampered_tag[tag_at] ^= 1;
    assert!(!open(&key, &nonce(nd, 0), &tampered_tag, ct));
    let commit_at = tag_at + 32 + 1;
    let mut tampered_commit = header;
    tampered_commit[commit_at] ^= 1;
    assert!(!open(&key, &nonce(nd, 0), &tampered_commit, ct));
}

// ---------------------------------------------------------------------------
// Scenario rules: equivocation, rollback, reuse, rotation
// ---------------------------------------------------------------------------

#[test]
fn equal_counters_on_distinct_device_keys_never_share_a_key() {
    let k_e = counting(0);
    let fpr_a = device_fingerprint(SUITE_ID, KEM_ID_X25519, &[0xe1; 32], &[0xc1; 32]);
    let fpr_b = device_fingerprint(SUITE_ID, KEM_ID_X25519, &[0xe2; 32], &[0xc2; 32]);
    assert_ne!(fpr_a, fpr_b);
    // Even if a hostile service hands both devices the same nonce domain and
    // counter, the derived AEAD keys differ, so (key, nonce) cannot repeat.
    let forced = nonce([0xde, 0xad, 0xbe, 0xef], 7);
    let key_a = aead_key(&k_e, &VAULT, 1, &fpr_a, OBJ_ENVELOPE);
    let key_b = aead_key(&k_e, &VAULT, 1, &fpr_b, OBJ_ENVELOPE);
    assert_ne!(key_a, key_b);
    let ct_a = seal(&key_a, &forced, b"aad", b"same plaintext");
    let ct_b = seal(&key_b, &forced, b"aad", b"same plaintext");
    assert_ne!(ct_a, ct_b);
    // The envelope and snapshot contexts of one device are separated too.
    assert_ne!(key_a, aead_key(&k_e, &VAULT, 1, &fpr_a, OBJ_SNAPSHOT));
    // A new epoch with a fresh root never reuses a key.
    assert_ne!(
        key_a,
        aead_key(&counting(0x40), &VAULT, 2, &fpr_a, OBJ_ENVELOPE)
    );
}

#[derive(Debug, PartialEq, Eq)]
enum NonceError {
    NonceStateLost,
    RotateEpoch,
}

/// The device-side counter register: persist before use, with an independent
/// anti-rollback witness. A restored or inconsistent register is lost state.
struct Register {
    next: u64,
    witness: u64,
}

impl Register {
    fn take(&mut self) -> Result<u64, NonceError> {
        if self.next < self.witness {
            return Err(NonceError::NonceStateLost);
        }
        if self.next > MAX_COUNTER {
            return Err(NonceError::RotateEpoch);
        }
        let used = self.next;
        self.next += 1; // persisted before encrypting
        self.witness = self.next;
        Ok(used)
    }
}

#[test]
fn counter_rollback_is_lost_state_and_the_cap_forces_rotation() {
    let mut r = Register {
        next: 0,
        witness: 0,
    };
    assert_eq!(r.take(), Ok(0));
    assert_eq!(r.take(), Ok(1));
    // A backup restore rolls the register back but not the independent witness.
    r.next = 0;
    assert_eq!(r.take(), Err(NonceError::NonceStateLost));

    let mut at_cap = Register {
        next: MAX_COUNTER,
        witness: MAX_COUNTER,
    };
    assert_eq!(at_cap.take(), Ok(MAX_COUNTER));
    assert_eq!(at_cap.take(), Err(NonceError::RotateEpoch));
}

/// The service's duplicate detection is a backstop, never the security source.
#[test]
fn service_backstop_accepts_only_exact_retries() {
    use std::collections::HashMap;
    type Context = (u64, [u8; 32], u8, [u8; 12]);
    let mut seen: HashMap<Context, Vec<u8>> = HashMap::new();
    let mut accept = |ctx: Context, bytes: &[u8]| -> bool {
        match seen.get(&ctx) {
            None => {
                seen.insert(ctx, bytes.to_vec());
                true
            }
            Some(first) => first == bytes,
        }
    };
    let ctx = (1, [7; 32], OBJ_ENVELOPE, nonce([1, 2, 3, 4], 0));
    assert!(accept(ctx, b"ciphertext"));
    assert!(
        accept(ctx, b"ciphertext"),
        "an exact retransmission is idempotent"
    );
    assert!(
        !accept(ctx, b"different"),
        "same-context reuse fails closed"
    );
}

#[test]
fn artifact_addresses_are_unlinkable_across_epochs_and_vaults() {
    let storage = [0x5a; 32];
    let a = artifact_id(&counting(0), &VAULT, &storage);
    // A fresh epoch root re-addresses the same plaintext.
    assert_ne!(a, artifact_id(&counting(0x40), &VAULT, &storage));
    // Another vault with the same root and plaintext is also unlinkable.
    assert_ne!(a, artifact_id(&counting(0), &[0x12; 16], &storage));
    // The address is not a plaintext digest and not the local storage id.
    assert_ne!(a, storage);
    assert_ne!(a, sha256(b"synthetic document"));
}

#[test]
fn manifest_commitment_is_order_independent_and_exact() {
    let (x, y) = ([1u8; 32], [2u8; 32]);
    assert_eq!(manifest_commitment(&[x, y]), manifest_commitment(&[y, x]));
    assert_ne!(manifest_commitment(&[x, y]), manifest_commitment(&[x]));
    assert_ne!(manifest_commitment(&[x]), manifest_commitment(&[]));
}

#[test]
fn artifact_object_rejects_altered_bytes_and_wrong_address() {
    let all = vectors();
    let get = |n: &str| all.iter().find(|(k, _)| *k == n).unwrap().1.clone();
    let key: [u8; 32] = get("artifact_object_key").try_into().unwrap();
    let aad = get("artifact_aad");
    let ct = get("artifact_object_ciphertext");
    assert!(open(&key, &[0u8; 12], &aad, &ct));
    let mut altered = ct.clone();
    altered[10] ^= 1;
    assert!(!open(&key, &[0u8; 12], &aad, &altered));
    // The same bytes presented under a different address do not authenticate.
    let other = artifact_aad(&VAULT, 1, &[9u8; 32]);
    assert!(!open(&key, &[0u8; 12], &other, &ct));
}

#[test]
fn epoch_record_context_binds_every_recipient_and_the_membership_head() {
    let fpr_a = device_fingerprint(SUITE_ID, KEM_ID_X25519, &[0xe1; 32], &[0xc1; 32]);
    let fpr_b = device_fingerprint(SUITE_ID, KEM_ID_X25519, &[0xe2; 32], &[0xc2; 32]);
    let head = [5u8; 32];
    let hh = [6u8; 32];
    let base = hpke_info(2, &[0xb1; 16], &fpr_b, &fpr_a, &head, &hh);
    // A blob sealed for one recipient, epoch, sealer, head or record header
    // has a different info string, so replaying it elsewhere fails to open.
    assert_ne!(base, hpke_info(2, &[0xa1; 16], &fpr_a, &fpr_a, &head, &hh));
    assert_ne!(base, hpke_info(1, &[0xb1; 16], &fpr_b, &fpr_a, &head, &hh));
    assert_ne!(base, hpke_info(2, &[0xb1; 16], &fpr_b, &fpr_b, &head, &hh));
    assert_ne!(
        base,
        hpke_info(2, &[0xb1; 16], &fpr_b, &fpr_a, &[7; 32], &hh)
    );
    assert_ne!(
        base,
        hpke_info(2, &[0xb1; 16], &fpr_b, &fpr_a, &head, &[8; 32])
    );
    // A replayed old-epoch key is caught by the commitment in the signed record.
    assert_ne!(
        key_commitment_ke(&counting(0)),
        key_commitment_ke(&counting(0x40))
    );
}

// ---------------------------------------------------------------------------
// The owner's decisions stay pinned in the specification and the ADR
// ---------------------------------------------------------------------------

const ADR_0077: &str = include_str!("../../../docs/adr/0077-wire-formats.md");
const ADR_INDEX: &str = include_str!("../../../docs/adr/README.md");

#[test]
fn owner_decisions_are_pinned_in_the_spec_and_the_adr() {
    for needle in [
        // Crypto 1: the tag secret and its entropy invariant.
        "**Non-rotating**",
        "sealed together with `K_e`",
        "UUID version 4",
        // Crypto 2: independent keys that never leave Rust or roam.
        "crosses the IPC boundary",
        "never roam, sync or restore",
        "strict Ed25519 verification",
        // Crypto 3: sealed keys only inside a signed epoch record.
        "travels **only** inside a signed epoch record",
        "`KeyCommitmentMismatch`",
        // PQ posture and F5.
        "`UnsupportedSuite`",
        "**before** it rejects",
        // Export A, C and D.
        "`SetHouseholdTimezone`",
        "Each derived key seals exactly",
        "**never** the legacy 19 MiB profile",
        "**Full-history guarantee.**",
    ] {
        assert!(SPEC.contains(needle), "spec lost: {needle}");
    }
    // ADR 0077 records the decisions and is indexed as a public proposal.
    for needle in [
        "- **Tier:** Public",
        "non-rotating, vault-scoped secret `S`",
        "independent** key pairs",
        "epoch record signed by an authorized member",
        "Classical now, with agility",
        "every class-1 ledger row of every age",
    ] {
        assert!(ADR_0077.contains(needle), "ADR 0077 lost: {needle}");
    }
    assert!(
        ADR_INDEX.contains(
            "| 0077 | [Data ownership and wire formats](0077-wire-formats.md) | Public | Accepted"
        ),
        "the ADR index must list 0077"
    );
}

// ---------------------------------------------------------------------------
// Portable bundle rules
// ---------------------------------------------------------------------------

#[test]
fn export_header_accepts_only_the_exact_kdf_allowlist() {
    let salt = [3u8; 32];
    assert!(parse_export_header(&export_header(EXPORT_PROFILE_INTERACTIVE, &salt, 2)).is_ok());
    assert!(parse_export_header(&export_header(EXPORT_PROFILE_HIGH, &salt, 2)).is_ok());

    // LegacyCompatibility (19 MiB, t = 2) is never a writer profile.
    let mut legacy = export_header(EXPORT_PROFILE_INTERACTIVE, &salt, 2);
    let label = 2 + "dohflow/export-header/v1".len();
    legacy[label + 2 + 1..label + 2 + 1 + 4].copy_from_slice(&19_456u32.to_be_bytes());
    assert!(matches!(
        parse_export_header(&legacy),
        Err(WireError::Semantic(_))
    ));
    // An unknown profile id is rejected before any KDF work.
    let mut unknown = export_header(EXPORT_PROFILE_INTERACTIVE, &salt, 2);
    unknown[label + 2] = 9;
    assert!(matches!(
        parse_export_header(&unknown),
        Err(WireError::Semantic(_))
    ));
    // Trailing bytes and a zero object count are malformed.
    let mut trailing = export_header(EXPORT_PROFILE_INTERACTIVE, &salt, 2);
    trailing.push(0);
    assert_eq!(
        parse_export_header(&trailing),
        Err(WireError::TrailingBytes)
    );
    assert!(parse_export_header(&export_header(EXPORT_PROFILE_INTERACTIVE, &salt, 0)).is_err());
}

#[test]
fn export_objects_bind_header_path_index_and_length() {
    let kek = counting(9);
    let hh = sha256(b"header");
    let key = export_object_key(&kek, &hh, 1);
    let aad = export_object_aad(&hh, "data/a.jsonl", 1, 0, true, 5);
    let ct = seal(&key, &[0u8; 12], &aad, b"hello");
    assert!(open(&key, &[0u8; 12], &aad, &ct));
    for wrong in [
        export_object_aad(&sha256(b"other header"), "data/a.jsonl", 1, 0, true, 5),
        export_object_aad(&hh, "data/b.jsonl", 1, 0, true, 5),
        export_object_aad(&hh, "data/a.jsonl", 2, 0, true, 5),
        export_object_aad(&hh, "data/a.jsonl", 1, 1, true, 5),
        export_object_aad(&hh, "data/a.jsonl", 1, 0, false, 5),
        export_object_aad(&hh, "data/a.jsonl", 1, 0, true, 6),
    ] {
        assert!(!open(&key, &[0u8; 12], &wrong, &ct));
    }
    // Every object has its own key, and the manifest key uses a distinct label.
    assert_ne!(
        export_object_key(&kek, &hh, 1),
        export_object_key(&kek, &hh, 2)
    );
    assert_ne!(
        export_object_key(&kek, &hh, 0),
        export_object_key(&kek, &hh, 1)
    );
    // A retry draws a new salt, which changes the header hash and every key.
    let h1 = sha256(&export_header(EXPORT_PROFILE_INTERACTIVE, &[1; 32], 2));
    let h2 = sha256(&export_header(EXPORT_PROFILE_INTERACTIVE, &[2; 32], 2));
    assert_ne!(
        export_object_key(&kek, &h1, 1),
        export_object_key(&kek, &h2, 1)
    );
}

// ---------------------------------------------------------------------------
// Epoch-record acceptance (Part C.6/C.7), modelled without signatures
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EpochFacts {
    key_epoch: u64,
    record_id: [u8; 32],
    prev_epoch_id: Option<[u8; 32]>,
    head_version: u64,
    head_id: [u8; 32],
    /// The record is a revocation cutover.
    cutover: bool,
    /// The record's `membership_head` is a `revoke` record.
    head_is_revoke: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum EpochError {
    StaleMembershipHead,
    MembershipChainFork,
    ChainBehind,
    EpochEquivocation,
    BadEpochSequence,
    CutoverHeadNotRevoke,
}

/// What a recipient has verified so far. Signature, recipient-set and
/// commitment checks come first in the real protocol; this model pins the
/// freshness and sequencing rules that decide whether a well-formed record is
/// still acceptable.
struct Recipient {
    head_version: u64,
    head_id: [u8; 32],
    latest: Option<EpochFacts>,
    /// Every distinct record seen for an epoch, retained for diagnosis.
    retained: Vec<EpochFacts>,
}

impl Recipient {
    fn accept(&mut self, r: EpochFacts) -> Result<(), EpochError> {
        // Equivocation first: a different record for an epoch already accepted.
        if let Some(prev) = self.latest {
            if r.key_epoch == prev.key_epoch && r.record_id != prev.record_id {
                self.retained.push(r);
                return Err(EpochError::EpochEquivocation);
            }
            if r.key_epoch == prev.key_epoch {
                return Ok(()); // an exact duplicate is idempotent
            }
        }
        if r.head_version < self.head_version {
            return Err(EpochError::StaleMembershipHead);
        }
        if r.head_version == self.head_version && r.head_id != self.head_id {
            self.retained.push(r);
            return Err(EpochError::MembershipChainFork);
        }
        if r.head_version > self.head_version {
            return Err(EpochError::ChainBehind); // fetch and verify the chain first
        }
        let expected_epoch = self.latest.map_or(1, |p| p.key_epoch + 1);
        let expected_prev = self.latest.map(|p| p.record_id);
        if r.key_epoch != expected_epoch || r.prev_epoch_id != expected_prev {
            return Err(EpochError::BadEpochSequence);
        }
        if r.cutover && !r.head_is_revoke {
            return Err(EpochError::CutoverHeadNotRevoke);
        }
        self.latest = Some(r);
        self.retained.push(r);
        Ok(())
    }
}

fn recipient_at(version: u64, head: u8, epoch1: Option<EpochFacts>) -> Recipient {
    Recipient {
        head_version: version,
        head_id: [head; 32],
        latest: epoch1,
        retained: epoch1.into_iter().collect(),
    }
}

fn epoch1() -> EpochFacts {
    EpochFacts {
        key_epoch: 1,
        record_id: [0xe1; 32],
        prev_epoch_id: None,
        head_version: 4,
        head_id: [4; 32],
        cutover: false,
        head_is_revoke: false,
    }
}

#[test]
fn a_stale_head_record_signed_by_a_since_revoked_member_is_rejected() {
    // The recipient has verified the chain through version 5, where device R was
    // revoked. R signs a well-formed epoch-2 record that names the older head
    // (version 4), where R was still authorized.
    let mut me = recipient_at(5, 5, Some(epoch1()));
    let stale = EpochFacts {
        key_epoch: 2,
        record_id: [0xe2; 32],
        prev_epoch_id: Some([0xe1; 32]),
        head_version: 4,
        head_id: [4; 32],
        cutover: true,
        head_is_revoke: false,
    };
    assert_eq!(me.accept(stale), Err(EpochError::StaleMembershipHead));
    assert_eq!(me.latest, Some(epoch1()), "no key from the rejected record");
}

#[test]
fn a_cutover_must_name_the_revoke_record_as_its_head() {
    let mut me = recipient_at(5, 5, Some(epoch1()));
    let record = |head_is_revoke| EpochFacts {
        key_epoch: 2,
        record_id: [0xe2; 32],
        prev_epoch_id: Some([0xe1; 32]),
        head_version: 5,
        head_id: [5; 32],
        cutover: true,
        head_is_revoke,
    };
    assert_eq!(
        me.accept(record(false)),
        Err(EpochError::CutoverHeadNotRevoke)
    );
    assert_eq!(me.accept(record(true)), Ok(()));
}

#[test]
fn epoch_numbers_are_sequential_and_chained() {
    let mut me = recipient_at(4, 4, Some(epoch1()));
    let mut skip = epoch1();
    skip.key_epoch = 3;
    skip.record_id = [0xe3; 32];
    skip.prev_epoch_id = Some([0xe1; 32]);
    assert_eq!(me.accept(skip), Err(EpochError::BadEpochSequence));
    let mut wrong_prev = epoch1();
    wrong_prev.key_epoch = 2;
    wrong_prev.record_id = [0xe2; 32];
    wrong_prev.prev_epoch_id = Some([0x99; 32]);
    assert_eq!(me.accept(wrong_prev), Err(EpochError::BadEpochSequence));
}

#[test]
fn equivocating_epoch_records_and_forked_chains_are_rejected_and_retained() {
    let mut me = recipient_at(4, 4, None);
    assert_eq!(me.accept(epoch1()), Ok(()));
    assert_eq!(
        me.accept(epoch1()),
        Ok(()),
        "an exact duplicate is idempotent"
    );
    let mut other = epoch1();
    other.record_id = [0xaa; 32];
    assert_eq!(me.accept(other), Err(EpochError::EpochEquivocation));
    assert!(
        me.retained.contains(&other),
        "both records stay for diagnosis"
    );
    assert!(me.retained.contains(&epoch1()));

    // A second record at the same chain version with a different id is a fork.
    let mut fork = EpochFacts {
        key_epoch: 2,
        record_id: [0xe2; 32],
        prev_epoch_id: Some([0xe1; 32]),
        head_version: 4,
        head_id: [0x77; 32],
        cutover: false,
        head_is_revoke: false,
    };
    assert_eq!(me.accept(fork), Err(EpochError::MembershipChainFork));
    // A head newer than anything verified means: fetch the chain first.
    fork.head_version = 6;
    assert_eq!(me.accept(fork), Err(EpochError::ChainBehind));
}

// ---------------------------------------------------------------------------
// Artifact seal guard (Part C.9): the zero nonce needs a fixed plaintext
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum SealError {
    ArtifactDescriptorChanged,
}

/// A client records the plaintext digest of the first seal of every
/// `(key_epoch, sync_artifact_id)` and refuses a different one before it
/// encrypts, so the fixed zero nonce never protects two plaintexts.
#[derive(Default)]
struct SealGuard {
    sealed: std::collections::HashMap<(u64, [u8; 32]), [u8; 32]>,
}

impl SealGuard {
    fn plan(&mut self, epoch: u64, id: [u8; 32], plaintext: &[u8]) -> Result<(), SealError> {
        let digest = sha256(plaintext);
        match self.sealed.get(&(epoch, id)) {
            None => {
                self.sealed.insert((epoch, id), digest);
                Ok(())
            }
            Some(first) if *first == digest => Ok(()),
            Some(_) => Err(SealError::ArtifactDescriptorChanged),
        }
    }
}

#[test]
fn a_changed_descriptor_for_a_published_address_is_refused_before_encryption() {
    let mut g = SealGuard::default();
    let id = [7u8; 32];
    assert_eq!(g.plan(1, id, b"descriptor v1 + bytes"), Ok(()));
    // A re-publication of the same bytes is the same plaintext: allowed.
    assert_eq!(g.plan(1, id, b"descriptor v1 + bytes"), Ok(()));
    // A renamed file or corrected MIME type changes the plaintext: refused.
    assert_eq!(
        g.plan(1, id, b"descriptor v2 + bytes"),
        Err(SealError::ArtifactDescriptorChanged)
    );
    // A new epoch re-addresses and re-seals under a new key: allowed.
    assert_eq!(g.plan(2, [8u8; 32], b"descriptor v2 + bytes"), Ok(()));
}

// ---------------------------------------------------------------------------
// Byte-level determinism rules settled in review
// ---------------------------------------------------------------------------

#[test]
fn the_manifest_commitment_presence_rule_is_exact() {
    let (x, y) = ([1u8; 32], [2u8; 32]);
    let c = manifest_commitment(&[x, y]);
    // An envelope with no artifacts carries an absent commitment.
    assert!(manifest_presence_ok(false, &[], None));
    assert!(!manifest_presence_ok(
        false,
        &[],
        Some(manifest_commitment(&[]))
    ));
    // An envelope with artifacts carries exactly their commitment.
    assert!(manifest_presence_ok(false, &[x, y], Some(c)));
    assert!(!manifest_presence_ok(false, &[x, y], None));
    assert!(!manifest_presence_ok(false, &[x, y], Some([0; 32])));
    // A snapshot always carries one, even for an empty closure.
    assert!(manifest_presence_ok(
        true,
        &[],
        Some(manifest_commitment(&[]))
    ));
    assert!(!manifest_presence_ok(true, &[], None));
}

#[test]
fn manifest_lists_must_be_strictly_ascending() {
    let (x, y) = ([1u8; 32], [2u8; 32]);
    assert!(validate_manifest_list(&[x, y]).is_ok());
    assert!(validate_manifest_list(&[y, x]).is_err());
    assert!(validate_manifest_list(&[x, x]).is_err());
}

#[test]
fn the_write_set_hash_sorts_raw_bytes_not_length_prefixed_encodings() {
    // Raw order puts "ab" before "b"; a length-prefixed order would put "b"
    // first (shorter length). The hash must follow the raw order.
    let (d1, d2) = ([1u8; 32], [2u8; 32]);
    let rows = [("b", &[0u8][..], d2), ("ab", &[0u8][..], d1)];
    let expected = {
        let e = Enc::new("dohflow/sync-writeset/v1")
            .u32(2)
            .text("ab", 64)
            .bytes(&[0], 256)
            .fixed(&d1)
            .text("b", 64)
            .bytes(&[0], 256)
            .fixed(&d2);
        sha256(&e.done())
    };
    assert_eq!(write_set_hash(&rows), expected);
    // Input order never matters.
    assert_eq!(write_set_hash(&rows), write_set_hash(&[rows[1], rows[0]]));
    // Primary keys sort bytewise within a table, shorter first on a prefix.
    let by_pk = [("t", &[1u8, 0][..], d1), ("t", &[1u8][..], d2)];
    assert_eq!(
        write_set_hash(&by_pk),
        write_set_hash(&[by_pk[1], by_pk[0]])
    );
}

#[test]
fn the_object_count_and_file_count_have_stated_bounds() {
    let salt = [3u8; 32];
    assert!(parse_export_header(&export_header(1, &salt, MAX_OBJECTS)).is_ok());
    assert!(parse_export_header(&export_header(1, &salt, MAX_OBJECTS + 1)).is_err());
    // The snapshot file count has its own bound.
    assert_eq!(MAX_FILES, 1024);
}

#[test]
fn the_fingerprint_commits_to_the_suite_the_kem_and_a_variable_length_key() {
    let ed = [0xe1u8; 32];
    let x = [0xc1u8; 32];
    let base = device_fingerprint(SUITE_ID, KEM_ID_X25519, &ed, &x);
    // A different suite or KEM id yields a different identity.
    assert_ne!(base, device_fingerprint(2, KEM_ID_X25519, &ed, &x));
    assert_ne!(base, device_fingerprint(SUITE_ID, 0x0021, &ed, &x));
    // A future KEM public key of a different length enrolls without a new
    // membership-record layout, and cannot collide with a 32-byte key.
    let hybrid = [0xc1u8; 1216];
    assert_ne!(base, device_fingerprint(SUITE_ID, 0x0030, &ed, &hybrid));
    let long = membership_body(1, None, 1, &base, &[0xa1; 16], &ed, &hybrid, 0);
    assert!(long.len() > 1216);
}
