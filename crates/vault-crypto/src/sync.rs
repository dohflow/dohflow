//! The idempotency-key tag of Sync (`docs/architecture/vault-format.md` C.3 and C.5;
//! personal-cfo-t2s4b part 6b).
//!
//! A command's idempotency key never travels in clear: its envelope header carries a *tag* of it,
//! so a device (or the service) can recognize a retry without learning the key. The tag is keyed
//! with the vault's shared tag secret `S`, which only enrolled devices hold, so it lives here
//! beside the other key derivations; `sync-core` (pure format code) takes the finished 32 bytes
//! as an input and wraps them in its own type.
//!
//! ```text
//! tag_key = HKDF-Expand(S, label "dohflow/sync-idem-tag-key/v1" ‖ uuid vault_id), 32 bytes
//! tag     = HMAC-SHA256(tag_key, label "dohflow/sync-idem-tag/v1" ‖ uuid canonical_idempotency_key)
//! ```
//!
//! `S` is uniformly random, so Expand alone is sound (no Extract step). The tag key's info
//! deliberately omits `protocol_version` and `key_epoch`: the tag must be stable across protocol
//! versions and key epochs (ADR 0074 decision 2, the dual-identity lookup after tail truncation),
//! so do not add either. Labels are CBE1 labels
//! (`u16` length, then ASCII), written here because this crate does not depend on `sync-core`;
//! the C.14 vectors in `tests/sync_idempotency_tag.rs` pin the bytes.
//!
//! # Secret handling
//!
//! The tag key lives in a [`SecretBytes`] (locked, zeroized on drop). The HKDF and HMAC values
//! used to compute it are short-lived stack values whose internal state is functionally
//! equivalent to `S` and to the tag key respectively; the RustCrypto crates do not zeroize their
//! core state, so those are scrubbed only by being overwritten. Same best-effort posture as the
//! rest of this crate (`storage_id` in `attachment.rs`).

use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::secret::SecretBytes;

/// The label of the tag-key derivation.
pub const TAG_KEY_LABEL: &str = "dohflow/sync-idem-tag-key/v1";
/// The label of the tag itself.
pub const TAG_LABEL: &str = "dohflow/sync-idem-tag/v1";

/// A CBE1 label: `u16` big-endian length, then the ASCII bytes.
fn label_bytes(label: &str) -> Vec<u8> {
    debug_assert!(
        (1..=64).contains(&label.len())
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"/_.-".contains(&b)),
        "bad CBE1 label {label:?}"
    );
    let mut out = Vec::with_capacity(2 + label.len());
    out.extend_from_slice(&(label.len() as u16).to_be_bytes());
    out.extend_from_slice(label.as_bytes());
    out
}

/// Derive the vault's idempotency-tag key from the tag secret `S`, in a protected buffer.
///
/// # Panics
/// Never in practice: a 32-byte HKDF-SHA256 output is far inside its limit.
#[must_use]
pub fn idempotency_tag_key(secret: &SecretBytes<32>, vault_id: &[u8; 16]) -> SecretBytes<32> {
    let hk = Hkdf::<Sha256>::from_prk(secret.expose()).expect("a 32-byte secret is a valid PRK");
    let mut info = label_bytes(TAG_KEY_LABEL);
    info.extend_from_slice(vault_id);
    SecretBytes::<32>::init(|out| hk.expand(&info, out).map_err(|_| ()))
        .expect("32 bytes is within HKDF-SHA256's output limit")
}

/// The tag of a command's `canonical_idempotency_key` (its 16 raw UUID bytes).
///
/// The caller is responsible for the key being a UUID version 4: `sync-core`'s `IdempotencyKey`
/// is the only way an envelope builder gets one, and `check_envelope` hands its tagger that type.
/// This function tags whatever 16 bytes it is given (the same key always has the same tag), so a
/// caller outside the envelope path must pass a key it has already validated.
#[must_use]
pub fn idempotency_key_tag(tag_key: &SecretBytes<32>, canonical_key: &[u8; 16]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(tag_key.expose())
        .expect("HMAC accepts a key of any length");
    mac.update(&label_bytes(TAG_LABEL));
    mac.update(canonical_key);
    mac.finalize().into_bytes().into()
}
