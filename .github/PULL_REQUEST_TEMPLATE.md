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
> Hosted CI is `workflow_dispatch`-only by deliberate decision, so there is no
> automatic status check on this PR. Two independent local gate runs — the
> implementation agent's above, and the reviewer's in its comment — are the
> evidence.
>
> Automatic reviewer merging is disabled during calibration. A pass returns
> `APPROVED_TO_MERGE`; the repository owner authorizes or performs the merge.
