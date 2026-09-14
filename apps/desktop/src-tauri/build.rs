use std::path::PathBuf;
use std::process::Command;

/// Stamp build provenance into the binary for the in-app from-source updater
/// (personal-cfo-1ik.3): the commit the app was built from, and the repo root it was
/// built in. `check_for_update` compares this commit to the repo's upstream. Both are
/// best-effort — a build outside a git checkout still succeeds (commit = "unknown").
fn stamp_update_provenance() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // apps/desktop/src-tauri -> repo root (up three).
    let repo_root = manifest_dir
        .join("../../..")
        .canonicalize()
        .unwrap_or(manifest_dir);
    let repo_root = repo_root.to_string_lossy();

    let commit = Command::new("git")
        .args(["-C", &repo_root, "rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());

    // Whether the worktree had uncommitted tracked changes at build time
    // (personal-cfo-4d8.27.3.1). `git status --porcelain` prints a line per change, so
    // any output means dirty — the stamped commit alone would otherwise imply the build
    // matches that commit exactly. Unknowable outside a checkout → treated as clean.
    let dirty = Command::new("git")
        .args([
            "-C",
            &repo_root,
            "status",
            "--porcelain",
            "--untracked-files=no",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty());

    // The release profile is the proxy for "this is a real build": `tauri dev` and
    // `cargo run` are debug. An explicit PCFO_BUILD_CHANNEL wins, so a beta/pre-release
    // pipeline can label itself without a code change.
    let channel = std::env::var("PCFO_BUILD_CHANNEL").unwrap_or_else(|_| {
        match std::env::var("PROFILE").as_deref() {
            Ok("release") => "release".to_owned(),
            _ => "dev".to_owned(),
        }
    });

    // Build timestamp (RFC 3339, UTC). SOURCE_DATE_EPOCH is honoured so a reproducible
    // build can pin it; otherwise the wall clock at compile time.
    let build_time = build_timestamp();

    println!("cargo:rustc-env=PCFO_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=PCFO_REPO_ROOT={repo_root}");
    println!("cargo:rustc-env=PCFO_GIT_DIRTY={dirty}");
    println!("cargo:rustc-env=PCFO_BUILD_CHANNEL={channel}");
    println!("cargo:rustc-env=PCFO_BUILD_TIME={build_time}");
    // KEEPING THE STAMP HONEST. Emitting any `rerun-if-changed` opts out of cargo's
    // default "re-run when any package file changed", so the triggers below are the ONLY
    // ones — and a stale stamp is worse than none (it asserts a commit and a clean tree
    // that are not what you built).
    //
    // `.git/HEAD` alone is not enough: committing on the same branch rewrites
    // `.git/refs/heads/<branch>` (or `packed-refs`), never HEAD itself, so an ordinary
    // commit — or a `git pull --ff-only` — would leave the stamp behind. Watch the refs
    // too, and let the build scripts force a re-stamp via PCFO_BUILD_ID for the case no
    // file trigger can catch: editing a tracked file (which changes `dirty`, not git
    // metadata).
    println!("cargo:rerun-if-changed={repo_root}/.git/HEAD");
    println!("cargo:rerun-if-changed={repo_root}/.git/refs/heads");
    println!("cargo:rerun-if-changed={repo_root}/.git/packed-refs");
    println!("cargo:rerun-if-env-changed=PCFO_BUILD_ID");
    println!("cargo:rerun-if-env-changed=PCFO_BUILD_CHANNEL");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
}

/// The build timestamp as RFC 3339 UTC, without pulling a date crate into the build
/// script: seconds since the epoch → civil date via the standard algorithm.
fn build_timestamp() -> String {
    let secs: u64 = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        });
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant's civil_from_days.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn main() {
    stamp_update_provenance();
    tauri_build::build()
}
