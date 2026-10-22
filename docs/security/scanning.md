# Security scanning

Automated checks that catch committed secrets and known-vulnerable dependencies
(`personal-cfo-6r6` / `-7t7`). They run in the `security-scan` CI job, unconditionally
on every trigger including a weekly schedule (`personal-cfo-fkt5.7`) — but that job
is guarded to run only once the repository is public (or on a manual dispatch). The
weekly schedule needs a real API call to check that, rather than the plain
`github.event.repository.private == false` guard every other job uses: a `schedule`
event's payload carries no `repository` object at all, and GitHub's expression
language happens to evaluate the resulting `null == false` comparison as `true` —
so a schedule-only job using the ordinary guard would silently run every week
regardless of visibility. `security-scan`'s own first step queries the API for real
instead (see that step's comment in `ci.yml`). Until the flip, gates should still be
run locally before pushing, like the other gates.

## Secret scanning — gitleaks (6r6)

Scans the full git history for credentials.

```sh
gitleaks detect --config .gitleaks.toml --no-banner --redact
```

The repo also contains material that *looks* like secrets but isn't — the generated
Beads data (`.beads/`, `scripts/beads/`) and the log-redaction test fixtures
(`crates/observability/src/lib.rs`, `crates/finance-kernel/tests/log_redaction.rs`),
which embed secret-shaped strings on purpose to prove they're scrubbed
(`personal-cfo-zobt`). `.gitleaks.toml` allowlists exactly those. Keep the allowlist
tight: prefer fixing a real finding over widening it.

## Real-value scanning — scripts/value-scan.sh (personal-cfo-o1nxk)

A different, complementary risk from gitleaks above: a real bank/brokerage
name, an SSN-shaped string, or a full card-number-shaped string — none of
which are "secrets" gitleaks' rules look for, but all of which would be a
privacy incident (this project's own demo/seed data is entirely invented on
purpose — `docs/agent/demo-vault.md`).

```sh
./scripts/value-scan.sh
```

Runs in CI, against the **committed** tree — this matters specifically
because the script's own development produced two real bugs, both only ever
caught by an independent reviewer running it against a tree the implementer's
own working copy hadn't yet pushed (see the script's own header comment for
the full account: a self-match against its own denylist, then the same
self-match recurring against its own test file once that was added). A CI
step against the actual pushed tree closes that class of gap structurally,
rather than relying on remembering to re-run the script by hand before every
push. `scripts/tests/value-scan.test.sh` (10 hermetic cases, throwaway git
repos) covers the regression directly plus every documented exception's
correctness *and* scope.

## Dependency scanning — cargo audit + pnpm audit (7t7)

Both Cargo workspaces (the root and the standalone desktop crate) and the frontend:

```sh
cargo audit                                       # root workspace
cargo audit -f apps/desktop/src-tauri/Cargo.lock  # desktop crate
pnpm -C apps/desktop audit --prod                 # frontend
```

A **vulnerability** fails the scan. Unmaintained / unsound *warnings* are
informational and do not fail it — notably the gtk-rs GTK3 bindings pulled in by
Tauri's Linux webview stack, which the macOS app does not use. If a warning ever
becomes an actual advisory, address it (update the dependency, or document an
explicit ignore with rationale).

### When the advisory scans are required, and the daily main audit (personal-cfo-md8h2.14)

The advisory database changes under us: a new advisory, or a withdrawn one that
leaves an allowance stale, can turn the scan red with no code change. Owner decision
`personal-cfo-915.15` (option B) therefore splits the work:

- **On a pull request or merge group** the required check
  `Security scanning (secrets + dependencies)` always runs and always reports. The
  **advisory scans** inside it (cargo audit, pnpm audit) are required only when the
  change touches a manifest, a lockfile or the audit setup (`Cargo.toml`,
  `Cargo.lock`, `package.json`, `pnpm-lock.yaml`, `pnpm-workspace.yaml`,
  `deny.toml`, `audit.toml`, `scripts/cargo-audit.py` and its test; the list is
  `scripts/dependency-change-check.sh`). On any other change they are skipped with
  an explicit "no dependency change" notice, and the job still succeeds. The check
  name does not change.
- **Always, on every change:** secrets scanning (gitleaks), the real-value scan, the
  ADR tier check, and the license scans (cargo deny, the frontend license script).
  None of them reads the advisory database.
- **On every push to `main`, on schedule and on manual dispatch** the advisory scans
  run in full, so a red advisory still turns `main` red and raises the
  `ci-main-red` alert.
- **Every day** `.github/workflows/advisory-audit.yml` audits `main` itself. On any
  finding, or on a stale allowance (an exception in `scripts/cargo-audit.py` that
  matches nothing), it opens, or comments on, **one** open issue labeled
  `security-advisory` that names the advisory IDs and includes the tool output. A
  clean run only comments on an open issue; it never closes one. A manual dispatch
  with `simulate` set exercises the issue path with a made-up finding (close the
  issue afterwards).

**Response to the daily alert.** The agent on duty, who sees the issue first:

1. Reads the issue and the linked run.
2. Files a **P0 bead** for it (ADR 0069: P0 bypasses queues), after telling
   `01-planning`, and links the issue and the bead to each other.
3. Fixes it (bump the dependency, or for a stale allowance remove it from
   `scripts/cargo-audit.py`), as the dependency-changing pull request that this
   split deliberately makes the one place the advisory scans are required.
4. Closes the issue once `main`'s audit is clean again.

While `main` is red on an advisory, a pull request that fails only on it may use the
red-main fast path (`docs/agent/WORKFLOW_ROLES.md`), except where an exclusion
applies.

## Dependency license scanning — cargo deny + a frontend script (personal-cfo-o1nxk)

DohFlow is AGPL-3.0-only (ADR 0043). A dependency under an incompatible
license is not a vulnerability `cargo audit`/`pnpm audit` would ever catch —
this is a separate check, against an explicit allowlist of licenses known to
combine cleanly with AGPL-3.0-only (permissive licenses, and copyleft
licenses compatible with or subsumed by AGPL: MIT, Apache-2.0, BSD-2/3-Clause,
ISC, Zlib, MPL-2.0, Unicode-3.0, CC0-1.0, BSL-1.0, OpenSSL, LGPL-2.1/3.0-or-later,
GPL-2.0/3.0-or-later, AGPL-3.0(-only), plus a handful the first real audit run
found and documented in place — see each config file's own comments,
particularly `CDLA-Permissive-2.0` and `Apache-2.0 WITH LLVM-exception` in the
Rust configs and `OFL-1.1`/`CC-BY-4.0`/`BlueOak-1.0.0`/`MIT-0`/`Python-2.0` in
the frontend script).

Two Cargo workspaces (root and the standalone desktop crate) each have their
own `deny.toml`, checked separately:

```sh
cargo deny check licenses                                                    # root workspace
cargo deny --manifest-path apps/desktop/src-tauri/Cargo.toml check licenses  # desktop crate
```

The frontend has no `cargo-deny` equivalent, so `scripts/check-frontend-licenses.mjs`
hand-rolls the same allowlist against `pnpm licenses list --json` rather than
adding a new devDependency for a one-shot CI gate:

```sh
node scripts/check-frontend-licenses.mjs --prod   # what the packaged app actually bundles
node scripts/check-frontend-licenses.mjs          # + devDependencies, for full transparency
```

Any license outside the allowlist is a **hard failure**, not a warning — and,
per the review that added this check (`personal-cfo-o1nxk`), a genuine
finding here is a `HUMAN_DECISION`: replace the dependency, document an
explicit, owner-approved exception, or block the release. An agent should
never quietly add an exception to either allowlist to make a red check green.

First-party crates in this repo carry `license = "AGPL-3.0-only"` (ADR
0043; `personal-cfo-fkt5.4`) but are unpublished — `[licenses.private]` in
both `deny.toml` files, backed by `publish = false` on every workspace
crate, tells `cargo-deny` to skip them and audit only what the app
actually *depends on*, not its own license field.

## Tools

`gitleaks`, `cargo-audit`, and `cargo-deny` are dev tools, not project
dependencies. Install with `brew install gitleaks cargo-deny` and
`cargo install cargo-audit` (CI installs all three via
`taiki-e/install-action`). `pnpm audit` and `pnpm licenses` are built into
pnpm; `scripts/check-frontend-licenses.mjs` and `scripts/value-scan.sh` need
no extra install — both are plain scripts against tools already required
(Node, `git`).
