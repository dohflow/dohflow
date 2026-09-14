//! Integration tests for the vault-crypto KDF (personal-cfo-j0o).
//!
//! The headline test is the **timing-regression** check: Argon2id is
//! data-independent, so deriving a KEK from a "wrong" password must take
//! essentially the same time as from the "correct" one (Risk Register §24 —
//! prevents a timing oracle on the unlock path). We assert the median latency
//! ratio stays within 2×.

use std::time::{Duration, Instant};

use vault_crypto::{derive_kek, generate_salt, Argon2Params, Profile, ALGORITHM, PARAMS_VERSION};

/// A deliberately cheap profile so the timing loop runs fast in CI while still
/// exercising the real Argon2id code path. (Argon2 floor: m_cost >= 8 * p_cost.)
fn fast_params() -> Argon2Params {
    Argon2Params {
        algorithm: ALGORITHM,
        memory_kib: 256,
        time_cost: 1,
        parallelism: 1,
        version: PARAMS_VERSION,
    }
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn median_derive_latency(
    password: &[u8],
    salt: &vault_crypto::Salt,
    params: &Argon2Params,
) -> Duration {
    // A couple of warm-up iterations, then measure the median of many so a
    // single descheduling spike cannot skew the result.
    for _ in 0..2 {
        let _ = derive_kek(password, salt, params).unwrap();
    }
    let mut samples = Vec::with_capacity(15);
    for _ in 0..15 {
        let start = Instant::now();
        let _ = derive_kek(password, salt, params).unwrap();
        samples.push(start.elapsed());
    }
    median(samples)
}

#[test]
fn wrong_password_latency_stays_within_constant_time_envelope() {
    let params = fast_params();
    let salt = generate_salt().unwrap();

    // Same-length passwords so input size is not a confound.
    let correct = b"correct-password";
    let wrong = b"wrongggg-passwrd";
    assert_eq!(correct.len(), wrong.len());

    let t_correct = median_derive_latency(correct, &salt, &params);
    let t_wrong = median_derive_latency(wrong, &salt, &params);

    let lo = t_correct.min(t_wrong).as_secs_f64();
    let hi = t_correct.max(t_wrong).as_secs_f64();
    // Guard against a zero/sub-microsecond floor producing a divide blow-up.
    let ratio = if lo > 1e-9 { hi / lo } else { 1.0 };

    assert!(
        ratio < 2.0,
        "derive_kek latency must be input-independent: correct={t_correct:?} wrong={t_wrong:?} ratio={ratio:.2}x"
    );
}

#[test]
fn end_to_end_unlock_shape() {
    // Mimics what personal-cfo-vhv will do: generate a salt at "create", persist
    // it (here just kept in memory), then re-derive the same KEK at "unlock".
    let params = Profile::InteractiveDefault.params();
    let salt = generate_salt().unwrap();

    let at_create = derive_kek(b"hunter2-but-better", &salt, &params).unwrap();
    let at_unlock = derive_kek(b"hunter2-but-better", &salt, &params).unwrap();
    assert_eq!(at_create.expose_bytes(), at_unlock.expose_bytes());

    // A different password at unlock derives a different KEK (the envelope
    // unwrap that vhv layers on top is what ultimately rejects it).
    let wrong = derive_kek(b"not-the-password!!", &salt, &params).unwrap();
    assert_ne!(at_create.expose_bytes(), wrong.expose_bytes());
}

#[test]
fn persisted_salt_round_trips() {
    let params = fast_params();
    let salt = generate_salt().unwrap();
    let raw: [u8; 16] = *salt.as_bytes();

    let original = derive_kek(b"pw", &salt, &params).unwrap();
    // Reconstruct the salt from persisted bytes (as vhv will from vault_metadata).
    let reloaded = vault_crypto::Salt::from_bytes(raw);
    let again = derive_kek(b"pw", &reloaded, &params).unwrap();

    assert_eq!(original.expose_bytes(), again.expose_bytes());
}
