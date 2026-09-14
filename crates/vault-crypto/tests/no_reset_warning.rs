//! Pins [`vault_crypto::CANONICAL_NO_RESET_WARNING`] to ADR 0002
//! (personal-cfo-n7bo). The ADR is the reviewed human source of truth for this
//! safety-critical copy; the const must quote it byte-for-byte, so a wording
//! change cannot silently diverge from the documented decision.

use vault_crypto::CANONICAL_NO_RESET_WARNING;

#[test]
fn warning_matches_adr_0002_verbatim() {
    let adr = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/adr/0002-local-encrypted-vault.md"
    ))
    .expect("read docs/adr/0002-local-encrypted-vault.md");

    assert!(
        adr.contains(CANONICAL_NO_RESET_WARNING),
        "ADR 0002 no longer contains the canonical no-reset warning verbatim — the \
         const and the ADR have diverged. Update both together (changing this copy \
         is an ADR change)."
    );
}
