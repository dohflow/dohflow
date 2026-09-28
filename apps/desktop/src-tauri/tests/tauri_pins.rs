//! Tauri version pins and upgrade discipline (personal-cfo-io42, ADR 0001).
//!
//! ADR 0001 pins Tauri v2 to a minor version and makes every Tauri upgrade its
//! own PR. This test is the CI half of that rule. It fails when:
//!
//! * the resolved Tauri family — Rust `tauri*` / `wry` / `tao` in Cargo.lock,
//!   npm `@tauri-apps/*` in pnpm-lock.yaml — differs from the reviewed record in
//!   `tauri-pins.toml`, even by a patch release (`drift`);
//! * a manifest would let a new minor in: every Tauri spec in Cargo.toml and
//!   apps/desktop/package.json must be `~X.Y.Z` or exact (`manifest-range`);
//! * the Rust and npm halves fall out of lockstep: each Rust plugin and its npm
//!   package share a major.minor, `tauri` shares one with `@tauri-apps/cli` and
//!   `@tauri-apps/api`, and every CLI platform binary matches the CLI
//!   (`lockstep`).
//!
//! `check` is pure over file CONTENTS, so the tests below can feed it a
//! deliberately mismatched copy of any input. The real lockfiles are never
//! touched.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone)]
struct Inputs {
    cargo_toml: String,
    cargo_lock: String,
    package_json: String,
    pnpm_lock: String,
    pins: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Violation {
    rule: &'static str,
    subject: String,
    detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}: {}", self.rule, self.subject, self.detail)
    }
}

fn v(rule: &'static str, subject: impl Into<String>, detail: impl Into<String>) -> Violation {
    Violation {
        rule,
        subject: subject.into(),
        detail: detail.into(),
    }
}

/// Rust packages in the pinned family.
fn is_tauri_crate(name: &str) -> bool {
    name.starts_with("tauri") || name == "wry" || name == "tao"
}

/// Dependencies whose manifest spec must hold the minor (the crates this app
/// depends on directly and that Tauri versions in lockstep).
fn is_pinned_rust_dependency(name: &str) -> bool {
    name == "tauri" || name == "tauri-build" || name.starts_with("tauri-plugin-")
}

/// `~X.Y.Z` or an exact version (`=X.Y.Z` in Cargo, `X.Y.Z` in npm).
fn holds_minor(spec: &str, cargo: bool) -> bool {
    let version = if let Some(rest) = spec.strip_prefix('~') {
        rest
    } else if cargo {
        match spec.strip_prefix('=') {
            Some(rest) => rest,
            None => return false,
        }
    } else {
        spec
    };
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts[..2]
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        && !parts[2].is_empty()
}

fn major_minor(version: &str) -> String {
    version.split('.').take(2).collect::<Vec<_>>().join(".")
}

/// Resolved Rust Tauri-family versions from Cargo.lock: name → versions.
fn rust_resolved(cargo_lock: &str) -> Result<BTreeMap<String, Vec<String>>, Violation> {
    let lock: toml::Table = cargo_lock
        .parse()
        .map_err(|e: toml::de::Error| v("lock-shape", "Cargo.lock", e.to_string()))?;
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pkg in lock
        .get("package")
        .and_then(|p| p.as_array())
        .ok_or_else(|| v("lock-shape", "Cargo.lock", "no [[package]] entries"))?
    {
        let (Some(name), Some(version)) = (
            pkg.get("name").and_then(|n| n.as_str()),
            pkg.get("version").and_then(|n| n.as_str()),
        ) else {
            continue;
        };
        if is_tauri_crate(name) {
            out.entry(name.to_owned())
                .or_default()
                .push(version.to_owned());
        }
    }
    Ok(out)
}

/// Resolved `@tauri-apps/*` versions for the apps/desktop importer, plus every
/// `@tauri-apps/*` package key in the lockfile's `packages:` section.
fn npm_resolved(pnpm_lock: &str) -> (BTreeMap<String, String>, Vec<(String, String)>) {
    let mut importer = BTreeMap::new();
    let mut in_desktop = false;
    let mut current: Option<String> = None;
    let mut all_packages = Vec::new();
    let mut in_packages = false;
    for line in pnpm_lock.lines() {
        if !line.starts_with(' ') && !line.is_empty() {
            in_packages = line == "packages:";
            in_desktop = false;
        }
        if line.starts_with("  ") && !line.starts_with("   ") {
            in_desktop = line.trim_end() == "  apps/desktop:";
            if in_packages {
                // `  '@tauri-apps/cli-darwin-arm64@2.11.2':`
                let key = line.trim().trim_end_matches(':').trim_matches('\'');
                if let Some(rest) = key.strip_prefix("@tauri-apps/") {
                    if let Some((name, version)) = rest.rsplit_once('@') {
                        all_packages.push((format!("@tauri-apps/{name}"), version.to_owned()));
                    }
                }
            }
            continue;
        }
        if !in_desktop {
            continue;
        }
        let trimmed = line.trim();
        if let Some(name) = trimmed
            .strip_suffix(':')
            .map(|n| n.trim_matches('\''))
            .filter(|n| n.starts_with("@tauri-apps/"))
        {
            current = Some(name.to_owned());
        } else if let (Some(name), Some(version)) = (&current, trimmed.strip_prefix("version: ")) {
            let version = version.split('(').next().unwrap_or(version).trim();
            importer.insert(name.clone(), version.to_owned());
            current = None;
        }
    }
    (importer, all_packages)
}

fn check(inputs: &Inputs) -> Vec<Violation> {
    let mut out = Vec::new();

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Pins {
        rust: BTreeMap<String, String>,
        npm: BTreeMap<String, String>,
    }
    let pins: Pins = match toml::from_str(&inputs.pins) {
        Ok(p) => p,
        Err(e) => return vec![v("pins-shape", "tauri-pins.toml", e.to_string())],
    };

    // --- drift: Rust ---------------------------------------------------------
    match rust_resolved(&inputs.cargo_lock) {
        Ok(resolved) => {
            for (name, versions) in &resolved {
                if versions.len() > 1 {
                    out.push(v(
                        "drift",
                        name.clone(),
                        format!("resolved twice: {versions:?}"),
                    ));
                }
                match pins.rust.get(name) {
                    None => out.push(v(
                        "drift",
                        name.clone(),
                        format!("{} is in Cargo.lock but not pinned", versions.join(", ")),
                    )),
                    Some(pinned) if versions.iter().any(|v| v != pinned) => out.push(v(
                        "drift",
                        name.clone(),
                        format!("Cargo.lock has {}, pinned {pinned}", versions.join(", ")),
                    )),
                    Some(_) => {}
                }
            }
            for name in pins.rust.keys().filter(|n| !resolved.contains_key(*n)) {
                out.push(v(
                    "drift",
                    name.clone(),
                    "pinned but no longer in Cargo.lock",
                ));
            }
        }
        Err(e) => out.push(e),
    }

    // --- drift: npm ----------------------------------------------------------
    let (importer, all_packages) = npm_resolved(&inputs.pnpm_lock);
    for (name, version) in &importer {
        match pins.npm.get(name) {
            None => out.push(v(
                "drift",
                name.clone(),
                format!("{version} resolved but not pinned"),
            )),
            Some(pinned) if pinned != version => out.push(v(
                "drift",
                name.clone(),
                format!("pnpm-lock.yaml has {version}, pinned {pinned}"),
            )),
            Some(_) => {}
        }
    }
    for name in pins.npm.keys().filter(|n| !importer.contains_key(*n)) {
        out.push(v(
            "drift",
            name.clone(),
            "pinned but not a dependency of apps/desktop",
        ));
    }

    // --- manifests hold the minor ----------------------------------------------
    match inputs.cargo_toml.parse::<toml::Table>() {
        Ok(manifest) => {
            let mut tables: Vec<(String, &toml::Value)> =
                ["dependencies", "build-dependencies", "dev-dependencies"]
                    .iter()
                    .filter_map(|t| manifest.get(*t).map(|d| ((*t).to_owned(), d)))
                    .collect();
            if let Some(targets) = manifest.get("target").and_then(|t| t.as_table()) {
                for (target, body) in targets {
                    for t in ["dependencies", "build-dependencies", "dev-dependencies"] {
                        if let Some(d) = body.get(t) {
                            tables.push((format!("target.{target}.{t}"), d));
                        }
                    }
                }
            }
            for (table, deps) in tables {
                for (name, spec) in deps.as_table().into_iter().flatten() {
                    if !is_pinned_rust_dependency(name) {
                        continue;
                    }
                    let version = spec
                        .as_str()
                        .or_else(|| spec.get("version").and_then(|v| v.as_str()))
                        .unwrap_or("<no version>");
                    if !holds_minor(version, true) {
                        out.push(v(
                            "manifest-range",
                            format!("Cargo.toml [{table}] {name}"),
                            format!("`{version}` allows a new minor — use `~X.Y.Z` or `=X.Y.Z`"),
                        ));
                    }
                }
            }
        }
        Err(e) => out.push(v("manifest-shape", "Cargo.toml", e.to_string())),
    }
    match serde_json::from_str::<serde_json::Value>(&inputs.package_json) {
        Ok(pkg) => {
            for section in ["dependencies", "devDependencies", "optionalDependencies"] {
                for (name, spec) in pkg[section].as_object().into_iter().flatten() {
                    if !name.starts_with("@tauri-apps/") {
                        continue;
                    }
                    let spec = spec.as_str().unwrap_or("<non-string>");
                    if !holds_minor(spec, false) {
                        out.push(v(
                            "manifest-range",
                            format!("package.json {section} {name}"),
                            format!("`{spec}` allows a new minor — use `~X.Y.Z` or `X.Y.Z`"),
                        ));
                    }
                }
            }
        }
        Err(e) => out.push(v("manifest-shape", "package.json", e.to_string())),
    }

    // --- lockstep, on the pinned record (which drift ties to both lockfiles) -----
    for (rust_name, rust_version) in &pins.rust {
        if let Some(plugin) = rust_name.strip_prefix("tauri-plugin-") {
            if let Some(npm_version) = pins.npm.get(&format!("@tauri-apps/plugin-{plugin}")) {
                if major_minor(rust_version) != major_minor(npm_version) {
                    out.push(v(
                        "lockstep",
                        rust_name.clone(),
                        format!(
                            "Rust {rust_version} vs npm @tauri-apps/plugin-{plugin} {npm_version}"
                        ),
                    ));
                }
            }
        }
    }
    if let Some(tauri) = pins.rust.get("tauri") {
        for npm in ["@tauri-apps/cli", "@tauri-apps/api"] {
            match pins.npm.get(npm) {
                Some(version) if major_minor(version) == major_minor(tauri) => {}
                Some(version) => out.push(v(
                    "lockstep",
                    npm,
                    format!("{version} vs tauri {tauri} — the minors must match"),
                )),
                None => out.push(v("lockstep", npm, "missing from the pinned npm set")),
            }
        }
    }
    if let Some(cli) = importer.get("@tauri-apps/cli") {
        for (name, version) in &all_packages {
            if name.starts_with("@tauri-apps/cli-") && version != cli {
                out.push(v(
                    "lockstep",
                    name.clone(),
                    format!("{version} vs @tauri-apps/cli {cli}"),
                ));
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

// ---------------------------------------------------------------------------

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(manifest_dir().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn real_inputs() -> Inputs {
    Inputs {
        cargo_toml: read("Cargo.toml"),
        cargo_lock: read("Cargo.lock"),
        package_json: read("../package.json"),
        pnpm_lock: read("../../../pnpm-lock.yaml"),
        pins: read("tauri-pins.toml"),
    }
}

fn render(violations: &[Violation]) -> String {
    violations.iter().map(|v| format!("  {v}\n")).collect()
}

fn check_with(edit: impl FnOnce(&mut Inputs)) -> Vec<Violation> {
    let mut inputs = real_inputs();
    edit(&mut inputs);
    check(&inputs)
}

fn replace_once(text: &mut String, from: &str, to: &str) {
    assert_eq!(
        text.matches(from).count(),
        1,
        "`{from}` occurs exactly once"
    );
    *text = text.replacen(from, to, 1);
}

fn assert_reports(found: &[Violation], rule: &str, subject: &str) {
    assert!(
        found
            .iter()
            .any(|v| v.rule == rule && v.subject.contains(subject)),
        "expected a `{rule}` violation naming `{subject}`, got:\n{}",
        render(found)
    );
}

#[test]
fn the_resolved_tauri_family_matches_the_reviewed_pins() {
    let violations = check(&real_inputs());
    assert!(
        violations.is_empty(),
        "Tauri version drift — a Tauri change is a dedicated upgrade PR \
         (docs/agent/TAURI_UPGRADES.md) that updates tauri-pins.toml by hand:\n{}",
        render(&violations)
    );
}

#[test]
fn the_parsers_read_the_real_lockfiles() {
    // Sanity: the checks above are not vacuous.
    let rust = rust_resolved(&read("Cargo.lock")).expect("Cargo.lock parses");
    assert!(rust.contains_key("tauri") && rust.contains_key("wry") && rust.len() >= 15);
    let (importer, all) = npm_resolved(&read("../../../pnpm-lock.yaml"));
    assert!(importer.contains_key("@tauri-apps/api") && importer.contains_key("@tauri-apps/cli"));
    assert!(
        all.iter().any(|(n, _)| n.starts_with("@tauri-apps/cli-")),
        "{all:?}"
    );
}

#[test]
fn a_patch_bump_in_cargo_lock_is_drift() {
    let found = check_with(|i| {
        replace_once(
            &mut i.cargo_lock,
            "name = \"tauri\"\nversion = \"2.11.2\"",
            "name = \"tauri\"\nversion = \"2.11.3\"",
        )
    });
    assert_reports(&found, "drift", "tauri");
}

#[test]
fn a_new_tauri_crate_or_a_duplicate_version_is_drift() {
    let found = check_with(|i| {
        i.cargo_lock
            .push_str("\n[[package]]\nname = \"tauri-plugin-shell\"\nversion = \"2.3.0\"\n");
        i.cargo_lock
            .push_str("\n[[package]]\nname = \"wry\"\nversion = \"0.56.0\"\n");
    });
    assert_reports(&found, "drift", "tauri-plugin-shell");
    assert_reports(&found, "drift", "wry");
}

#[test]
fn an_npm_bump_in_pnpm_lock_is_drift() {
    let found = check_with(|i| {
        replace_once(
            &mut i.pnpm_lock,
            "specifier: ~2.11.0\n        version: 2.11.0",
            "specifier: ~2.11.0\n        version: 2.11.1",
        )
    });
    assert_reports(&found, "drift", "@tauri-apps/api");
}

#[test]
fn a_caret_or_loose_range_in_either_manifest_is_rejected() {
    let cargo = check_with(|i| {
        replace_once(
            &mut i.cargo_toml,
            "tauri-plugin-process = \"~2.3.1\"",
            "tauri-plugin-process = \"2\"",
        )
    });
    assert_reports(&cargo, "manifest-range", "tauri-plugin-process");
    let caret = check_with(|i| {
        replace_once(
            &mut i.cargo_toml,
            "tauri = { version = \"~2.11.2\", features = [] }",
            "tauri = { version = \"2.11.2\", features = [] }",
        )
    });
    assert_reports(&caret, "manifest-range", "[dependencies] tauri");
    let npm = check_with(|i| {
        replace_once(
            &mut i.package_json,
            "\"@tauri-apps/cli\": \"~2.11.2\"",
            "\"@tauri-apps/cli\": \"^2.11.2\"",
        )
    });
    assert_reports(&npm, "manifest-range", "@tauri-apps/cli");
}

#[test]
fn rust_and_npm_out_of_lockstep_is_rejected() {
    // A plugin upgraded on one side only — the pins updated to match, so this is
    // the lockstep rule firing, not drift.
    let found = check_with(|i| {
        replace_once(
            &mut i.pins,
            "tauri-plugin-dialog = \"2.7.1\"",
            "tauri-plugin-dialog = \"2.8.0\"",
        );
        replace_once(
            &mut i.cargo_lock,
            "name = \"tauri-plugin-dialog\"\nversion = \"2.7.1\"",
            "name = \"tauri-plugin-dialog\"\nversion = \"2.8.0\"",
        );
    });
    assert!(
        !found.iter().any(|v| v.rule == "drift"),
        "{}",
        render(&found)
    );
    assert_reports(&found, "lockstep", "tauri-plugin-dialog");

    let cli = check_with(|i| {
        replace_once(
            &mut i.pins,
            "\"@tauri-apps/cli\" = \"2.11.2\"",
            "\"@tauri-apps/cli\" = \"2.12.0\"",
        );
        replace_once(
            &mut i.pnpm_lock,
            "specifier: ~2.11.2\n        version: 2.11.2",
            "specifier: ~2.11.2\n        version: 2.12.0",
        );
    });
    assert_reports(&cli, "lockstep", "@tauri-apps/cli");
}

#[test]
fn a_cli_platform_binary_out_of_step_with_the_cli_is_rejected() {
    let (_, all) = npm_resolved(&read("../../../pnpm-lock.yaml"));
    let (platform, version) = all
        .iter()
        .find(|(n, _)| n.starts_with("@tauri-apps/cli-"))
        .cloned()
        .expect("a CLI platform binary");
    let found = check_with(|i| {
        let from = format!("\n  '{platform}@{version}':");
        let first = i.pnpm_lock.find(&from).expect("platform key");
        i.pnpm_lock.replace_range(
            first..first + from.len(),
            &format!("\n  '{platform}@2.99.0':"),
        );
    });
    assert_reports(&found, "lockstep", &platform);
}

#[test]
fn ci_runs_the_pin_check_on_every_change_that_can_move_tauri() {
    // The desktop job runs `cargo test` (this file). It must trigger on the Rust
    // side (apps/desktop/src-tauri/**, which holds Cargo.toml, Cargo.lock and the
    // pins) AND the npm side, or an npm-only Tauri bump would skip this check, the
    // capability suite and the runtime probe.
    let ci = read("../../../.github/workflows/ci.yml");
    let job = ci
        .split("\n  ipc-codegen:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n  linux-appimage-spike:").next())
        .expect("ipc-codegen job");
    for path in [
        "- 'apps/desktop/src-tauri/**'",
        "- 'apps/desktop/package.json'",
        "- 'pnpm-lock.yaml'",
    ] {
        assert!(
            job.contains(path),
            "the desktop job's path filter lost {path}"
        );
    }
}

#[test]
fn dependabot_keeps_tauri_bumps_out_of_generic_update_prs() {
    // Dependabot assigns a dependency to the FIRST group it matches, so the `tauri`
    // group must come before `minor-and-patch` in both ecosystems that carry the
    // Tauri family (see the TAURI note at the top of .github/dependabot.yml).
    let dependabot = read("../../../.github/dependabot.yml");
    for (ecosystem, patterns) in [
        (
            "package-ecosystem: \"cargo\"\n    directory: \"/apps/desktop/src-tauri\"",
            &["\"tauri\"", "\"tauri-*\"", "\"wry\"", "\"tao\""][..],
        ),
        (
            "package-ecosystem: \"npm\"\n    directory: \"/\"",
            &["\"@tauri-apps/*\""][..],
        ),
    ] {
        // Slice one `updates:` entry: from its `package-ecosystem` line up to the
        // next entry (the group pattern lists also start with "- ", deeper indented).
        let start = dependabot
            .find(ecosystem)
            .unwrap_or_else(|| panic!("dependabot entry {ecosystem}"));
        let rest = &dependabot[start..];
        let entry = &rest[..rest.find("\n  - package-ecosystem").unwrap_or(rest.len())];
        let tauri = entry
            .find("      tauri:")
            .unwrap_or_else(|| panic!("{ecosystem}: no tauri group"));
        let generic = entry
            .find("      minor-and-patch:")
            .expect("minor-and-patch group");
        assert!(
            tauri < generic,
            "{ecosystem}: the tauri group must be listed first"
        );
        for pattern in patterns {
            assert!(
                entry[tauri..generic].contains(pattern),
                "{ecosystem}: tauri group lost pattern {pattern}"
            );
        }
    }
}

#[test]
fn the_upgrade_procedure_and_pr_template_stay_complete() {
    // ADR 0001: an upgrade is its own PR that documents what changed. The template
    // must keep the sections that make it so, and the vendored-source checklist
    // must stay in step with the procedure doc.
    let template = read("../../../.github/PULL_REQUEST_TEMPLATE/tauri-upgrade.md");
    for section in [
        "## What changed (Tauri family)",
        "## Permission expansion diff",
        "## Vendored-source re-verification",
        "**Tauri-upgrade gates (all required):**",
        "## Review",
    ] {
        assert!(
            template.contains(section),
            "tauri-upgrade.md lost `{section}`"
        );
    }
    let doc = read("../../../docs/agent/TAURI_UPGRADES.md");
    let checklist_rows = |text: &str| {
        (1..=40)
            .filter(|n| text.contains(&format!("\n| {n} | ")))
            .count()
    };
    assert_eq!(
        checklist_rows(&template),
        checklist_rows(&doc),
        "the template's re-verification table and TAURI_UPGRADES.md's checklist differ"
    );
    assert!(
        checklist_rows(&doc) >= 13,
        "the vendored-source checklist shrank"
    );
    assert!(read("../../../scripts/tauri-acl-expansion.mjs").contains("acl-manifests.json"));
    // The release smoke must run under a throwaway identity: a release-profile
    // build ignores PCFO_DATA_DIR (ADR 0070), so the real identifier would write
    // into the owner's real data directory (io42 review F1).
    for (file, text) in [("TAURI_UPGRADES.md", &doc), ("tauri-upgrade.md", &template)] {
        assert!(
            text.contains("ai.personalcfo.upgradesmoke")
                && text.contains("ignores `PCFO_DATA_DIR`"),
            "{file} lost the throwaway-identity rule for the release smoke"
        );
    }
    assert!(
        doc.contains(
            r#""identifier":"ai.personalcfo.upgradesmoke","productName":"DohFlowUpgradeSmoke""#
        ),
        "TAURI_UPGRADES.md step 7 must build the smoke under the throwaway identifier"
    );
}
