//! The AES key schedule and the CTR state inside `Aes256Gcm` are scrubbed on drop
//! (personal-cfo-7ie.19).
//!
//! `Aes256Gcm` holds the expanded AES round keys, the CTR-mode state and the GHASH key schedule,
//! all functionally the raw key. `aes-gcm`'s own `zeroize` feature scrubs only the GHASH key it
//! derives while constructing a cipher; the long-lived key material lives in `aes`, `ctr` and
//! `ghash`, whose `zeroize` features the workspace dependencies enable (root `Cargo.toml`).
//!
//! What this does **not** cover: `ghash 0.5` and `polyval 0.6` keep their precomputed GHASH key
//! powers (derived from the cipher as `AES_K(0)`, a function of the key and not the key itself) in
//! a type that is not `ZeroizeOnDrop`, so no feature can scrub them; `ghash`'s feature scrubs only
//! the temporaries it builds. That residue cannot recover the AES key, and is accepted.
//!
//! These bounds compile only with those features on, so removing one from the workspace is a
//! compile error here and not a silent regression.

use aes_gcm::{Aes256Gcm, KeyInit};

fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}

#[test]
fn the_aes_round_keys_are_zeroized_on_drop() {
    assert_zeroize_on_drop::<aes::Aes128>();
    assert_zeroize_on_drop::<aes::Aes256>();
}

#[test]
fn the_ctr_mode_state_is_zeroized_on_drop() {
    assert_zeroize_on_drop::<ctr::Ctr32BE<aes::Aes256>>();
}

#[test]
fn aes_gcm_still_encrypts_and_decrypts_with_the_features_on() {
    use aes_gcm::aead::Aead;
    let cipher = Aes256Gcm::new_from_slice(&[7_u8; 32]).unwrap();
    let nonce = aes_gcm::Nonce::from_slice(&[1_u8; 12]);
    let sealed = cipher.encrypt(nonce, b"plaintext".as_ref()).unwrap();
    assert_eq!(
        cipher.decrypt(nonce, sealed.as_ref()).unwrap(),
        b"plaintext"
    );
}
