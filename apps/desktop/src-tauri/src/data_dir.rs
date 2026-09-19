//! Dev/release vault data-directory separation (ADR 0070, `personal-cfo-he3xo`;
//! Cargo-profile gate added `personal-cfo-qrh3t`, ADR 0070 addendum).
//!
//! `app.path().app_data_dir()` derives from the frozen bundle identifier
//! `ai.personalcfo.desktop` (ADR 0067) and is otherwise channel-blind, so a dev
//! build and the installed release app would open the very same vault files.
//! [`resolve_data_dir`] is the pure split: a release build always gets the
//! identifier's own directory verbatim, and a dev build gets an explicit
//! `PCFO_DATA_DIR` override when set, else an automatic `<name>-dev` sibling.

use std::path::{Path, PathBuf};

/// Resolve the directory the app should store its vaults in.
///
/// The release path requires BOTH a non-`"dev"` channel label AND a
/// release-profile binary (`personal-cfo-qrh3t`, ADR 0070 addendum). Before
/// this, the split depended on the channel label alone: a debug binary built
/// with an explicit non-`"dev"` `PCFO_BUILD_CHANNEL` (e.g. `PCFO_BUILD_CHANNEL=beta
/// pnpm tauri dev`) was treated as release and pointed at the real vault
/// directory. A debug-profile binary now ALWAYS gets the dev path, whatever
/// its channel calls itself — `override_dir` is only inspected once that
/// combined check has already ruled out release, so nothing at runtime can
/// point a release-profile build somewhere else (`personal-cfo-h93wf`; ADR
/// 0070 decision 2, unchanged — this only closes the gap on the debug side).
pub fn resolve_data_dir(
    channel: &str,
    is_debug_build: bool,
    app_data_dir: PathBuf,
    override_dir: Option<PathBuf>,
) -> PathBuf {
    if channel != "dev" && !is_debug_build {
        return app_data_dir;
    }
    override_dir.unwrap_or_else(|| dev_sibling(&app_data_dir))
}

/// `.../Application Support/ai.personalcfo.desktop` ->
/// `.../Application Support/ai.personalcfo.desktop-dev`.
fn dev_sibling(app_data_dir: &Path) -> PathBuf {
    let name = app_data_dir
        .file_name()
        .expect("app_data_dir has a final path component");
    let mut dev_name = name.to_os_string();
    dev_name.push("-dev");
    match app_data_dir.parent() {
        Some(parent) => parent.join(dev_name),
        None => PathBuf::from(dev_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> PathBuf {
        PathBuf::from("/Users/test/Library/Application Support/ai.personalcfo.desktop")
    }

    #[test]
    fn release_returns_app_data_dir_unchanged() {
        assert_eq!(resolve_data_dir("release", false, base(), None), base());
    }

    #[test]
    fn release_ignores_override() {
        let override_dir = PathBuf::from("/tmp/somewhere-else");
        assert_eq!(
            resolve_data_dir("release", false, base(), Some(override_dir)),
            base()
        );
    }

    #[test]
    fn dev_default_is_a_distinct_sibling() {
        // The everyday `pnpm tauri dev` / `cargo run` shape: no explicit
        // channel override, so build.rs's PROFILE-derived default is "dev",
        // and the binary is debug-profile too.
        let resolved = resolve_data_dir("dev", true, base(), None);
        assert_ne!(resolved, base());
        assert_eq!(
            resolved,
            PathBuf::from("/Users/test/Library/Application Support/ai.personalcfo.desktop-dev")
        );
    }

    #[test]
    fn dev_and_release_paths_always_differ_with_no_override() {
        assert_ne!(
            resolve_data_dir("dev", true, base(), None),
            resolve_data_dir("release", false, base(), None)
        );
    }

    #[test]
    fn dev_override_wins_when_set() {
        let override_dir = PathBuf::from("/tmp/fixture-vault");
        assert_eq!(
            resolve_data_dir("dev", true, base(), Some(override_dir.clone())),
            override_dir
        );
    }

    #[test]
    fn unknown_channel_on_a_release_profile_build_is_treated_as_release() {
        // A RELEASE-PROFILE build with an unusual/mislabeled channel is still
        // release-safe by construction — there is no allow-list of "known dev
        // spellings" to keep in sync with build.rs. This is the direction
        // ADR 0070 decision 2 protects: nothing can accidentally point a real
        // release build elsewhere. (personal-cfo-qrh3t narrows this to
        // release-profile builds specifically — see the next test for the
        // debug-profile side, which used to share this same assertion and
        // was the actual gap.)
        assert_eq!(resolve_data_dir("beta", false, base(), None), base());
    }

    #[test]
    fn a_debug_profile_build_never_resolves_to_release_regardless_of_channel_label() {
        // personal-cfo-qrh3t, ADR 0070 addendum: before this, a debug binary
        // built with an explicit non-"dev" PCFO_BUILD_CHANNEL (e.g.
        // `PCFO_BUILD_CHANNEL=beta pnpm tauri dev`) was treated as release and
        // pointed `pnpm tauri dev` at the real vault directory. Now the
        // profile itself gates it, whatever the channel claims to be —
        // including the literal string "release".
        assert_ne!(resolve_data_dir("beta", true, base(), None), base());
        assert_ne!(resolve_data_dir("release", true, base(), None), base());
    }
}
