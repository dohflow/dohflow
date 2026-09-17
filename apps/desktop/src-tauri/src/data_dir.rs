//! Dev/release vault data-directory separation (ADR 0070, `personal-cfo-he3xo`).
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
/// A release build (`channel != "dev"`) always returns `app_data_dir`
/// unchanged — `override_dir` is not even inspected, so nothing at runtime can
/// point a release build somewhere else (`personal-cfo-h93wf`; ADR 0070
/// decision 2).
pub fn resolve_data_dir(
    channel: &str,
    app_data_dir: PathBuf,
    override_dir: Option<PathBuf>,
) -> PathBuf {
    if channel != "dev" {
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
        assert_eq!(resolve_data_dir("release", base(), None), base());
    }

    #[test]
    fn release_ignores_override() {
        let override_dir = PathBuf::from("/tmp/somewhere-else");
        assert_eq!(
            resolve_data_dir("release", base(), Some(override_dir)),
            base()
        );
    }

    #[test]
    fn dev_default_is_a_distinct_sibling() {
        let resolved = resolve_data_dir("dev", base(), None);
        assert_ne!(resolved, base());
        assert_eq!(
            resolved,
            PathBuf::from("/Users/test/Library/Application Support/ai.personalcfo.desktop-dev")
        );
    }

    #[test]
    fn dev_and_release_paths_always_differ_with_no_override() {
        assert_ne!(
            resolve_data_dir("dev", base(), None),
            resolve_data_dir("release", base(), None)
        );
    }

    #[test]
    fn dev_override_wins_when_set() {
        let override_dir = PathBuf::from("/tmp/fixture-vault");
        assert_eq!(
            resolve_data_dir("dev", base(), Some(override_dir.clone())),
            override_dir
        );
    }

    #[test]
    fn unknown_channel_is_treated_as_release() {
        // Anything other than the literal "dev" string is release-safe by
        // construction — there is no allow-list of "known dev spellings" to
        // keep in sync with build.rs.
        assert_eq!(resolve_data_dir("beta", base(), None), base());
    }
}
