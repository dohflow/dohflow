<!-- TAURI UPGRADE PR (personal-cfo-io42, ADR 0001). Use this template ONLY for a
     dedicated Tauri upgrade: the Rust and npm halves of the Tauri family move
     together, and nothing else ships in this PR. Procedure and checklist:
     docs/agent/TAURI_UPGRADES.md. Open with
     `gh pr create --body-file .github/PULL_REQUEST_TEMPLATE/tauri-upgrade.md`
     or `?template=tauri-upgrade.md` on the compare URL. -->

<!-- AGENTS.md §16 · docs/agent/WORKFLOW_ROLES.md
     This body is owned by the IMPLEMENTATION agent.
     The reviewer does not edit it — it replies in a separate structured
     comment and links that comment under "Review" below. -->

## Bead

<!-- Outside contributor? Skip this section — you don't need a bead ID, and
     no CI check requires one. Cite the GitHub issue you're addressing in
     the Summary below instead. This section is for this project's own
     agent/owner sessions, which track work as beads (ADR 0082). -->

- **Bead ID:** personal-cfo- <!-- agent/owner only; never the bead's TITLE — see ADR 0082 decision 5 -->
- **Candidate SHA:**
- **Process level:** full / lightweight / user-authorized exception
- **Owning session:** 02-implementation / 03-escalation (takeover authorized: )

## CLA

Code contributions are accepted under a Contributor License Agreement — an
automated bot prompts for a one-time signature on your first PR, per the
AGPL-3.0-only + CLA licensing decision (ADR 0043, 2026-09-02 addendum,
`CLA.md`). No action needed here beyond following the bot's prompt if it
appears.

## Summary

<!-- What changed and why. Plain language first, then technical detail. -->

## What changed (Tauri family)

<!-- One row per line that changed in apps/desktop/src-tauri/tauri-pins.toml.
     Both halves: Rust tauri*/wry/tao and npm @tauri-apps/*. -->

| Package | From | To | Release notes / changelog reviewed |
|---|---|---|---|
| `tauri` | | | |
| `@tauri-apps/cli` | | | |
| `@tauri-apps/api` | | | |

- **Manifests:** `Cargo.toml` and `apps/desktop/package.json` specs are still `~X.Y.Z` or exact (`tests/tauri_pins.rs` `manifest-range`).
- **Lockstep:** each Rust plugin matches its npm package's minor, and `tauri` = `cli` = `api` minor (`lockstep`).
- **Network step used:** <!-- the exact cargo update / pnpm install commands run -->
- **Code changes the upgrade forced:** <!-- list them, or "none" -->

## Permission expansion diff

<!-- `node scripts/tauri-acl-expansion.mjs` on main vs on this branch, diffed.
     Paste the diff, or write "no change". Any grant that now allows MORE is a
     policy change: link the ADR 0010 addendum that accepts it. -->

```diff
```

## Vendored-source re-verification

<!-- docs/agent/TAURI_UPGRADES.md, checklist items 1–13. For each, confirm it
     still holds in the new sources, or say what changed and how it was handled.
     Anything that no longer holds is fixed here (and reviewed) or blocks the
     upgrade. -->

| # | Claim | Still holds? | Notes |
|---|---|---|---|
| 1 | ACL rejects ungranted commands before dispatch | | |
| 2 | One invoke handler; IPC bridge in every webview | | |
| 3 | Only ACL exception is channel-fetch (app-wide queue) | | |
| 4 | Plugin `on_navigation` can veto; first load unseen | | |
| 5 | Bundled origin per platform | | |
| 6 | `is_dev()` selects devUrl + devCsp | | |
| 7 | CSP rewriting: script hashes; style nonce only for `<style>` | | |
| 8 | `core:default` has no window creation/navigation; inert members still inert | | |
| 9 | `add_capability` still the only runtime-grant route | | |
| 10 | Opener scope matching on the raw URL; no `with` program | | |
| 11 | Updater verifies the signature before install | | |
| 12 | wry new-window behavior; `on_new_window` available | | |
| 13 | `eval_with_callback`, mock runtime, `generate_context!(test = true)` exist | | |

## Acceptance criteria

<!-- Describe how each criterion is satisfied, one line each — but do NOT
     paste the bead's acceptance-criteria text or notes verbatim into this
     PUBLIC PR body if this repo is dohflow/dohflow (ADR 0082, decision 5):
     summarize the outcome in your own words instead. This restriction does
     not apply to a private-repo PR. -->

- [ ]

## Out of scope / preserved behavior

<!-- What deliberately did not change. -->

## Implementation-agent checks

> Filled by the implementation session. Run the subset that applies; one build
> at a time. Definition of Done: `docs/architecture/definition-of-done.md`

| Command | Result |
|---|---|
| `cargo fmt --check` | |
| `cargo clippy --workspace --all-targets -- -D warnings` | |
| `cargo test --workspace` | |
| `pnpm run typecheck` | |
| `pnpm run lint` | |
| `pnpm test` | |
| `pnpm run build` | |

**Tauri-upgrade gates (all required):**

| Command | Result |
|---|---|
| `cargo test` in `apps/desktop/src-tauri` (includes `tauri_pins`, `acl_coverage` + drift audit, `window_isolation`) | |
| Runtime isolation probe, **macOS locally** (`cargo build --features tauri/custom-protocol` + `PCFO_ISOLATION_PROBE`) | |
| CI "Runtime isolation probe (real WebView, ADR 0010)" (Linux) | |
| `node scripts/check-dist-csp.mjs apps/desktop/dist` after `pnpm build` | |
| `pnpm tauri build` + launch smoke + signed-updater check (ADR 0068) | |

**Checks not run, and why:**

## Manual verification / demo

<!-- Exact steps, or screenshots. Demo vault: docs/agent/demo-vault.md -->

## Known limitations and risks

<!-- Regression, migration, data-loss, security, privacy, release.
     State "none identified" explicitly rather than leaving this blank. -->

---

## Review

> **The reviewer does not edit this body.** It posts an independent structured
> comment on this PR containing: the PR number, the SHA reviewed, each
> acceptance criterion assessed individually, its own commands and results,
> checks it could not run and why, findings, the verdict, and confirmation that
> the PR head still matched the reviewed SHA at verdict time.
>
> **Link the review comment here:** <!-- URL -->
>
> CI runs automatically on this PR (this repository is public and is where
> development happens — `personal-cfo-r36ck`) and its five required checks
> are the primary evidence. It does not replace the two independent local
> gate runs below — the implementation agent's own, and the reviewer's in
> its comment — which prove each agent understood what it ran, not just
> that a status check went green.
>
> Automatic reviewer merging is disabled during calibration. A pass returns
> `APPROVED_TO_MERGE`; the repository owner authorizes or performs the merge.
