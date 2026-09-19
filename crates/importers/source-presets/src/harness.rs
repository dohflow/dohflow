//! The preset test harness (personal-cfo-gvidg AC #3): a generic check that
//! runs against every registered [`SourcePreset`] via the registry, plus a
//! shared helper each preset's own module uses for its detailed assertions.
//!
//! Split from each preset's own test module on purpose: the checks here
//! (fixture present, help_slug present, the fixture actually parses through
//! `csv-importer` without error) are the same for every preset and must stay
//! true as new presets are added, so they live once, driven off the
//! registry — not copy-pasted per preset. A preset's OWN test module (e.g.
//! `ynab::tests`) asserts what that source's specific export shape must
//! produce (which date, which sign, which category), which cannot be
//! generic across sources.

use csv_importer::GenericCsv;
use importer_core::{ImporterPlugin, ParserInput, SourcePreset};

/// Parse `preset`'s own `fixture_csv()` through the generic CSV importer,
/// applying `preset.hints()` exactly as the real import flow will (personal-
/// cfo-gvidg: presets are hints applied *before* a user's own mapping — this
/// harness never supplies one, since the fixture is built to need none).
pub fn parse_fixture(preset: &dyn SourcePreset) -> importer_core::ParsedBatch {
    let input = ParserInput::new(preset.fixture_csv().as_bytes().to_vec())
        .with_filename(format!("{}.csv", preset.id()));
    GenericCsv
        .parse(&input, &preset.hints())
        .unwrap_or_else(|e| {
            panic!(
                "preset {:?}'s own fixture failed to parse through its own hints: {e}",
                preset.id()
            )
        })
}

#[test]
fn every_registered_preset_has_a_fixture_and_a_help_slug() {
    let presets: Vec<_> = importer_core::all_presets().collect();
    assert!(
        !presets.is_empty(),
        "no presets registered — did a module forget `mod ynab;` or its own `register_preset!`?"
    );
    for preset in presets {
        assert!(
            !preset.fixture_csv().trim().is_empty(),
            "preset {:?} has an empty fixture_csv()",
            preset.id()
        );
        assert!(
            !preset.help_slug().trim().is_empty(),
            "preset {:?} has an empty help_slug()",
            preset.id()
        );
        assert!(
            !preset.verified_against().trim().is_empty(),
            "preset {:?} has an empty verified_against()",
            preset.id()
        );
        assert!(
            !preset.source_app_url().trim().is_empty(),
            "preset {:?} has an empty source_app_url()",
            preset.id()
        );
    }
}

#[test]
fn every_registered_preset_s_fixture_parses_without_error() {
    for preset in importer_core::all_presets() {
        let batch = parse_fixture(preset);
        assert!(
            !batch.records.is_empty(),
            "preset {:?}'s fixture parsed but produced zero records",
            preset.id()
        );
    }
}
