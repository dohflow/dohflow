# Pre-release security review — v0.1.0

**Bead:** `personal-cfo-o1nxk` · **Scope:** internal, evidence-based (owner
decision, 2026-09-07 — the external pentest is deferred to `personal-cfo-5kua`/
`-mh83` until revenue justifies it; nothing here overstates what has and has
not been independently verified). Every section below states what was
checked, the exact command or evidence, the result, residual risk, and any
follow-up bead.

## 1. IPC surface audit (closes `personal-cfo-o4il`)

**What was checked:** every Tauri command registered in
`apps/desktop/src-tauri/permissions/app-commands.toml` (the non-destructive
Finance-Kernel surface) and `permissions/destructive-commands.toml` (the
destructive surface), cross-referenced against
`apps/desktop/src-tauri/capabilities/default.json` and `capabilities/destructive.json`.

**Command/evidence:**

```sh
cargo test --test acl_coverage
```

**Result:** 10/10 tests pass. This is a genuinely thorough, already-existing
drift guard, not new work here:

- `every_registered_command_has_exactly_one_acl_grant` — parses every
  `#[tauri::command]` registered in `src/lib.rs`'s `collect_commands!` and
  asserts each appears in exactly one of the two ACL files (~130 commands in
  `app-commands.toml`, 7 in `destructive-commands.toml`), no duplicates, no
  stale grants, floor of >100 registered commands.
- `destructive_grants_stay_a_deliberate_short_list` — pins the destructive
  set to exactly: `delete_vault`, `export_backup`,
  `export_transactions_csv`, `restore_backup`, `apply_update`,
  `relaunch_app`, `connector_forget`.
- `no_capability_grants_opener_default_shell_fs_or_http` — forbids
  `opener:default`, `allow-default-urls`, `allow-open-path`,
  `allow-reveal-item-in-dir`, and any `shell:`/`fs:`/`http:`-prefixed
  permission in either capability file.
- `the_main_window_opener_grant_is_exactly_open_url_scoped_to_dohflow` —
  exactly one `opener:*` permission across both files combined:
  `opener:allow-open-url` in `default.json` only, scope exactly
  `https://dohflow.app/*`.
- `the_opener_grant_did_not_loosen_the_csp`, `the_opener_plugin_injects_no_click_interceptor` —
  CSP retains `default-src 'self'`/`connect-src 'self' ipc: http://ipc.localhost`/`frame-src 'none'`;
  the opener plugin is registered with `open_js_links_on_click(false)`, never bare `::init()`.
- `the_updater_grant_is_exactly_updater_default_in_the_general_capability`,
  `the_updater_pubkey_and_endpoint_are_exactly_the_committed_values` —
  exactly one `updater:*` permission (`updater:default`); the pinned
  minisign pubkey and the single endpoint
  (`https://github.com/dohflow/dohflow/releases/latest/download/latest.json`)
  match the committed values exactly; `bundle.createUpdaterArtifacts == true`.
- `the_window_theme_grant_is_exactly_allow_set_theme_in_the_general_capability`,
  `the_process_grant_is_exactly_allow_restart_in_the_destructive_capability` —
  as named.

**Residual risk:** none identified beyond what's already guarded. One note,
not a defect: the connector command group (`connector_link`,
`connector_connections`, `connector_set_account_link`, `connector_sync`,
`connector_auto_sync`) carries its own inline caveat in
`app-commands.toml` (lines 159-164) that this whole group needs
re-evaluation before ever being granted to a non-main window — correctly
flagged in the source, not a live gap today (only the `main` window exists).

**Follow-up bead:** none.

## 2. Logs audited for sensitive data (closes `personal-cfo-sq0p`)

**What was checked:** the redaction layer (`crates/observability`) and its
test corpus, plus an owner spot-check of real dogfood logs from the
installed build.

**Command/evidence:**

```sh
cargo test --workspace   # includes crates/finance-kernel/tests/log_redaction.rs
```

**Result:** `zobt` (release-blocking log-redaction gate) is closed and
merged (PR #62): "the §6.6 corpus through the production `RedactingMakeWriter`
while a real kernel workload emits spans; no secret survives, no
over-redaction." The redaction pattern table covers emails, `password=`/
`secret=`/`token=`/`*_key=` fields, bearer/API-key-shaped tokens, `$`/
thousands-grouped numeric amounts, and 12-19 digit runs
(`docs/security/logging-redaction.md`).

**Residual risk (real, worth recording):** `docs/security/logging-redaction.md`
describes this as a "release-blocking CI gate," but the actual corpus tests
run inside the general `cargo test --workspace` job like every other unit
test — there is no CI step that names or isolates the redaction suite
specifically, so a change that broke redaction would fail the same generic
job any other Rust test failure would, not a distinctly-labeled security
gate. Functionally equivalent today (the workspace test job already runs on
every PR and blocks merge), but worth a small follow-up if a distinctly-named
gate is ever wanted for visibility. Not filed as a bead — low enough value to
not clutter the graph; noted here for the record.

**Owner step:** _[owner: read a sample of real dogfood logs from the
installed build and confirm no plaintext financial value appears; record the
result here — no log content in this document]._

**Follow-up bead:** none required; the owner step above is the only open
item in this section.

## 3. Dependency + license audit (closes `personal-cfo-xiwn`)

**What was checked:** `cargo audit` (vulnerabilities, both Rust workspaces),
`pnpm audit --prod` (frontend vulnerabilities), and — the one control that
did not exist at all before this bead — a dependency **license** audit for
AGPL-3.0-only compatibility, both Rust workspaces and the frontend.

### 3a. Vulnerabilities

```sh
cargo audit                                       # root workspace
cargo audit -f apps/desktop/src-tauri/Cargo.lock  # desktop crate
pnpm -C apps/desktop audit --prod                 # frontend
```

**Result — root workspace:** clean. One informational warning only
(`anyhow` 1.0.102, RUSTSEC-2026-0190, unsoundness in `downcast_mut()`) —
per `docs/security/scanning.md`'s existing policy, warnings are
informational and do not fail the scan.

**Result — desktop crate: found and fixed a real HIGH-severity finding.**
`quick-xml` 0.39.4 (pulled in transitively via `tauri` → `plist`, used
internally by Tauri for Apple `.plist`/`Info.plist` parsing — **not** the
OFX importer, and not any path that parses user-supplied files) carried two
CVSS-7.5 (high) advisories:

- `RUSTSEC-2026-0195` — unbounded namespace-declaration allocation in
  `NsReader`, memory-exhaustion DoS.
- `RUSTSEC-2026-0194` — quadratic run time checking a start tag for
  duplicate attribute names.

Both fixed by `quick-xml >=0.41.0`. `plist` is not our own direct
dependency (only pulled in via `tauri`), so a normal version bump in our
own `Cargo.toml` wasn't available — escalated to the owner per this bead's
own rule ("any finding rated high or critical stops the launch sequence and
is raised to the owner before continuing"). **Owner decision: apply the fix
now.** Added `plist = "1.10.1"` as an explicit (otherwise-unused) direct
dependency in `apps/desktop/src-tauri/Cargo.toml` — the standard mechanism
for forcing Cargo's resolver to a version newer than what a transitive
consumer (`tauri`) declares on its own — which resolves `quick-xml` to
`0.42.0`. Re-ran `cargo audit -f apps/desktop/src-tauri/Cargo.lock`
afterward: **both advisories gone**, only the same 9 pre-existing
informational warnings remain (unmaintained `unic-*`/`paste`/
`proc-macro-error`, unsound `anyhow`/`glib` — all previously known, all
Linux-GTK-webview-only or non-exploitable-in-this-app per existing policy).
Verified the fix doesn't break anything: `pnpm -C apps/desktop build` and
`cargo check` (desktop crate) both succeed cleanly after the bump.

**Result — frontend:** `pnpm -C apps/desktop audit --prod` → "No known
vulnerabilities found."

### 3b. License audit (new)

No license-compatibility check existed anywhere before this bead — two
source comments in the repo said so explicitly (`crates/connector-core/Cargo.toml`
lines 28-33, `.github/workflows/ci.yml`'s connector-mock-production-grep
step) and `find . -iname deny.toml` returned nothing.

**Added:**

- `deny.toml` (root workspace) and `apps/desktop/src-tauri/deny.toml`
  (a separate Cargo workspace, checked separately) — both allowlist exactly
  this bead's specified set (MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib,
  MPL-2.0, Unicode-3.0, CC0-1.0, BSL-1.0, OpenSSL, LGPL-2.1/3.0-or-later,
  GPL-2.0/3.0-or-later, AGPL-3.0(-only)), plus two findings from the first
  real audit run (below), each documented in place with rationale — never
  silently passed.
- `scripts/check-frontend-licenses.mjs` — the frontend equivalent (no
  cargo-deny analogue exists for JS), hand-rolled against
  `pnpm licenses list --json` rather than adding a new devDependency.
- Both wired into `.github/workflows/ci.yml`'s `security-scan` job.
- **A prerequisite fix**: every first-party crate in this repo carries a
  placeholder `license = "LicenseRef-Proprietary"` field (real license TBD
  by the LICENSE-swap bead, `personal-cfo-fkt5.4`) — not a real SPDX
  identifier, and cargo-deny correctly rejected it on the first run. Added
  `publish = false` to every workspace crate (and the standalone desktop
  crate) — true regardless (none of these were ever meant to reach
  crates.io) and it's also exactly what `[licenses.private]` needs to
  correctly skip first-party crates and audit only third-party
  dependencies, which is the actual point of this check.

**Commands and results:**

```sh
cargo deny check licenses                                                    # root: licenses ok
cargo deny --manifest-path apps/desktop/src-tauri/Cargo.toml check licenses  # desktop crate: licenses ok
node scripts/check-frontend-licenses.mjs --prod  # 62 packages, 8 licenses, all compatible
node scripts/check-frontend-licenses.mjs         # 340 packages (incl. dev), 14 licenses, all compatible
```

**Two license findings from the first real audit run — not on this bead's
original list, added with rationale (not silently passed):**

- `CDLA-Permissive-2.0` (Rust: `webpki-roots`, transitive via `ureq` /
  SimpleFIN adapter) — Community Data License Agreement, Permissive 2.0.
  Governs the bundled Mozilla root-CA certificate *data*, not code. Read
  the actual license text: §1.1 permits free use/modification/sharing,
  §2.1's only condition is including the agreement text alongside shared
  *data*, §3.1 explicitly imposes no restriction on "Results" of using the
  data. No copyleft, no conflict with AGPL-3.0-only. Same category as the
  already-allowed CC0-1.0/Unicode-3.0 — permissive licenses attached to
  bundled data, not copyleft code licenses.
- `Apache-2.0 WITH LLVM-exception` (Rust desktop crate: `target-lexicon`,
  via the GTK/webkit2gtk build-dependency chain cargo-deny checks across
  the full dependency graph, even though this app ships macOS-only) —
  Apache-2.0 plus the LLVM exception, which *removes* a restriction
  (permits static linking without the exception's own extra notice
  obligation). Strictly more permissive than bare Apache-2.0, already
  allowed.
- Frontend: `OFL-1.1` (the bundled `@fontsource-variable` IBM Plex Sans /
  Nunito font files, ADR 0063 — **ships in production**), `CC-BY-4.0`
  (`caniuse-lite`'s bundled browser-support data), `BlueOak-1.0.0`
  (`minimatch`), `MIT-0` (`@csstools/color-helpers`), `Python-2.0`
  (`argparse`) — the last four are devDependency-only (build tooling never
  shipped). All permissive, all compatible, each documented in
  `scripts/check-frontend-licenses.mjs`'s `ALLOW` set with the exact
  package and reasoning.

**No incompatible license was found.** Nothing triggered this bead's own
escalation rule for licenses.

**SBOM generation** (`cargo cyclonedx`, named as optional in this bead's
description): **skipped.** Not needed for this internal review; revisit if
external distribution requirements ever call for one.

**Follow-up bead:** none for licensing. The `quick-xml` fix is recorded
above, already applied.

## 4. Threat model currency (closes `personal-cfo-zaq6` as internal)

**What was checked:** `docs/security/threat-model.md` against what has
actually shipped since it was written.

**Command/evidence:**

```sh
git log --follow -- docs/security/threat-model.md
grep -ni "simplefin\|opener\|updater\|867.1.2" docs/security/threat-model.md
```

**Result:** the document's substantive content was authored 2026-06-24
(`personal-cfo-agh`, PR #118) and has had only one touch since — a
find/replace rename on 2026-09-06, not a content update. The grep for
SimpleFIN/opener/updater/867.1.2 returns **zero matches**. Three real
surfaces have shipped since 2026-06-24 and are not reflected:

1. The SimpleFIN bank-connector (ADR 0060) — a new network-egress surface;
   the document's TB3 currently claims "no network egress by design."
2. The `opener:allow-open-url` grant (ADR 0010 addendum 2026-09-06) — a new
   outbound-URL capability.
3. The real signed-artifact auto-updater (`personal-cfo-867.1.2`, ADR
   0068) — a new network-fetch + code-update surface, arguably the
   highest-value gap to close (supply-chain risk: a compromised signing key
   or release feed).

Per this bead's own scope ("no external reviewer... list gaps as follow-up
beads" — rewriting the threat model is explicitly out of scope here), this
is not rewritten in this PR.

**Also found and fixed in passing:** `personal-cfo-6rp7` ("Doc:
docs/security/threat-model.md"), a P2 bead authored 2026-05-03 asking
someone to *write* this document, was still open even though the document
has existed since 2026-06-24 under a different bead (`agh`). Closed as a
duplicate during this review's foundation-first graph reconciliation.

**Follow-up bead:** `personal-cfo-7ie.8` — add the three missing surfaces to
the threats/mitigations table, revise TB3's network-egress claim, assess
the updater's supply-chain risk specifically.

## 5. Vault + encryption design (closes `personal-cfo-a4ih` as internal)

**What was checked:** the shipped Argon2id profile, envelope version, and
SQLCipher/SQLite pins against `docs/architecture/stack.md` and ADR 0002.

**Command/evidence:**

```sh
cargo test -p finance-kernel --test cross_version_vault
```

(Note: this bead's own description names `cargo test --test cross_version_vault`
run from `apps/desktop/src-tauri` — that does not resolve; the test lives
at `crates/finance-kernel/tests/cross_version_vault.rs`, a root-workspace
member. The invocation above is the corrected one.)

**Result:** 2/2 tests pass —
`linked_engine_versions_match_the_pin` (asserts the linked SQLCipher/SQLite
match the exact pins `docs/architecture/stack.md` documents: SQLCipher
`4.5.7 community`, SQLite `3.45.3`) and
`golden_vault_round_trips_on_the_pinned_engine` (a real vault built via
production Finance-Kernel commands, closed, reopened via the production
unlock path, account count and a transaction-display checksum unchanged).

Argon2id profiles (`crates/vault-crypto/src/lib.rs` lines 74-117): three
`Profile` variants —
`InteractiveDefault` (default): 64 MiB / t=3 / p=1;
`HighSecurity`: 256 MiB / t=4 / p=1;
`LegacyCompatibility`: 19 MiB / t=2 / p=1 (OWASP floor, open-only, rekeys
forward). Matches `docs/security/threat-model.md`'s own stated "64 MiB /
t=3 default... 256 MiB profile exists."

**Residual risk (real, worth recording):** ADR 0002 line 123 says Argon2id
parameters "are recorded in `docs/security/encryption-design.md`" — that
file does not exist anywhere in the repo. Not a missing *decision* (the
design is real, correct, and tested per above) — a missing *write-up* ADR
0002 promised. Confirmed by `find . -iname encryption-design.md` (no
results).

**Follow-up bead:** `personal-cfo-7ie.9` — write
`docs/security/encryption-design.md`, citing `crates/vault-crypto/src/lib.rs`
and the `cross_version_vault` test as the source of truth so the doc can't
silently drift from the code again.

## 6. Release signing tested end-to-end (closes `personal-cfo-6wnw`)

**What was checked:** pointer to existing evidence, per this bead's own
instruction not to duplicate the tests.

**Result:** `personal-cfo-867.1.2` (Tauri v2 updater: owner minisign
keypair, plugin wiring, `latest.json` on GitHub Releases, public
smoke-repo round-trip + tampered-signature refusal) is closed and merged
(PR #405, `c1252248`, 2026-09-08). Its close reason records: a live
`0.9.0 -> 0.9.1` round trip via `dohflow/updater-smoke` passed (signature
verified, auto-relaunch confirmed by the owner); both negative tests
(mismatched signature, tampered artifact) passed with exact refusal text
recorded on that bead; `release.sh` refuses to run without signing
environment variables set; the capability-drift tests (`acl_coverage.rs`,
section 1 above) cover the new grants; ADR 0068 is merged.

`personal-cfo-867.1.3`'s own Gatekeeper first-launch smoke test (a
separate, not-yet-done step — second-Mac/VM notarization check) is tracked
on that bead directly, not duplicated here.

**Residual risk:** none beyond what `867.1.3` already tracks.

**Follow-up bead:** none — already tracked by `867.1.3`.

## 7. Connector relay (closes `personal-cfo-n9uc` as NOT APPLICABLE)

**What was checked:** whether a connector relay/proxy service ships or is
planned.

**Result:** no relay ships. ADR 0060 ("Connector strategy: SimpleFIN
first") establishes the SimpleFIN integration as entirely client-side — the
user pastes a SimpleFIN setup token directly into the app; DohFlow holds no
project-owned provider secrets and runs no server-side proxy. README.md's
own architecture section states this directly: "Connectors hold no
project-owned secrets — SimpleFIN's user-token flow runs entirely
client-side (ADR 0004/0060)."

**Follow-up bead:** none — not applicable by design.

## 8. Public-exposure sweep (new)

**What was checked:** every tracked file under `docs/`, `scripts/`,
`.github/`, plus `README.md`, `SECURITY.md`, `CONTRIBUTING.md`, for
internal-only content — email addresses other than project addresses,
password-manager product names, private hostnames, real account or
institution names tied to the owner.

**Commands/evidence:**

```sh
git grep -nEoI '[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}' -- docs/ scripts/ .github/ README.md SECURITY.md CONTRIBUTING.md
git grep -niE "<password-manager product name patterns>" -- <same scope>
git grep -nE "(192\.168\.|10\.[0-9]+\.[0-9]+\.[0-9]+|172\.(1[6-9]|2[0-9]|3[01])\.)" -- <same scope>
git grep -niE "chase\.com|wellsfargo|bankofamerica|citibank|schwab\.com|fidelity\.com|vanguard\.com" -- <same scope>
```

**Findings:**

- **`docs/agent/HANDOFF.md:109`** — a real, non-project (corporate) email
  address, disclosed as an operational trap ("wrangler on this Mac targets
  the WRONG Cloudflare account"). Also carried a password-manager product
  name (line 86); independently, the owner's real GitHub handle appears in
  the sibling `dohflow-site` repo's `workers/cms-auth/README.md`. Fixed by
  `personal-cfo-7d6k9`: the trap is now generic deploy-account guidance with
  no account, employer, or brand named; the `dohflow-site` handle is tracked
  separately for a post-go-live PR (same bead's Phase 3 note).
- **`docs/adr/0062-public-repo-fork-mechanics.md:104`** — quoted one of
  three real historical git-commit author identities (a personal-device
  hostname) in the context of that ADR's own §2 decision ("keep the Gmail
  author, do not rewrite it") — a deliberate, already-accepted disclosure
  documenting *why* that decision was made, not a new leak. Originally
  assessed low severity (the same pattern present in essentially every
  public GitHub repo with local commits) with no action taken; superseded
  2026-09-14 by `personal-cfo-7d6k9`, which replaced the quoted identity
  with a generic description while keeping the decision and its reasoning
  intact.
- **Two stale factual claims**, now fixed: `README.md` and `SECURITY.md`
  both said "No auto-update channel yet; the in-app updater is a
  from-source dev-machine updater" — stale since `867.1.2` shipped a real
  signed-artifact updater (section 6 above). `SECURITY.md` also cited
  `personal-cfo-6rp7` as the still-open threat-model-authoring bead — see
  section 4 above (closed as a duplicate; `SECURITY.md` now describes the
  document's actual state instead of naming a bead).
- Swept `README.md`, `SECURITY.md`, `CONTRIBUTING.md`, `.github/PULL_REQUEST_TEMPLATE.md`,
  and `.github/workflows/ci.yml` directly (read in full, not just grepped)
  — no other exposure found. `Keychain Access` references in
  `docs/operations/release-signing.md` are the generic macOS system app,
  not a private credential store name.

**Fix — the actual go-live mechanism had no exclusion for the ADR
0062 §6 disposition.** ADR 0062 §6 (Accepted 2026-09-07) already decided
to remove six `docs/agent/` files from the public tree:
`HANDOFF.md`, `HANDOFF-2026-08-01.md`, `HANDOFF-2026-08-02.md`,
`SESSION_KICKOFF.md`, `PLAN_KICKOFF.md`, `BEAD_REVIEW_KICKOFF.md`. But the
actual snapshot procedure (`docs/operations/public-launch-snapshot.md`
step 3) is `git archive <sha> | tar -x` — a straight archive of the
**entire tracked tree**, with **no exclusion step**. As written, go-live
would have shipped all six files despite the ADR's decision. Fixed by
extending `.gitattributes` with `export-ignore` for exactly these six
paths — the same mechanism already used in this repo for exactly this
purpose (`scripts/restore-beads-local-files.sh`, `personal-cfo-apesm`).
Verified directly:

```sh
git archive --worktree-attributes HEAD | tar -t | grep -iE "HANDOFF|SESSION_KICKOFF|PLAN_KICKOFF|BEAD_REVIEW_KICKOFF"
# (empty — confirmed absent from the archive)
git archive --worktree-attributes HEAD | tar -t | grep "docs/agent/"
# docs/agent/FRONTEND.md, PROJECT_PROFILE.md, TAURI_RUST_REACT_FINANCE_APPENDIX.md,
# WORKFLOW_ROLES.md, demo-vault.md — the ADR 0062 §6 "keep" list, present
git ls-files --error-unmatch docs/agent/HANDOFF.md   # still tracked — nothing deleted from the private repo
```

Also added a matching absence check to `public-launch-snapshot.md`'s
step-4 verification block (same shape as the existing `.beads/` check), so
a future snapshot fails loudly rather than silently if these ever reappear.

**Follow-up bead:** none — fixed directly, verified, done.

## 9. Tauri release-config audit (new)

**What was checked:** `tauri.conf.json`'s CSP, devtools status, remote-URL
policy, and the updater endpoint pin.

**Result:**

- **CSP**: production `csp` is
  `default-src 'self'; script-src 'self'; ...; connect-src 'self' ipc: http://ipc.localhost; ...; object-src 'none'; frame-src 'none'; ...; form-action 'none'` —
  no remote origin permitted anywhere. `devCsp` additionally allows
  `'unsafe-eval'` and the local Vite dev server, dev-only. Already
  CI-guarded (`ci.yml`'s CSP-configured check, section 1's
  `the_opener_grant_did_not_loosen_the_csp` test).
- **No remote URL loads**: confirmed by the CSP itself and by section 1's
  opener-scope tests (outbound URLs limited to `https://dohflow.app/*`).
- **Updater endpoint**: pinned to exactly
  `https://github.com/dohflow/dohflow/releases/latest/download/latest.json`,
  with the minisign pubkey pinned — both asserted by
  `the_updater_pubkey_and_endpoint_are_exactly_the_committed_values`
  (section 1).
- **Devtools**: off in the current build, but only *incidentally* —
  `apps/desktop/src-tauri/Cargo.toml`'s `tauri` dependency declares an
  empty `features = []`, which happens to exclude the `devtools` Cargo
  feature. Nothing tests this stays true. The crate's own header comment
  says devtools-off-in-release is owned by `personal-cfo-rhci`, which is
  still open.

**Residual risk:** devtools-off is real today but unverified by any gate —
a future dependency bump or an unrelated feature-flag change could
silently re-enable it with no CI signal.

**Follow-up bead:** `personal-cfo-rhci.1` — a CI-enforced check that the
`devtools` Cargo feature is never enabled for a release build.

## 10. Snapshot-tree scans (personal-cfo) — item 10, snapshot approach per
the 2026-09-08 planning amendment

**What was checked:** full-history gitleaks (informational — the private
repo's history never becomes public, only the tree at the snapshot SHA
does) and the new value-scan script (real financial figures, bank/broker
patterns) over the current tracked tree.

**Commands/results:**

```sh
gitleaks detect --config .gitleaks.toml --no-banner --redact
# 907 commits scanned, ~132.98 MB, no leaks found

./scripts/value-scan.sh
# value-scan: clean — no real institution names, SSN-shaped, or
# card-number-shaped strings in the tracked tree.
```

The value-scan script (new, `scripts/value-scan.sh`) is what
`public-launch-snapshot.md` step 4 refers to as "the o1nxk value scan" —
that doc previously said "see that bead for the exact scan command," which
this now resolves to a real, runnable script rather than bead prose. It
checks a denylist of real US bank/brokerage names (deliberately excluding
the demo vault's own invented names — Saltmarsh CU, Kestrel, Copperleaf,
etc.), SSN-shaped strings, and full-card-number-shaped strings.

**Round-2 review correction (recorded here, not silently fixed).** The
first version of this section reported "clean" based on a run that
predated the script's own commit — `git grep` never sees untracked files,
so the self-match this created (the denylist names its own patterns; this
document quotes them) was invisible until the commit that made it
visible, at which point the script failed on the very tree it ships in.
Caught in independent review, not by the implementer. Three real defects
were found and fixed, each verified with a new hermetic test
(`scripts/tests/value-scan.test.sh`, 10 cases):

1. **Self-match** — the script and this document now exclude themselves
   from their own scan (`excluded_paths` in the script), without being
   exempted from actually *shipping* (both remain in every `git archive`
   output — verified directly).
2. **The SSN/card regexes never worked at all**, in any version, including
   the one this review originally reported clean. `git grep -E` does not
   support `\b` word boundaries — it compiles the pattern without error
   but silently never matches, rather than failing loudly. Fixed with a
   portable POSIX-ERE boundary (`(^|[^0-9-])...([^0-9-]|$)`) that also
   rejects a hyphen neighbor specifically — needed because this codebase's
   real UUID fixture IDs (e.g. `0190a000-0000-7000-8000-000000000001`)
   contain their own internal `-NNNN-NNNN-NNNN-` run, which the
   word-boundary-only form would have false-positived on as a truncated
   card number.
3. Fixing (2) made the checks *actually run* for the first time, which
   surfaced real false positives: `Cargo.lock` (both workspaces' lockfiles; SHA256
   checksums, 64 hex characters, long enough to contain a 16-digit run by
   chance — now excluded, `*.lock` carries no narrative privacy risk) and
   the redaction layer's own test corpus, which deliberately embeds fake
   card-number-shaped strings (`crates/observability/src/lib.rs`,
   `crates/finance-kernel/tests/log_redaction.rs` — the exact same files
   `.gitleaks.toml` already allowlists for the identical reason,
   `personal-cfo-zobt`; plus `crates/finance-kernel/tests/side_file_leak.rs`,
   which gitleaks' own credential-pattern rules don't happen to trigger on
   but this scan's broader digit-shape heuristic does). All three added as
   a documented, narrowly-scoped exception for the SSN/card checks only.
4. **The round-2 fix for (1) recurred one level up**, caught by a second
   review round: the new hermetic test file added alongside the fix
   (`scripts/tests/value-scan.test.sh`) itself plants the same denylist
   words and exception strings as fixture content, so once *it* was
   committed it self-matched too — the identical failure mode, at a
   different file, because the first fix excluded the script and this
   document but not the newest self-referential file. Fixed three ways:
   the test file is now also excluded from the scan's own pathspec; the
   test's own fixture-building helper now copies the test file itself
   into every throwaway repo (alongside the script and the doc excerpt
   already copied there), so its own case 1 is a complete regression test
   for all three files, not two; and — the structural fix for the
   *recurring* root cause, not just this specific instance of it —
   `./scripts/value-scan.sh` is now wired into `.github/workflows/ci.yml`'s
   `security-scan` job, so this class of defect gets caught by CI against
   the actual committed tree, rather than depending on a human (implementer
   or reviewer) remembering to re-run it by hand at the right moment.

The `CapitalOne` institution-name exception from the original version is
unchanged: `crates/importers/csv-importer/src/lib.rs`,
`crates/db-worker/src/migrations.rs`,
`crates/importers/importer-core/src/lib.rs`, and
`docs/adr/0045-ingestion-field-capture-and-dual-date.md` — describing a
real bank's CSV **export format** for import compatibility (ADR 0045),
always with entirely synthetic fixture data. Every exception (this one and
the three above) was verified by reading the actual code before excepting
it, and `scripts/tests/value-scan.test.sh` proves each is scoped to
exactly its named files — a synthetic leak or fixture-shaped string
planted in an unlisted file is still caught.

**Result after the fix, cited to a specific, checkable commit** — the round-2
review's own required remedy, precisely because the first two "clean"
claims in this section were each recorded from a working tree that did not
yet match what got pushed: at commit `dad8c6f2f987298d072897af9e95a4dccf236a55`
(`git rev-parse HEAD`, working tree clean per `git status --short` at the
time), `./scripts/value-scan.sh` → clean, `exit 0`. This commit already
exists and is checkable directly; this sentence recording it is necessarily
a later commit, since a commit cannot cite its own final SHA — the
verifiable claim is about the tree the script actually ran against, not
about this document's own eventual hash.

**Known limitation:** this is a text/grep scan and cannot inspect binary
file content (the `apps/desktop/screenshots/raw/*.png` masters from
`personal-cfo-n76x.13`). Those were separately, manually inspected
pixel-by-pixel before commit — see that bead and
`dohflow-site/docs/screenshots.md`'s "Privacy check" section. Not
re-verified here.

**Follow-up bead:** none.

## 11. `dohflow-site` full-history scan (owner decision F, 2026-09-08)

**What was checked:** a full-history gitleaks scan of the sibling
`dohflow-site` repository (reusing this repo's `.gitleaks.toml`, since
`dohflow-site` has none of its own), plus a direct read of
`workers/cms-auth` — the site's one server-side component, a GitHub OAuth
proxy Worker for Sveltia CMS holding a real client secret.

**Command/result:**

```sh
gitleaks detect --config <personal-cfo>/.gitleaks.toml --no-banner --report-format json ...
# 44 commits scanned, ~2.56 MB, 1 finding
```

**The one finding, verified as a false positive.** `generic-api-key` rule
matched `public/admin/sveltia-cms.mjs:2026` (the vendored Sveltia CMS
bundle), specifically the substring `r=i.focus.key`. Read the surrounding
minified code directly: `i._selection`, `._nodeMap`, `.anchor.key`,
`.focus.key` — this is Slate.js rich-text-editor internals (`.anchor`/
`.focus` are selection endpoints, `.key` is a document-node identifier),
not a credential of any kind. Zero real leaks.

**`workers/cms-auth` review:** read `src/index.js` (281 lines) directly.
The GitHub OAuth client secret is read from an environment variable, never
logged, never returned to the browser, never embedded in client-served
code. CSRF protection via a `__Host-` cookie carrying the OAuth `state`.
`postMessage` targets an exact origin, never `*`. `ALLOWED_DOMAINS` is
required and fails closed if unset. No hardcoded credential or account
identifier found (`test/xss.test.mjs` exercises the injection-prevention
paths — 65 lines, already run in that repo's own CI).

**Residual risk (real, worth recording):** `dohflow-site` has **zero
standing security scanning** — no gitleaks step, no dependency-audit step,
anywhere in its CI, and no `.gitleaks.toml` of its own. This one-time scan
(what this section was actually asked to do) is necessary but not
sufficient — every commit to that repo after this review goes unscanned
until standing coverage exists.

**Follow-up bead:** `personal-cfo-n76x.25` — add a `security-scan` CI job
to `dohflow-site` (gitleaks with its own `.gitleaks.toml`, allowlisting
the vendored-bundle false positive above by fingerprint; a dependency
audit), matching this repo's own job's shape.

---

## Summary

| Item | Bead | Disposition |
|---|---|---|
| 1. IPC surface | `o4il` | Closed — existing `acl_coverage.rs` (10/10) is thorough |
| 2. Log redaction | `sq0p` | Closed — `zobt` gate exists and passes; owner log-sample step pending |
| 3. Dependency + license audit | `xiwn` | Closed — cargo-deny + frontend script added; 2 HIGH vulnerabilities found and fixed (owner-approved) |
| 4. Threat model currency | `zaq6` (internal) | Closed — real gaps found, not rewritten here; follow-up `7ie.8` |
| 5. Vault + encryption design | `a4ih` (internal) | Closed — design confirmed correct and tested; missing doc filed as `7ie.9` |
| 6. Release signing | `6wnw` | Closed — `867.1.2` evidence cited |
| 7. Connector relay | `n9uc` | Closed — not applicable (no relay ships) |
| 8. Public-exposure sweep | new | Fixed directly (`.gitattributes`, two stale doc claims, one bead-graph cleanup) |
| 9. Tauri release-config audit | new | CSP/remote-URL/updater confirmed solid; devtools gap filed as `rhci.1` |
| 10. Snapshot-tree scans (personal-cfo) | new | Clean (gitleaks + `value-scan.sh`), verified at a specific cited commit — round 2 caught `value-scan.sh` self-matching and a non-functional boundary regex; round 3 caught the same self-match recurring at the fix's own new test file. All fixed; `value-scan.sh` now also runs in CI against the committed tree, not just locally |
| 11. `dohflow-site` full-history scan | owner decision F | Clean (1 confirmed false positive); standing CI gap filed as `n76x.25` |

**Deferred, not evaluated here:** `personal-cfo-5kua`/`-mh83` (external
security review / pentest) — owner decision 2026-09-07, deferred until
revenue justifies it. `SECURITY.md` and the site's `/security` page
already disclose no third-party audit has happened.

**New follow-up beads filed:** `personal-cfo-7ie.8` (threat model
currency), `personal-cfo-7ie.9` (missing encryption-design.md),
`personal-cfo-rhci.1` (devtools-off-in-release has no test),
`personal-cfo-n76x.25` (dohflow-site standing security-scan CI).

**Bead-graph cleanup found and done in passing:** `personal-cfo-6rp7`
closed as a duplicate (see section 4).

---

## Owner sign-off

_Reviewed and accepted for the v0.1.0 pre-release gate:_

- Owner: ______________________
- Date: ______________________
- Notes (if any): ______________________
