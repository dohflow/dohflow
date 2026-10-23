//! The exact key and signature acceptance rules of Sync (`docs/architecture/vault-format.md`
//! C.6), stated as checks so that every implementation can reproduce them (personal-cfo-g3m.21).
//!
//! Ed25519 implementations disagree on edge cases (cofactored versus cofactorless verification,
//! ZIP-215 versus RFC 8032 acceptance, canonical encodings), and a membership chain accepted by one
//! implementation and rejected by another splits the views permanently. So the rule here is the
//! strict one, with every check named:
//!
//! 1. the public key `A` decompresses, re-compresses to the same 32 bytes (no non-canonical
//!    encoding), lies in the prime-order subgroup (`[L]A` is the identity, which excludes every
//!    mixed-order point and every point of order 2, 4 or 8) and is not the identity itself
//!    (order 1, in the subgroup trivially): `A` has order exactly `L`;
//! 2. the signature's `R` decompresses, re-compresses to the same 32 bytes, lies in the
//!    prime-order subgroup and is not the identity: `R` has order exactly `L`;
//! 3. the signature's `S`, as a little-endian integer, is below the group order `L`;
//! 4. the verification equation is the cofactorless one, as `verify_strict` computes it. With a
//!    torsion-free `A` and `R` and `S < L`, the cofactored and cofactorless equations accept exactly
//!    the same signatures, so an implementation's choice of equation can no longer matter.
//!
//! A suite-1 KEM public key (X25519) is also screened when it enters the chain: the all-zero
//! value and the small-order u-coordinates (with their non-canonical twins) are refused, because
//! sealing to a low-order key gives a predictable shared secret or blocks every later cutover.

use ed25519_dalek::{Signature, VerifyingKey};

/// The Ed25519 group order `L`, little-endian (RFC 8032).
const GROUP_ORDER_LE: [u8; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10,
];

/// Whether the 32 bytes encode an Ed25519 point acceptably: a valid point, in its one canonical
/// encoding, of order exactly `L`: in the prime-order subgroup (so not of mixed order) and not the
/// identity (`is_weak` rejects the identity and every small-order point).
#[must_use]
pub fn ed25519_point_acceptable(encoded: &[u8; 32]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(encoded) else {
        return false;
    };
    // Decompression accepts a y coordinate at or above the field prime; re-compressing yields the
    // canonical bytes, so a different result means the input was a non-canonical twin.
    let point = key.to_edwards();
    point.compress().to_bytes() == *encoded && !key.is_weak() && point.is_torsion_free()
}

/// Whether `s` (the second half of a signature, little-endian) is a canonical scalar, below `L`.
fn scalar_below_order(s: &[u8; 32]) -> bool {
    for i in (0..32).rev() {
        if s[i] != GROUP_ORDER_LE[i] {
            return s[i] < GROUP_ORDER_LE[i];
        }
    }
    false // equal to L
}

/// Whether `public_key` may enter a chain as an Ed25519 key (rule 1).
#[must_use]
pub fn ed25519_public_key_acceptable(public_key: &[u8; 32]) -> bool {
    ed25519_point_acceptable(public_key)
}

/// Verify `signature` over `message` under `public_key`, by the strict rule above.
#[must_use]
pub fn ed25519_verify_strict(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    if !ed25519_point_acceptable(public_key) {
        return false;
    }
    let (r, s) = signature.split_at(32);
    let r: [u8; 32] = r.try_into().expect("32 bytes");
    let s: [u8; 32] = s.try_into().expect("32 bytes");
    if !ed25519_point_acceptable(&r) || !scalar_below_order(&s) {
        return false;
    }
    let Ok(key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    key.verify_strict(message, &Signature::from_bytes(signature))
        .is_ok()
}

/// The field prime `2^255 - 19` as four little-endian 64-bit limbs.
const P: [u64; 4] = [
    0xffff_ffff_ffff_ffed,
    0xffff_ffff_ffff_ffff,
    0xffff_ffff_ffff_ffff,
    0x7fff_ffff_ffff_ffff,
];

fn limbs(bytes: &[u8; 32]) -> [u64; 4] {
    let mut out = [0_u64; 4];
    for (i, limb) in out.iter_mut().enumerate() {
        *limb = u64::from_le_bytes(bytes[8 * i..8 * i + 8].try_into().expect("8 bytes"));
    }
    out
}

fn at_least(a: &[u64; 4], b: &[u64; 4]) -> bool {
    for i in (0..4).rev() {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}

fn subtract(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
    let mut out = [0_u64; 4];
    let mut borrow = 0_u64;
    for i in 0..4 {
        let (d1, b1) = a[i].overflowing_sub(b[i]);
        let (d2, b2) = d1.overflowing_sub(borrow);
        out[i] = d2;
        borrow = u64::from(b1 || b2);
    }
    out
}

/// Whether the 32 bytes are an acceptable suite-1 KEM public key (an X25519 u-coordinate): not
/// one of the small-order values, whether written canonically or as a non-canonical twin (a value
/// at or above `p`, or with the unused top bit set).
#[must_use]
pub fn x25519_public_key_acceptable(public_key: &[u8; 32]) -> bool {
    let mut u = limbs(public_key);
    u[3] &= 0x7fff_ffff_ffff_ffff; // X25519 ignores the top bit
    if at_least(&u, &P) {
        u = subtract(&u, &P);
    }
    !LOW_ORDER_U_VALUES.iter().any(|low| limbs(low) == u)
}

/// The five reduced low-order u-coordinates as 32-byte little-endian values.
const LOW_ORDER_U_VALUES: [[u8; 32]; 5] = [
    // 0
    [0; 32],
    // 1
    [
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ],
    // p - 1
    [
        0xec, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x7f,
    ],
    // the first point of order 8
    [
        0xe0, 0xeb, 0x7a, 0x7c, 0x3b, 0x41, 0xb8, 0xae, 0x16, 0x56, 0xe3, 0xfa, 0xf1, 0x9f, 0xc4,
        0x6a, 0xda, 0x09, 0x8d, 0xeb, 0x9c, 0x32, 0xb1, 0xfd, 0x86, 0x62, 0x05, 0x16, 0x5f, 0x49,
        0xb8, 0x00,
    ],
    // the second point of order 8
    [
        0x5f, 0x9c, 0x95, 0xbc, 0xa3, 0x50, 0x8c, 0x24, 0xb1, 0xd0, 0xb1, 0x55, 0x9c, 0x83, 0xef,
        0x5b, 0x04, 0x44, 0x5c, 0xc4, 0x58, 0x1c, 0x8e, 0x86, 0xd8, 0x22, 0x4e, 0xdd, 0xd0, 0x9f,
        0x11, 0x57,
    ],
];
