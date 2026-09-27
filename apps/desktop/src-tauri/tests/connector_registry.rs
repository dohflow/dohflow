//! Single-source guard for the SimpleFIN disclosure (personal-cfo-5jjz,
//! ADR 0015 §5). The registry entry's four disclosure points are the shipped
//! onboarding copy (personal-cfo-kdw6) as rendered text. Until the provider
//! picker (personal-cfo-dto2j) renders the registry directly, the copy exists
//! in both places — this test fails the moment they drift apart.

// Link the adapter crate so its registration is collected in this binary.
use simplefin_adapter as _;

const BRIDGE_EDUCATION: &str = include_str!("../../src/onboarding/BridgeEducation.tsx");

/// The component's rendered text, approximately: tags dropped, `{" "}`
/// spacers kept as spaces, whitespace runs collapsed — which is what JSX
/// does to the text between tags.
fn rendered_text(tsx: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for ch in tsx.replace("{\" \"}", " ").chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn simplefin_disclosure_is_the_shipped_onboarding_copy() {
    let rendered = rendered_text(BRIDGE_EDUCATION);
    let entry = connector_core::registration_by_id("simplefin").expect("registered");
    let disclosure = entry.metadata.disclosure;
    for (point, text) in [
        ("independent_party", disclosure.independent_party),
        ("handles_credentials", disclosure.handles_credentials),
        ("cost_summary", disclosure.cost_summary),
        ("optional", disclosure.optional),
    ] {
        assert!(
            rendered.contains(text),
            "registry {point} drifted from BridgeEducation.tsx:\n  registry: {text}\n  rendered: {rendered}"
        );
    }
}
