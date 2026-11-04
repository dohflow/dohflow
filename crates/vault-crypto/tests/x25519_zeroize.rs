//! The X25519 secrets HPKE builds are scrubbed on drop (personal-cfo-1cltt.2).
//!
//! `hpke` 0.13 depends on `x25519-dalek` with default features off, which leaves its `zeroize`
//! feature off, so the recipient's private scalar and the raw Diffie-Hellman result would never be
//! wiped. The workspace turns the feature on through vault-crypto's own `x25519-dalek` entry (root
//! `Cargo.toml`), by the same device as `aes_zeroize.rs`.
//!
//! `x25519-dalek` 2.0.1 implements this with `#[zeroize(drop)]`, which gives the types a `Zeroize`
//! impl and a `Drop` that calls it, but not the `ZeroizeOnDrop` marker. So the guard is two checks:
//! the `Zeroize` bound compiles only with the feature on (losing it is a compile error here), and
//! `needs_drop` is true only because of the scrubbing `Drop` (these types are plain byte arrays
//! otherwise). The second check is behavioral in the sense that matters: a type with no `Drop`
//! cannot scrub itself.

use std::mem::needs_drop;

fn assert_zeroize<T: zeroize::Zeroize>() {}

#[test]
fn the_x25519_private_scalar_and_shared_secret_are_zeroized_on_drop() {
    assert_zeroize::<x25519_dalek::StaticSecret>();
    assert_zeroize::<x25519_dalek::SharedSecret>();
    assert!(needs_drop::<x25519_dalek::StaticSecret>());
    assert!(needs_drop::<x25519_dalek::SharedSecret>());
}

#[test]
fn zeroizing_a_static_secret_overwrites_it() {
    use zeroize::Zeroize;
    let mut secret = x25519_dalek::StaticSecret::from([0x42_u8; 32]);
    secret.zeroize();
    let public_of_zeroed = x25519_dalek::PublicKey::from(&secret);
    let public_of_zero =
        x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([0_u8; 32]));
    assert_eq!(public_of_zeroed.to_bytes(), public_of_zero.to_bytes());
}
