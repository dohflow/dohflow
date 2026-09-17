# ADR 0071 — Public-repo contribution model

- **Status:** Accepted (2026-09-17)
- **Tier:** Public — process/security posture for a public repository, no
  business content.
- **Bead:** `personal-cfo-du1e9`
- **Decider:** Owner, 2026-09-17, in chat with the implementation session
- **Related:** ADR 0064 (task tracking leaves the repo — beads stay private
  regardless of tracker choice, restated here as D8's premise), ADR 0068
  (release distribution — the local `scripts/release.sh` process this ADR's
  future-CI policy will eventually replace), ADR 0082 (public-disclosure
  boundary — the review/merge evidence rule this ADR's no-self-merge section
  extends), `personal-cfo-867.1.4` (the not-yet-built CI release workflow
  this ADR constrains in advance), `personal-cfo-56s6` (supply-chain risk)

## Context

Program plan v0.8.1 §6.3 (D8, R7/R10) asks for the contribution model to be
recorded once development moves to the public `dohflow/dohflow` (`r36ck`,
closed). Several load-bearing facts were only ever owner decisions in bead
notes or, worse, unverified assumptions — this ADR records what is actually
configured today, verified live via the GitHub API on 2026-09-17, rather
than what AGENTS.md's prose implies.

## Decision

### 1. No-self-merge is procedural, not technically enforced

The `main` branch ruleset (id `23361528`, verified live 2026-09-17) requires
**zero** approving reviews and grants the Admin repository role an
**always** bypass:

```json
{
  "rules": [
    {"type": "deletion"},
    {"type": "non_fast_forward"},
    {"type": "required_linear_history"},
    {"type": "pull_request", "parameters": {
      "required_approving_review_count": 0,
      "required_reviewers": [],
      "allowed_merge_methods": ["merge", "squash", "rebase"]
    }},
    {"type": "required_status_checks", "parameters": {
      "required_status_checks": [
        {"context": "IPC codegen (apps/desktop/src-tauri)"},
        {"context": "Rust (workspace)"},
        {"context": "Security scanning (secrets + dependencies)"},
        {"context": "Shell scripts"},
        {"context": "Frontend (apps/desktop)"}
      ]
    }}
  ],
  "bypass_actors": [{"actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "always"}],
  "current_user_can_bypass": "always"
}
```

So AGENTS.md §16's "no self-merge" and "review binds to a SHA" rules are
enforced by **process discipline alone** — nothing in GitHub configuration
stops the repository owner (or any future second Admin) from merging
without review. The artifact every merge cites in practice is the
`04-review` session's structured PR comment carrying an explicit
`PASS` / `APPROVED_TO_MERGE` verdict bound to the exact SHA merged (see
`personal-cfo-he3xo` PR #2 for the pattern this ADR formalizes: review
comment linked, owner performs the merge, bead closes citing the merge
commit). This is unchanged by this ADR — it is recorded, not altered, per
ADR 0082's "both are accepted, recorded limitations" precedent for
single-account review.

Branch deletion and non-fast-forward pushes to `main` are blocked by the
same ruleset regardless of bypass status (`deletion` and
`non_fast_forward` rule types have no bypass-relevant parameters — they
apply to the ref, not the actor).

### 2. Fork-PR secret isolation

GitHub's platform default already isolates `pull_request`-triggered runs
from forks: they receive no repository secrets, full stop. `ci.yml` (the
only workflow in the repository today) uses zero `secrets.*` references —
verified by grep — so there is nothing to isolate yet. This decision exists
to bind the **next** workflow that needs secrets, not to change anything
live:

- `personal-cfo-867.1.4` (the CI release workflow, not yet built) **must**
  trigger only on `push: tags` or `workflow_dispatch` — never `pull_request`
  — for any job that touches Apple signing or updater secrets.
- Those secrets (`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
  `APPLE_SIGNING_IDENTITY`, `APPLE_API_ISSUER`/`KEY`/`KEY_PATH`,
  `TAURI_SIGNING_PRIVATE_KEY`(`_PASSWORD`) — the set already named in
  `867.1.4`) live in a GitHub Environment named **`release`**, configured
  with a required reviewer, when that workflow is built. No Environment
  exists in the repository today (`GET /repos/dohflow/dohflow/environments`
  returned zero, verified live 2026-09-17) — this is a constraint on future
  work, not a description of current state.
- This ADR does **not** move the `.p12` or any signing material off the
  owner's Mac. `scripts/release.sh` (ADR 0068) remains the release process
  until `867.1.4` ships; that migration is `867.1.4`'s own decision, gated
  on "only once releases are frequent enough to justify moving signing
  material off the owner's Mac" per its existing bead notes.

### 3. GitHub Issues is the public tracker (D8)

The bead graph stays private regardless of tracker choice (ADR 0064) — a
public repository still needs a public place for outside reports and
requests. Decision: **GitHub Issues**, effective immediately.

- An outside issue that is accepted (a real bug, a reasonable feature ask)
  gets a bead created for it, with the issue URL recorded on the bead.
- When the fixing PR merges, an agent or the owner comments on the issue
  linking the merged PR, and the bead closes citing the same commit —
  matching the existing "merge closes the bead" rule (AGENTS.md §16 rule 4).
- `CONTRIBUTING.md` already tells outside contributors to open an issue
  before sending a PR and that bead IDs are internal and meaningless to
  them — this decision formalizes what that document already assumed.

### 4. Issue triage policy

**Who:** the owner. No dedicated triage role or second maintainer exists
today.

**Response window:** acknowledgement within **7 days** — the same modest,
deliberately-keepable commitment `SECURITY.md` already makes for
vulnerability reports, for the same reason: one unfunded maintainer, and a
promise that can't be kept is worse than a modest one that can.

**Out of scope:** an issue judged out of scope is closed with a comment
naming the specific reason. Once they exist, closures point to
`docs/product/non-goals.md` (`personal-cfo-39a5`, not yet written) and the
public roadmap (`personal-cfo-sygn`, not yet written) for the general
policy; until then, the closing comment states the reason directly rather
than pointing at a document that doesn't exist yet.

## Consequences

- **Positive.** The actual enforcement posture (procedural, not technical)
  is now written down instead of implied — anyone reading AGENTS.md §16 sees
  the honest picture, not an assumption that a ruleset backs it.
- **Positive.** `867.1.4` inherits a settled trigger/secrets-custody policy
  before it starts, per the foundation-first discipline (AGENTS.md §1A) —
  it does not have to make this decision mid-implementation.
- **Positive.** Outside contributors get a real answer to "where do I file
  a bug" the moment this merges, without waiting on the roadmap/non-goals
  docs.
- **Negative / accepted.** The 7-day response window is a target, not a
  guarantee, identical in spirit to `SECURITY.md`'s own framing — silence
  past the window is a failure to flag, not a policy.
- **Negative / accepted.** Out-of-scope closures cite ad hoc reasons until
  `39a5` and `sygn` exist. Not a regression — those docs don't exist for
  anyone today.
