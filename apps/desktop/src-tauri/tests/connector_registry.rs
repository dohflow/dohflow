//! Single-source guards for SimpleFIN's connection copy (personal-cfo-5jjz,
//! personal-cfo-dto2j; ADR 0015 §5 and its link-guide addendum).
//!
//! The provider picker renders SimpleFIN's disclosure panel from the
//! registry. Its text must stay byte-identical to the onboarding panel it
//! replaced (`BridgeEducation`, personal-cfo-kdw6), frozen in
//! `src/settings/connections/fixtures/simplefin-panel.shipped.txt`. The chain
//! is pinned in two links:
//!
//! 1. here — every registry string appears in the frozen shipped text, and
//!    the frontend's SimpleFIN DTO fixture equals what the registry emits;
//! 2. in vitest — `ProviderDisclosure` renders that fixture to exactly the
//!    frozen text.
//!
//! After a deliberate registry change, regenerate the fixture with
//! `REGENERATE_FIXTURES=1 cargo test --test connector_registry`, and review
//! the diff.

// Link the adapter crate so its registration is collected in this binary.
use simplefin_adapter as _;

use app_lib::ipc::commands::connector_adapters_impl;

const SHIPPED_PANEL: &str =
    include_str!("../../src/settings/connections/fixtures/simplefin-panel.shipped.txt");
const FIXTURE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../src/settings/connections/fixtures/simplefin-adapter.json"
);

#[test]
fn simplefin_copy_in_the_registry_is_the_shipped_panel_text() {
    let entry = connector_core::registration_by_id("simplefin").expect("registered");
    let disclosure = entry.metadata.disclosure;
    let guide = entry.metadata.link_guide;
    let mut points = vec![
        ("title", guide.title),
        ("independent_party", disclosure.independent_party),
        ("handles_credentials", disclosure.handles_credentials),
        ("cost_summary", disclosure.cost_summary),
        ("optional", disclosure.optional),
        ("refresh_note", guide.refresh_note),
    ];
    points.extend(guide.setup_steps.iter().map(|step| ("setup_step", *step)));
    for (point, text) in points {
        assert!(
            SHIPPED_PANEL.contains(text),
            "registry {point} drifted from the shipped panel:\n  registry: {text}\n  shipped: {SHIPPED_PANEL}"
        );
    }
}

#[test]
fn the_frontend_simplefin_fixture_is_what_the_registry_emits() {
    let listing = connector_adapters_impl(connector_core::all_registrations());
    let simplefin = listing
        .iter()
        .find(|a| a.adapter_id == "simplefin")
        .expect("simplefin in the listing");
    let emitted = serde_json::to_string_pretty(simplefin).unwrap() + "\n";
    if std::env::var_os("REGENERATE_FIXTURES").is_some() {
        std::fs::write(FIXTURE_PATH, &emitted).expect("write fixture");
    }
    let fixture = std::fs::read_to_string(FIXTURE_PATH).expect("fixture exists");
    assert_eq!(
        fixture, emitted,
        "the frontend fixture is stale — rerun with REGENERATE_FIXTURES=1 and review the diff"
    );
}
