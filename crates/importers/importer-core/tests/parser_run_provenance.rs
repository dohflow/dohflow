//! personal-cfo-eay: a registered plugin exposes the `id` + `version` recorded
//! on every `parser_run` (§8.1.3 provenance), is reachable through the
//! compile-time registry, and drives auto-detection. The dummy plugin is
//! registered via the public `register_importer!` macro, so this also proves the
//! compile-time registration path works end to end.

use importer_core::{
    register_importer, ImporterPlugin, ParseError, ParsedBatch, ParserHints, ParserInput,
};
use semver::Version;

struct DummyCsv;

impl ImporterPlugin for DummyCsv {
    fn id(&self) -> &'static str {
        "dummy-csv"
    }
    fn display_name(&self) -> &'static str {
        "Dummy CSV"
    }
    fn version(&self) -> Version {
        Version::new(1, 2, 3)
    }
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["csv"]
    }
    fn detect_confidence(&self, input: &ParserInput) -> u16 {
        if input.extension().as_deref() == Some("csv") {
            9000
        } else {
            0
        }
    }
    fn parse(&self, _input: &ParserInput, _hints: &ParserHints) -> Result<ParsedBatch, ParseError> {
        Ok(ParsedBatch {
            source_format: "csv".to_owned(),
            accounts: vec![],
            records: vec![],
            warnings: vec![],
        })
    }
}

register_importer!(DummyCsv);

#[test]
fn registered_plugin_exposes_id_and_version() {
    let plugin = importer_core::plugin_by_id("dummy-csv")
        .expect("the dummy plugin is registered at compile time");
    // These two fields are recorded on every parser_run (eay §8.1.3 provenance).
    assert_eq!(plugin.id(), "dummy-csv");
    assert_eq!(plugin.version(), Version::new(1, 2, 3));
    assert!(!plugin.id().is_empty());
    assert!(plugin.version() > Version::new(0, 0, 0));
}

#[test]
fn lookup_by_unknown_id_is_none() {
    assert!(importer_core::plugin_by_id("does-not-exist").is_none());
}

#[test]
fn detection_picks_the_csv_plugin_by_extension() {
    let csv = ParserInput::new(b"a,b\n1,2\n".to_vec()).with_filename("statement.csv");
    let best = importer_core::detect_best(&csv).expect("a plugin detects the csv");
    assert_eq!(best.id(), "dummy-csv");

    // Nothing claims a .png → no detection.
    let png = ParserInput::new(vec![]).with_filename("photo.png");
    assert!(importer_core::detect_best(&png).is_none());
}

#[test]
fn parse_returns_a_batch() {
    let plugin = importer_core::plugin_by_id("dummy-csv").unwrap();
    let batch = plugin
        .parse(&ParserInput::new(vec![]), &ParserHints::default())
        .unwrap();
    assert_eq!(batch.source_format, "csv");
}
