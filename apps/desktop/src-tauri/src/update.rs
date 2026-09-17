//! In-app from-source updater (personal-cfo-1ik.3 detect / 1ik.4 apply).
//!
//! A **dev-machine** updater: it compares the commit the app was built from (stamped by
//! `build.rs` into `PCFO_GIT_COMMIT`) against the source checkout's current-branch upstream,
//! using `git`, and can rebuild + reinstall + relaunch from that checkout ([`apply`] /
//! [`relaunch`], via `scripts/update-app.sh`). `git fetch` is the only network call, and it runs
//! here in the **trusted Rust handler** — not the webview — so the app's `connect-src 'self'` CSP
//! is unchanged (ADR 0003/0010 addendum). Spawning `git`/`bash` is a direct
//! `std::process::Command`, not the Tauri shell plugin, so it needs no capability grant.

use std::path::Path;
use std::process::Command;

/// The commit the running binary was built from, and the checkout it was built in
/// (`build.rs`). `PCFO_GIT_COMMIT` is `"unknown"` when built outside a git tree.
const BUILT_COMMIT: &str = env!("PCFO_GIT_COMMIT");
const REPO_ROOT: &str = env!("PCFO_REPO_ROOT");
/// Build identity stamped by `build.rs` (personal-cfo-4d8.27.3.1), so the running app can
/// say WHICH build it is: `dev` vs `release` (or an explicit channel), when it was built,
/// and whether the worktree had uncommitted changes at the time.
pub(crate) const BUILD_CHANNEL: &str = env!("PCFO_BUILD_CHANNEL");
const BUILD_TIME: &str = env!("PCFO_BUILD_TIME");
const BUILD_DIRTY: bool = matches!(env!("PCFO_GIT_DIRTY").as_bytes(), b"true");

/// The result of an update check. `checked` is false when the check could not run (no source
/// checkout, no upstream, git unavailable); `error` then explains why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateStatus {
    pub current_version: String,
    pub current_commit: String,
    /// `dev` | `release` (or an explicit `PCFO_BUILD_CHANNEL`).
    pub build_channel: String,
    /// When this binary was built, RFC 3339 UTC.
    pub build_time: String,
    /// Whether tracked files were modified in the worktree at build time — the stamped
    /// commit alone would otherwise imply the build matches it exactly.
    pub build_dirty: bool,
    pub latest_commit: Option<String>,
    pub commits_behind: Option<u32>,
    pub latest_date: Option<String>,
    pub up_to_date: bool,
    pub checked: bool,
    pub error: Option<String>,
}

impl UpdateStatus {
    fn unchecked(built_commit: &str, error: impl Into<String>) -> Self {
        Self {
            current_version: env!("CARGO_PKG_VERSION").to_owned(),
            current_commit: built_commit.to_owned(),
            build_channel: BUILD_CHANNEL.to_owned(),
            build_time: BUILD_TIME.to_owned(),
            build_dirty: BUILD_DIRTY,
            latest_commit: None,
            commits_behind: None,
            latest_date: None,
            up_to_date: false,
            checked: false,
            error: Some(error.into()),
        }
    }
}

/// Check for an update against the app's own source checkout.
#[must_use]
pub fn check() -> UpdateStatus {
    check_repo(REPO_ROOT, BUILT_COMMIT)
}

/// Testable core: check `repo_root` for commits ahead of `built_commit` on the current
/// branch's upstream. Deterministic given the repo state.
fn check_repo(repo_root: &str, built_commit: &str) -> UpdateStatus {
    if !Path::new(repo_root).join(".git").exists() {
        return UpdateStatus::unchecked(
            built_commit,
            "Source checkout not found — updates need the repo you built from.",
        );
    }
    // Bound the fetch so a stalled/slow remote can't hang the check: abort a transfer that
    // creeps under 1 KB/s for 15s (HTTP), and cap SSH connect time (in GIT_SSH_COMMAND).
    if let Err(e) = run_git(
        repo_root,
        &[
            "-c",
            "http.lowSpeedLimit=1000",
            "-c",
            "http.lowSpeedTime=15",
            "fetch",
            "--quiet",
        ],
    ) {
        return UpdateStatus::unchecked(built_commit, format!("Couldn't reach the remote: {e}"));
    }
    let latest_commit =
        match run_git(repo_root, &["rev-parse", "--short", "@{u}"]) {
            Ok(c) => c,
            Err(_) => return UpdateStatus::unchecked(
                built_commit,
                "No upstream branch to compare against — switch to a tracked branch (e.g. main).",
            ),
        };
    let latest_date = run_git(repo_root, &["show", "-s", "--format=%cs", "@{u}"]).ok();

    // How many commits the *built* commit is behind the upstream. Best-effort: if the built
    // commit isn't in the repo (unknown / rebased away), we can't count it.
    let commits_behind = if built_commit == "unknown" {
        None
    } else {
        run_git(
            repo_root,
            &["rev-list", "--count", &format!("{built_commit}..@{{u}}")],
        )
        .ok()
        .and_then(|s| parse_count(&s))
    };

    UpdateStatus {
        current_version: env!("CARGO_PKG_VERSION").to_owned(),
        current_commit: built_commit.to_owned(),
        build_channel: BUILD_CHANNEL.to_owned(),
        build_time: BUILD_TIME.to_owned(),
        build_dirty: BUILD_DIRTY,
        latest_commit: Some(latest_commit),
        commits_behind,
        latest_date,
        up_to_date: commits_behind == Some(0),
        checked: true,
        error: None,
    }
}

/// Run `git -C <repo_root> <args>` with a PATH that resolves `git` even when the app was
/// launched from Finder with a minimal environment. Returns trimmed stdout, or stderr on
/// failure.
fn run_git(repo_root: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .env("PATH", tool_path())
        // Never block on an interactive credential / SSH prompt — fail fast if auth is needed,
        // so a background check can't hang.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes -oConnectTimeout=15")
        .output()
        .map_err(|e| format!("git is not available ({e})"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// PATH with the usual toolchain locations prepended (mirrors `scripts/build-app.sh`), so a
/// Finder-launched app can still find `git`.
fn tool_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let extra =
        format!("{home}/.cargo/bin:/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin");
    std::env::var("PATH").map_or(extra.clone(), |p| format!("{extra}:{p}"))
}

/// Parse `git rev-list --count` output (a bare integer) into a count.
fn parse_count(s: &str) -> Option<u32> {
    s.trim().parse::<u32>().ok()
}

/// The outcome of applying an update (personal-cfo-1ik.4): whether the reinstall succeeded, and
/// the tail of its output for surfacing a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyResult {
    pub ok: bool,
    pub output_tail: String,
}

/// Rebuild + reinstall the app from the source checkout by running the shared
/// `scripts/update-app.sh` (the same engine as the terminal one-liner). Blocking + minutes-long;
/// the caller runs it off the main thread. The script pulls (when the tree is clean), builds,
/// and installs into `/Applications`.
#[must_use]
pub fn apply() -> ApplyResult {
    apply_in(REPO_ROOT)
}

/// Testable core: run the update script under `repo_root`.
fn apply_in(repo_root: &str) -> ApplyResult {
    let fail = |tail: String| ApplyResult {
        ok: false,
        output_tail: tail,
    };
    let script = format!("{repo_root}/scripts/update-app.sh");
    if !Path::new(&script).exists() {
        return fail("The update script wasn't found in your source checkout.".to_owned());
    }

    // A dirty tree makes update-app.sh skip the pull and rebuild the *same* commit — a silent
    // no-op. Refuse with a clear message rather than "succeed" without actually updating. Only
    // TRACKED changes count: untracked files (e.g. a stray SESSION_HANDOFF.md) don't block a
    // fast-forward pull, so `--untracked-files=no` keeps them from blocking the update
    // (personal-cfo-1ik.5).
    match run_git(
        repo_root,
        &["status", "--porcelain", "--untracked-files=no"],
    ) {
        Ok(dirty) if !dirty.is_empty() => {
            return fail(
                "You have uncommitted changes in the source checkout — commit or stash them, \
                 then update."
                    .to_owned(),
            );
        }
        Ok(_) => {}
        Err(e) => return fail(format!("Couldn't inspect the source checkout: {e}")),
    }

    // Single-flight: a create-new lockfile stops a second concurrent build/install (which would
    // race on the target dir + the /Applications bundle). Released on completion.
    let lock = Path::new(repo_root)
        .join("target")
        .join(".pcfo-update.lock");
    if let Some(parent) = lock.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .is_err()
    {
        return fail(format!(
            "An update is already in progress. If you're sure none is running, delete {}.",
            lock.display()
        ));
    }

    let result = run_update_script(repo_root, &script);
    let _ = std::fs::remove_file(&lock);
    result
}

/// Run `scripts/update-app.sh` via `bash` (don't rely on the executable bit surviving), with the
/// toolchain PATH so git/pnpm/cargo resolve even from a Finder-launched app.
fn run_update_script(repo_root: &str, script: &str) -> ApplyResult {
    match Command::new("bash")
        .arg(script)
        .current_dir(repo_root)
        .env("PATH", tool_path())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
    {
        Ok(o) => {
            let mut combined = String::from_utf8_lossy(&o.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&o.stderr));
            ApplyResult {
                ok: o.status.success(),
                output_tail: tail_lines(&combined, 20),
            }
        }
        Err(e) => ApplyResult {
            ok: false,
            output_tail: format!("Couldn't run the update script: {e}"),
        },
    }
}

/// The last `n` non-empty-trimmed lines of `s`, joined — enough to show why an update failed
/// without dumping the whole build log.
fn tail_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n").trim().to_owned()
}

/// Relaunch into the freshly-installed app. Spawns a **detached** helper that waits for this
/// process to exit, then `open`s `/Applications/DohFlow.app` — so the new bundle launches
/// rather than re-activating the quitting instance. The caller exits right after.
pub fn relaunch() {
    let _ = Command::new("sh")
        .arg("-c")
        .arg("sleep 1; open '/Applications/DohFlow.app'")
        .env("PATH", tool_path())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn parse_count_reads_a_bare_integer() {
        assert_eq!(parse_count("3"), Some(3));
        assert_eq!(parse_count(" 0 \n"), Some(0));
        assert_eq!(parse_count("nope"), None);
        assert_eq!(parse_count(""), None);
    }

    #[test]
    fn tail_lines_keeps_the_last_n_lines() {
        assert_eq!(tail_lines("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(tail_lines("only", 5), "only");
        assert_eq!(tail_lines("", 3), "");
    }

    #[test]
    fn apply_degrades_when_the_update_script_is_missing() {
        let result = apply_in("/no/such/repo/anywhere");
        assert!(!result.ok);
        assert!(result.output_tail.contains("wasn't found"));
    }

    #[test]
    fn apply_ignores_untracked_but_refuses_tracked_changes() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path();
        Command::new("git")
            .args(["init", "-b", "main"])
            .arg(repo)
            .output()
            .unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .output()
                .unwrap();
        };
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::create_dir_all(repo.join("scripts")).unwrap();
        // A stub update script, committed so the tree starts clean and the script is tracked.
        std::fs::write(
            repo.join("scripts/update-app.sh"),
            "#!/usr/bin/env bash\nexit 0\n",
        )
        .unwrap();
        std::fs::write(repo.join("tracked.txt"), "1").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "init"]);

        // Untracked-only dirt does NOT block — the guard passes and the (stub) script runs
        // (personal-cfo-1ik.5).
        std::fs::write(repo.join("untracked.txt"), "x").unwrap();
        let ran = apply_in(&repo.to_string_lossy());
        assert!(
            ran.ok,
            "untracked files must not block: {}",
            ran.output_tail
        );

        // A TRACKED change still blocks with the commit-or-stash message.
        std::fs::write(repo.join("tracked.txt"), "2").unwrap();
        let blocked = apply_in(&repo.to_string_lossy());
        assert!(!blocked.ok);
        assert!(
            blocked.output_tail.contains("uncommitted changes"),
            "got: {}",
            blocked.output_tail
        );
    }

    #[test]
    fn a_missing_checkout_is_unchecked_not_a_crash() {
        let status = check_repo("/no/such/repo/anywhere", "abc1234");
        assert!(!status.checked);
        assert!(status.error.is_some());
        assert!(!status.up_to_date);
    }

    /// Helper: run a command in `dir`, panicking on failure (test setup only).
    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git available in test env");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    #[test]
    fn reports_commits_behind_the_upstream() {
        let tmp = tempdir().unwrap();
        let origin = tmp.path().join("origin.git");
        let work = tmp.path().join("work");
        Command::new("git")
            .args(["init", "--bare", "-b", "main"])
            .arg(&origin)
            .output()
            .unwrap();
        Command::new("git")
            .args(["clone"])
            .arg(&origin)
            .arg(&work)
            .output()
            .unwrap();
        // Identity + non-signing so commits succeed in CI-like envs.
        git(&work, &["config", "user.email", "t@example.com"]);
        git(&work, &["config", "user.name", "Test"]);
        git(&work, &["config", "commit.gpgsign", "false"]);
        std::fs::write(work.join("a.txt"), "1").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", "one"]);
        let first = git(&work, &["rev-parse", "--short", "HEAD"]);
        std::fs::write(work.join("a.txt"), "2").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", "two"]);
        let second = git(&work, &["rev-parse", "--short", "HEAD"]);
        git(&work, &["push", "-u", "origin", "main"]);

        let repo = work.to_string_lossy();
        // Built from the first commit → one behind.
        let behind = check_repo(&repo, &first);
        assert!(behind.checked, "error: {:?}", behind.error);
        assert_eq!(behind.commits_behind, Some(1));
        assert!(!behind.up_to_date);
        assert_eq!(behind.latest_commit.as_deref(), Some(second.as_str()));
        assert!(behind.latest_date.is_some());

        // Built from the tip → up to date.
        let current = check_repo(&repo, &second);
        assert_eq!(current.commits_behind, Some(0));
        assert!(current.up_to_date);
    }
}
