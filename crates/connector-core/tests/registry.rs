//! The connector capability/cost/terms registry (personal-cfo-5jjz, ADR 0015)
//! over the real shipped roster: every adapter `all_connectors()` yields has
//! exactly one registry entry, every entry names a registered adapter, and the
//! entries are internally consistent.

// Link every shipped adapter crate so its `register_connector!` submission is
// collected (see this crate's dev-dependencies).
use simplefin_adapter as _;

use std::collections::BTreeSet;

use connector_core::{
    all_connectors, all_registrations, registration_by_id, review_date, validate_registrations,
    Payer,
};

/// The shipped adapter ids. Adding an adapter is a deliberate edit here.
const SHIPPED: &[&str] = &["simplefin"];

#[test]
fn every_shipped_adapter_has_exactly_one_registry_entry() {
    let adapters: Vec<&str> = all_connectors().map(|a| a.id()).collect();
    let entries: Vec<&str> = all_registrations().map(|r| r.adapter.id()).collect();

    let expected: BTreeSet<&str> = SHIPPED.iter().copied().collect();
    assert_eq!(adapters.iter().copied().collect::<BTreeSet<_>>(), expected);

    for id in &adapters {
        let count = entries.iter().filter(|e| *e == id).count();
        assert_eq!(count, 1, "{id} must have exactly one registry entry");
    }
    for id in &entries {
        assert!(
            adapters.contains(id),
            "registry entry {id} names no registered adapter"
        );
    }
    assert_eq!(validate_registrations(all_registrations()), Ok(()));
}

#[test]
fn only_simplefin_is_enabled_by_default() {
    let enabled: Vec<&str> = all_registrations()
        .filter(|r| r.metadata.enabled)
        .map(|r| r.adapter.id())
        .collect();
    assert_eq!(enabled, ["simplefin"]);
}

#[test]
fn simplefin_is_enabled_with_both_reviews_recorded() {
    let entry = registration_by_id("simplefin").expect("simplefin registered");
    let metadata = &entry.metadata;
    assert!(metadata.enabled);
    let economics = &metadata.economics;
    // Reviewed on or after the registry landed (2026-09-27, this bead's PR).
    let registry_landed = review_date(2026, 9, 27);
    assert!(economics.cost_reviewed_at >= registry_landed);
    assert!(economics.terms_reviewed_at >= registry_landed);
    assert!(economics.terms_url.is_some());
    assert_eq!(economics.payer, Payer::UserDirect);
    assert_eq!(economics.currency, Some("USD"));
    assert_eq!(metadata.regions, ["US"]);
}

#[test]
fn every_workspace_adapter_crate_is_linked_into_this_roster() {
    // A new crates/connectors/<name> member that is not a dev-dependency here
    // would register nowhere this test can see — fail instead of passing
    // blind.
    const WORKSPACE: &str = include_str!("../../../Cargo.toml");
    const THIS_CRATE: &str = include_str!("../Cargo.toml");
    let members: Vec<&str> = WORKSPACE
        .split('"')
        .filter_map(|s| s.strip_prefix("crates/connectors/"))
        .collect();
    assert!(members.contains(&"simplefin-adapter"), "parsed {members:?}");
    for member in members {
        assert!(
            THIS_CRATE.contains(&format!(
                "{member} = {{ path = \"../connectors/{member}\" }}"
            )),
            "{member} must be a connector-core dev-dependency so the registry tests see it"
        );
    }
}
