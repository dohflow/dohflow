//! No key reaches a span or an event (personal-cfo-1cltt.2; logging policy, never-log corpus).
//!
//! A full genesis, grant, acceptance and custody run executes under a capturing `tracing`
//! subscriber at the most verbose level, with span events on. The keys are known to the test (the
//! devices' private keys, `K_e` of three epochs and `S`), so the captured text is searched for every
//! one of them in hex, base64 and the byte-array form a `{:?}` of a `[u8; 32]` prints. A positive
//! control proves the search finds a key that is logged on purpose.

mod common;

use std::io;
use std::sync::{Arc, Mutex};

use common::register::MemoryRegisterStore;
use common::{chain, dev_known, epoch_key, epoch_record, id_of, membership, tag_secret, VAULT};
use platform_mock::{MockSecretStore, MockWitness};
use platform_traits::CounterContext;
use sync_core::membership::{verify_chain, Action, Reason, VerifiedState};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::MakeWriter;
use vault_crypto::sync_artifact::{
    first_artifact_id, open_artifact, readdress_container, seal_artifact, Fixity,
};
use vault_crypto::sync_counter::{CounterRegister, VerifiedStateGuard};
use vault_crypto::sync_custody::SyncKeyCustody;
use vault_crypto::sync_envelope::{EpochKey, SealerIdentity};
use vault_crypto::sync_keys::epoch_key_commitment;
use vault_crypto::sync_records::{accept_records, create_genesis};
use vault_crypto::sync_snapshot::{SavedSnapshotFiles, SnapshotFields, SnapshotSealer};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn hex(bytes: &[u8], upper: bool) -> String {
    bytes
        .iter()
        .map(|b| {
            if upper {
                format!("{b:02X}")
            } else {
                format!("{b:02x}")
            }
        })
        .collect()
}

fn base64(bytes: &[u8], url: bool, pad: bool) -> String {
    let alphabet: &[u8] = if url {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
    } else {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    };
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..=chunk.len() {
            out.push(alphabet[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
        if pad {
            for _ in chunk.len()..3 {
                out.push('=');
            }
        }
    }
    out
}

/// Every textual form a 32-byte secret could take in a log line.
fn renderings(secret: &[u8; 32]) -> Vec<String> {
    let mut out = vec![hex(secret, false), hex(secret, true), format!("{secret:?}")];
    for url in [false, true] {
        for pad in [false, true] {
            out.push(base64(secret, url, pad));
        }
    }
    out
}

fn leaks(log: &str, secrets: &[[u8; 32]]) -> Vec<String> {
    secrets
        .iter()
        .flat_map(renderings)
        .filter(|needle| log.contains(needle.as_str()))
        .collect()
}

/// Run `body` under a verbose capturing subscriber and return what it logged.
fn captured(body: impl FnOnce()) -> String {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_span_events(FmtSpan::FULL)
        .with_writer(capture.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, body);
    let bytes = capture.0.lock().unwrap().clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The private bytes the scenario uses.
const SIGN_A: u8 = 0x31;
const KEM_A: u8 = 0x32;
const SIGN_B: u8 = 0x33;
const KEM_B: u8 = 0x34;

fn secrets() -> Vec<[u8; 32]> {
    vec![
        [SIGN_A; 32],
        [KEM_A; 32],
        [SIGN_B; 32],
        [KEM_B; 32],
        [1_u8 ^ 0x40; 32],
        [2_u8 ^ 0x40; 32],
        [3_u8 ^ 0x40; 32],
        [0x77; 32],
    ]
}

#[test]
fn a_genesis_a_grant_and_a_custody_round_trip_log_no_key() {
    let log = captured(|| {
        let a = dev_known(0xa1, SIGN_A, KEM_A);
        let b = dev_known(0xb1, SIGN_B, KEM_B);
        let tag = tag_secret(0x77);

        // The real genesis path (random keys, so its secrets are not in the search set, but its
        // behavior under the subscriber is part of the run) ...
        let founder = vault_crypto::sync_keys::DeviceKeys::generate().unwrap();
        let genesis = create_genesis(VAULT, [0xa2; 16], &founder, 1).unwrap();
        accept_records(
            &VerifiedState::empty(VAULT),
            &[&genesis.membership],
            &[&genesis.epoch],
            &founder,
        )
        .unwrap();

        // ... and a chain whose every secret the test knows: genesis, a grant to B, a cutover.
        let m1 = membership(1, None, Action::Add, &a, &a);
        let e1 = epoch_record(
            Reason::Genesis,
            id_of(&m1),
            None,
            &a,
            &epoch_key(1),
            &tag,
            &[&a],
        );
        let m2 = membership(2, Some(id_of(&m1)), Action::Add, &a, &b);
        let e2 = epoch_record(
            Reason::Grant,
            id_of(&m2),
            Some(id_of(&e1)),
            &a,
            &epoch_key(1),
            &tag,
            &[&b],
        );
        let m3 = membership(3, Some(id_of(&m2)), Action::Revoke, &a, &b);
        let e3 = epoch_record(
            Reason::Cutover,
            id_of(&m3),
            Some(id_of(&e2)),
            &a,
            &epoch_key(2),
            &tag,
            &[&a],
        );
        let e4 = epoch_record(
            Reason::Rotation,
            id_of(&m3),
            Some(id_of(&e3)),
            &a,
            &epoch_key(3),
            &tag,
            &[&a],
        );

        for (device, store) in [(&a, MockSecretStore::new()), (&b, MockSecretStore::new())] {
            let custody = SyncKeyCustody::new(&store, VAULT);
            custody.store_device_keys(&device.keys).unwrap();
            let loaded = custody.load_device_keys().unwrap();
            let walk_epochs: Vec<&[u8]> = if device.id == a.id {
                vec![&e1, &e2, &e3, &e4]
            } else {
                vec![&e1, &e2]
            };
            let walk_memberships: Vec<&[u8]> = if device.id == a.id {
                vec![&m1, &m2, &m3]
            } else {
                vec![&m1, &m2]
            };
            let accepted = accept_records(
                &VerifiedState::empty(VAULT),
                &walk_memberships,
                &walk_epochs,
                &loaded,
            )
            .unwrap();
            let shown = format!("{accepted:?} {loaded:?}");
            tracing::info!(accepted = %shown, "debug forms are redacted");
            custody.install(&accepted).unwrap();
            custody.load_epoch_key(&accepted.state, 1).unwrap();
            custody.load_tag_secret(&accepted.state).unwrap();
        }
    });
    assert!(
        log.contains("debug forms are redacted"),
        "the capture works: {log:?}"
    );
    assert_eq!(
        leaks(&log, &secrets()),
        Vec::<String>::new(),
        "a key reached the log"
    );
}

#[test]
fn the_search_finds_a_key_that_is_logged_on_purpose() {
    // Positive control: if this passes vacuously, the test above proves nothing.
    let secret = [0x77_u8; 32];
    for rendering in renderings(&secret) {
        let log = captured(|| tracing::info!("oops {rendering}"));
        assert!(!leaks(&log, &[secret]).is_empty(), "missed {rendering}");
    }
    let log = captured(|| tracing::info!(?secret, "oops"));
    assert!(!leaks(&log, &[secret]).is_empty());
}

#[test]
fn a_snapshot_seal_the_register_and_the_witness_log_no_key_and_no_payload() {
    const PAYLOAD: &str = "SNAPSHOT-PAYLOAD-MARKER-7f3a91c2";
    let dir = std::env::temp_dir().join(format!("dohflow-redaction-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let c = chain();
    let m: Vec<&[u8]> = c.memberships[..3].iter().map(Vec::as_slice).collect();
    let e: Vec<&[u8]> = c.epochs[..3].iter().map(Vec::as_slice).collect();
    let state = verify_chain(&VAULT, &m, &e).unwrap();
    let key = epoch_key(1);
    let sealer = SealerIdentity {
        vault_id: VAULT,
        device_id: c.a.id,
        fingerprint: c.a.fingerprint(),
    };
    let log = captured(|| {
        let (store, witness) = (MemoryRegisterStore::new(), MockWitness::new());
        let register = CounterRegister::new(&store, &witness);
        let files = SavedSnapshotFiles::new(&dir);
        let publish = SnapshotSealer {
            register: &register,
            files: &files,
            key: &key,
            sealer: &sealer,
            chain: &state,
        };
        let fields = SnapshotFields {
            object_id: [0x33; 16],
            base_seq: 41,
            payload_schema_version: 1,
            engine_version: "dohflow-test".to_owned(),
            schema_version: 65,
        };
        let payload = common::snapshot_plaintext(&[], PAYLOAD);
        let sealed = publish.seal(&fields, &payload).unwrap();
        let reused = publish.seal(&fields, &payload).unwrap();
        tracing::info!(sealed = ?sealed, reused = ?reused, register = ?register, "debug forms are redacted");
        let guard = VerifiedStateGuard::new(&witness);
        let _ = guard.check(&VAULT, None);
        let _ = register.check(&CounterContext {
            vault_id: VAULT,
            key_epoch: 1,
            fingerprint: *c.a.fingerprint().as_bytes(),
            object_type: 2,
        });
        publish.discard(&[0x33; 16]).unwrap();
    });
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        log.contains("debug forms are redacted"),
        "the capture works: {log:?}"
    );
    assert_eq!(
        leaks(&log, &secrets()),
        Vec::<String>::new(),
        "a key reached the log"
    );
    assert!(!log.contains(PAYLOAD), "the payload reached the log");
}

#[test]
fn an_artifact_seal_open_and_readdress_log_no_key_and_no_blob() {
    use sync_core::artifact::{ArtifactKind, TransportPlaintext, MAX_ARTIFACT_CONTAINER};
    const BLOB: &str = "ARTIFACT-BLOB-MARKER-3c9e51d7";
    let key_of = |epoch: u64, first: u8| {
        let raw: [u8; 32] = std::array::from_fn(|i| first.wrapping_add(i as u8));
        let secret = vault_crypto::SecretBytes::new(raw);
        let commitment = epoch_key_commitment(&secret);
        EpochKey::from_verified(VAULT, epoch, secret, &commitment).unwrap()
    };
    let known: Vec<[u8; 32]> = [0x00_u8, 0x40]
        .iter()
        .map(|first| std::array::from_fn(|i| first.wrapping_add(i as u8)))
        .collect();
    let log = captured(|| {
        let (k1, k2) = (key_of(1, 0x00), key_of(2, 0x40));
        let id = first_artifact_id(&k1, &[0x5a; 32]);
        let plaintext = TransportPlaintext {
            artifact_id: id,
            kind: ArtifactKind::Attachment,
            logical_metadata: br#"{"mime_type":"text/plain"}"#,
            blob: BLOB.as_bytes(),
        };
        let sealed = seal_artifact(&k1, &plaintext, Fixity::First).unwrap();
        let opened = open_artifact(&k1, &id, &sealed.container, MAX_ARTIFACT_CONTAINER).unwrap();
        let moved = readdress_container(&k1, &k2, &id, &sealed.container, Fixity::First).unwrap();
        tracing::info!(
            sealed = ?sealed,
            opened = ?opened,
            plaintext = ?opened.plaintext(),
            moved = ?moved,
            "debug forms are redacted"
        );
    });
    assert!(
        log.contains("debug forms are redacted"),
        "the capture works: {log:?}"
    );
    assert_eq!(
        leaks(&log, &known),
        Vec::<String>::new(),
        "a key reached the log"
    );
    assert!(!log.contains(BLOB), "the blob reached the log");
}
