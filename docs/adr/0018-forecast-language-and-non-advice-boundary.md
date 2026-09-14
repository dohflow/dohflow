# ADR 0018: Forecast language and non-advice boundary

- **Status:** Accepted
- **Date:** 2026-06-23
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-9xq`](../../.beads/issues.jsonl)
- **Related plan sections:** §13.7.1, §21.2, §23, §26
- **Supersedes:** None

## Context

Personal CFO computes forecasts, risk flags, and (later) agent narratives over a
household's own financial data. It is **not** a licensed financial advisor and
does not want to become one — crossing from *calculation* into *advice* ("you
should pay off this card", "transfer money to savings", "you can afford this")
creates both **regulatory exposure** (advice is a regulated activity in many
jurisdictions) and **trust/UX risk** (a wrong "you should" is far more damaging
than a wrong projection, and erodes the local-first, user-in-control posture of
ADR 0002/0003).

The forecast already ships descriptive copy today, and the copy was audited clean
of prescriptive language when this ADR was written. But "descriptive, not
advisory" is currently a convention in people's heads, not an enforced boundary.
The Future Cash surfaces are expanding (scenarios, per-row explanation, the
upcoming readiness/insights work), and agent reports are on the roadmap — exactly
the surfaces most tempted to say "you should". This ADR records the boundary and
makes it **enforceable** so the next copy change can't quietly cross it.

This pairs with the forecast non-jargon copy work (`personal-cfo-w5dn`,
percentile labels) and the deterministic forecast architecture (ADR 0026).

## Decision

### 1. Forecast/dashboard/agent surfaces are descriptive, never prescriptive

User-facing copy on the forecast, dashboard, risk-flag, and (future) agent-report
surfaces **describes the user's situation and the calculation behind it**. It does
**not** tell the user what to do.

- **Allowed (descriptive):** "Based on current assumptions, your liquid cash
  reaches its low point of $X on June 30." · "At your current spend rate, you
  cross your $500 floor on July 12." · "This projection estimates a $1,200 surplus
  at day 90."
- **Forbidden (prescriptive):** "You should pay off this card." · "Transfer money
  to savings." · "You can afford this purchase." · "We recommend cutting
  spending."

The framing leads with the assumption basis ("Based on current assumptions…",
"This projection estimates…"), reinforcing that the number is a calculation over
inputs the user controls, not a verdict.

### 2. Risk flags describe the risk, not the remedy

A risk flag states **what** the projection shows and **why** ("Your balance is
projected to go negative on July 12 because rent and the card payment land before
your next paycheck"). It does **not** prescribe the fix ("so you should move the
payment"). Surfacing the risk is the product's job; choosing the response is the
user's.

### 3. An enforced copy-review check

A copy-review check (`apps/desktop/src/copy-review.test.ts`, part of the
`pnpm test` gate) scans the forecast and dashboard UI source for prescriptive
language and **fails** when it finds any. It covers the prescriptive verbs called
out in §21.2 — *should, recommend, suggest, must* — as **advice phrases**
("you should", "we recommend", "should pay", "you can afford", …) rather than bare
words, so it catches advice copy without flagging incidental, non-user-facing uses
(e.g. a code comment that says a value "must match the currency"). The pattern set
and the scanned surfaces live in that file and grow as new surfaces (agent
reports) land.

Because CI is manual-only on this repo (the workflow is dispatch-only — see the
`ci-is-manual-only` operating note), "fails the build" means the check is part of
the local gate suite that must pass before every push; the same `pnpm test` run is
what a manual CI dispatch executes.

### 4. Scope

This boundary governs **product-generated** copy on financial surfaces. It does
not constrain the user's own free-text (labels, notes, scenario names), nor
neutral UI chrome ("Add account", "Save"). Educational/explanatory copy is fine as
long as it explains the calculation rather than directing an action.

## Consequences

### Positive

- The advice/calculation boundary is enforced, not just intended — a future "you
  should…" string fails the gate instead of shipping.
- Lower regulatory surface: the product stays on the calculation side of the line.
- Reinforces the trust posture — the user stays the decision-maker; the app
  informs.

### Negative

- Some genuinely helpful phrasings are off-limits; copy must work a little harder
  to be useful while staying descriptive.
- The check is a heuristic (phrase patterns over source), so it can miss a novel
  advice phrasing or, rarely, flag a false positive; the pattern set is
  maintained, and human copy review remains the primary safeguard.

## Rejected alternatives

- **A legal disclaimer banner only ("not financial advice"), with no language
  rule.** ✗ A disclaimer doesn't change the words on screen; a banner plus
  "you should pay off X" still reads as advice and still carries the risk.
- **No rule — rely on judgment.** ✗ A non-licensed product drifting into advice is
  exactly the regulatory + UX risk this ADR exists to prevent; "in people's heads"
  is not enforceable as the surfaces and contributors grow.
- **Bare-word scan (any "should"/"must").** ✗ Too noisy — flags code comments and
  incidental prose, training people to suppress the check. Advice-phrase patterns
  catch the real cases with far fewer false positives.

## Revisit if

- A legal review establishes a different boundary (e.g. a registered-advisor
  posture), which would change both what is allowed and the disclaimer strategy.
- An agent/narrative surface needs richer language than phrase-scanning can govern,
  warranting a more structured copy-lint (e.g. an ESLint rule over JSX text).

## Test coverage

`apps/desktop/src/copy-review.test.ts` scans the forecast (`src/future-cash`) and
dashboard (`src/dashboard`) UI for the forbidden advice phrases and asserts none
are present. New financial surfaces (notably agent reports) are added to its
scanned-directory list as they ship.

## Linked beads

- `personal-cfo-9xq` (this ADR + the copy-review check)
- `personal-cfo-w5dn` (forecast percentile non-jargon copy — uses this boundary)
- `personal-cfo-5oo6` (safe-to-spend derived metric — must stay descriptive)
- `personal-cfo-rv2s` (pre-release disclaimer legal review)
- `personal-cfo-915` (strategic-decisions epic — parent)

## Addendum 2026-06-30 — Cash comfort-band recommendations: the "what-would-it-take" boundary (`915.1`)

The intelligent cash-flow arc wants to tell the user when their projection drifts out of a
target **liquid-cash comfort band** (e.g. $5,000–$10,000) and why. The user's literal ask
("recommend they cut spending to X for ABC categories") is **prescriptive** and fails this
ADR's boundary, the closed anti-task `gnj5` ("advice is forbidden"), and the copy-review scan.
Decision (project owner, 2026-06-29): such guidance is **descriptive "what-would-it-take"
math** — the system shows the *calculation*, never a directive.

- **The band wraps the floor, it does not replace it.** The shipped `minimum_cash_floor`
  (ADR 0029) becomes the band's **lower edge**; the band adds an **upper edge** (excess the
  user may choose to deploy). Existing floor logic is unchanged; the upper edge is new.
- **Allowed (descriptive math + evidence):**
  - *"Your projected balance crosses the lower band on Aug 14, driven mostly by a rising
    dining baseline."*
  - *"To stay above $5,000 through August, about $420/month of the recent dining increase
    would need to reverse."* (a derived quantity, not a directive)
  - *"You're projected to end August about $3,100 above the band."*
- **Forbidden (prescriptive remedy):**
  - *"Cut dining to $450/month."* / *"You should reduce spending."* /
    *"We recommend paying off Card A first."* / *"Consider moving the excess to savings."*
  The line: stating *what the numbers are and what would have to change* is allowed; telling
  the user *what to do* is not. "would need to" / "is projected to" are descriptive; "cut" /
  "should" / "recommend" / "consider" are directives.
- **`suggested_actions_json` is reconciled.** The planned `risk_flags.suggested_actions_json`
  (whose own example "Set dining target to $450/month" violates this ADR) is **renamed to
  `contributing_factors_json`** and carries only descriptive evidence (the categories /
  assumption events driving the drift, with magnitudes) — no imperative verbs. The same
  applies to any "suggested next action" field in `ut2w`/`mq52`/`5kjg`. A test asserts no
  serialized `risk_flag` contains an advice-phrase pattern (mirroring the copy-review scan).
- **The copy-review scan widens with the surfaces.** `copy-review.test.ts` adds the new
  financial UI source paths as they ship — the Money Inbox (`src/money-inbox`), the Accounts
  shell incl. the debt/investment views (`src/accounts`, per ADR 0037), and the
  forecast-activation/band surfaces — so the non-advice boundary stays enforced wherever
  band/debt/investment copy lands, not just `src/future-cash` + `src/dashboard`.

## Addendum (2026-07-02): the user-steered "cover it" tool (ADR 0040)

The owner's daily workflow ends in an action this ADR would otherwise forbid: when checking
is projected negative, *move money from savings/brokerage to cover the shortfall*. We permit
this as a **descriptive projection + a tool the user drives**, never as unsolicited advice:

- **Descriptive is unchanged:** the app states the shortfall as fact — "checking is projected
  to reach −$X on Jul 17" — with no imperative and no "you should".
- **The tool is user-initiated:** a "Cover it" affordance the user *chooses* to open may then
  propose the amount needed to clear the shortfall and let the user pick which reserve account
  to pull from and confirm the transfer. Proposing an *amount* to reach a user-invoked goal is
  a calculator, not advice; the app never selects the source account or tells the user to act
  unprompted.
- **Boundary that stays firm:** no banner, insight, notification, or forecast row says "move
  $X from Savings" on its own. The prescriptive content only appears *inside* a tool the user
  opened, framed as "to cover this, transfer …", with the user choosing source + confirming.

This keeps the non-advice philosophy (no unsolicited imperative guidance) while letting the
app be genuinely useful for the one place the owner's process is prescriptive. The
`copy-review` scan continues to forbid advice-phrases in passive/always-on copy; the cover-it
tool's proposal strings are exempt as user-invoked calculator output (asserted by a scoped
test, not a blanket allow).

## Addendum (2026-07-02): timing gates the response — cover-it vs. attribution (`3v6d`/`5ie.8`)

A band crossing is not one thing. **How far out the crossing is** determines which affordance
is appropriate, and both stay descriptive:

- **Near-term shortfall** (the crossing is soon enough that a transfer is actionable): the
  user-steered **"cover it" tool** above is the fitting affordance — the user opens it and moves
  money to cover the projected dip. A concrete transfer makes sense.
- **Far-horizon drift** (e.g. months out): a transfer is *not* the right response — the balance
  will move many times before then. Here the value is the **descriptive attribution** (`5ie.8`):
  *why* the projection is drifting out of the band — the contributing categories / assumption
  events with magnitudes (e.g. extraordinary spend reclassified as ordinary, a category baseline
  that rose, "dining +Z%"). The app states the drift and its evidence; the user deduces what to do.
  No transfer prompt is surfaced for a far-out drift, and — per the boundary above — nothing
  recommends an action.

Practical rule: **cover-it is offered only for near-term shortfalls; the far-term crossings are
carried by the descriptive drift attribution.** A single fixed horizon is not assumed here; the
threshold that separates "near" from "far" is an implementation choice (a small, documented
constant), not a new decision — the invariant is that neither path emits imperative guidance.
