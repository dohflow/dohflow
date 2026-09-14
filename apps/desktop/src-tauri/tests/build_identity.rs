//! Build-identity guards (personal-cfo-4d8.27.3.1 / .3.5): the app can only tell you
//! *which* build you are running if the version has one source of truth and the
//! provenance stamps actually reach the binary.

/// personal-cfo-4d8.27.3.5: the app's version is written in two places — the crate
/// manifest (which `CARGO_PKG_VERSION`, and therefore the in-app "Current version",
/// reads) and `tauri.conf.json` (which names the bundle). They are kept in lockstep by
/// hand, so nothing stops them drifting and showing the user one number while the
/// installed bundle claims another. This fails the build the moment they disagree.
#[test]
fn the_app_version_has_a_single_source_of_truth() {
    let conf = include_str!("../tauri.conf.json");
    let conf: serde_json::Value =
        serde_json::from_str(conf).expect("tauri.conf.json is valid JSON");
    let bundle_version = conf
        .get("version")
        .and_then(serde_json::Value::as_str)
        .expect("tauri.conf.json declares a version");

    assert_eq!(
        bundle_version,
        env!("CARGO_PKG_VERSION"),
        "tauri.conf.json version ({bundle_version}) and Cargo.toml version ({}) drifted — \
         the app would report one version while the bundle claims another",
        env!("CARGO_PKG_VERSION"),
    );
}

/// The provenance `build.rs` stamps must reach the binary: without them the running app
/// cannot say whether it is a dev or a release build (personal-cfo-4d8.27.3.1).
#[test]
fn build_provenance_is_stamped_into_the_binary() {
    // The channel must be a real label. It is NOT pinned to "dev": `cargo test --release`
    // builds the release profile, and an explicit PCFO_BUILD_CHANNEL (the beta path
    // build.rs invites) is equally valid — pinning it made this test fail on workflows
    // the feature explicitly supports.
    let channel = env!("PCFO_BUILD_CHANNEL");
    assert!(
        !channel.is_empty() && !channel.contains(char::is_whitespace),
        "build channel should be a single label, got {channel:?}"
    );
    if std::env::var("PCFO_BUILD_CHANNEL").is_err() {
        let expected = if cfg!(debug_assertions) {
            "dev"
        } else {
            "release"
        };
        assert_eq!(
            channel, expected,
            "without an explicit channel, the profile decides it"
        );
    }

    // RFC 3339 UTC, e.g. 2026-07-20T14:32:05Z — parsed, not just non-empty, so a broken
    // civil-date conversion in build.rs is caught here rather than shown to a user.
    let built = env!("PCFO_BUILD_TIME");
    let parsed = chrono::DateTime::parse_from_rfc3339(built)
        .unwrap_or_else(|e| panic!("build time {built:?} is not RFC 3339: {e}"));
    assert!(
        parsed.timestamp() > 1_700_000_000,
        "build time {built} should be a plausible recent instant"
    );

    // The dirty flag is a real bool either way.
    let dirty = env!("PCFO_GIT_DIRTY");
    assert!(dirty == "true" || dirty == "false", "dirty flag: {dirty:?}");
}

/// ADR 0067 (personal-cfo-fkt5.3): humans see DohFlow, machines see personal-cfo. The
/// product name drives everything a user reads (bundle name, menu bar, window title); the
/// identifier drives everything the OS addresses — above all the app-data directory every
/// vault lives under. Renaming the identifier would orphan existing vaults; renaming the
/// product back would ship the old name. Both halves of the rule are pinned here.
#[test]
fn the_product_name_and_identifier_follow_the_rename_policy() {
    let conf = include_str!("../tauri.conf.json");
    let conf: serde_json::Value =
        serde_json::from_str(conf).expect("tauri.conf.json is valid JSON");
    let product_name = conf
        .get("productName")
        .and_then(serde_json::Value::as_str)
        .expect("tauri.conf.json declares a productName");
    let identifier = conf
        .get("identifier")
        .and_then(serde_json::Value::as_str)
        .expect("tauri.conf.json declares an identifier");

    assert_eq!(
        product_name, "DohFlow",
        "productName is what users see — it must be the public name (ADR 0067)"
    );
    assert_eq!(
        identifier, "ai.personalcfo.desktop",
        "the bundle identifier derives the app-data directory; changing it orphans every \
         existing vault (ADR 0067)"
    );
}
