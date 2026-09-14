# ADR 0040: MLP-first roadmap — replace the spreadsheet before we ship

- **Status:** Accepted
- **Date:** 2026-07-02
- **Deciders:** Project owner
- **Beads:** [`personal-cfo-j0cg`](../../.beads/issues.jsonl) (Gate: MLP), [`personal-cfo-2owr`](../../.beads/issues.jsonl) (Gate: Launch)
- **Related:** ADR 0018 (non-advice — amended, see below), ADR 0031 (UI-quality/design loop), ADR 0036 (scenario overlays), ADR 0039 (card cycle/statement), `personal-cfo-867.1` (release packaging)
- **Supersedes:** the Week-8 first-playable gate (`personal-cfo-rtez`, met + closed); and this ADR's own first draft, which ordered distribution *before* product-completeness — that ordering was wrong.

## Context

A foundation-sweep re-evaluation (2026-07-02) established that the **feature engine is remarkably complete** and works end-to-end (vault + backup/restore, accounts + additive balances, transactions, income, bills, transfers, the Future Cash running-balance forecast + per-account + scenarios/overlays + Layer-2 band + readiness + actualization seam, CSV import + Money Inbox, the debt/credit arc incl. card statement forecast + payoff, recurring detection, in-app updater; safety gate passed).

The project owner then clarified the goal, and it reframes the whole roadmap: the target is not "MVP" but **MLP — a Minimum _Lovable_ Product**. Concretely: **the owner can completely replace his family-finance spreadsheet workflow every day, in an app whose UI feels polished/lovable.** Distribution and releasability are **not** the first gate — they come **after** MLP. There is no point signing, packaging, and sharing an app that doesn't yet fully replace the spreadsheet or feel good to use.

The spreadsheet workflow being replaced (from the owner's real `Family Finances.xlsx`) is a per-paycheck + daily loop: manually update ~8 bank + ~15 credit-card balances; read a running checking+savings+net Future-Cash ledger; pay upcoming non-autopay bills and mark each occurrence paid + advance it; pay credit cards at the **statement balance** (not the naive full outstanding) and update both the card and the paying account; pre-mark autopay bills that are due-but-not-yet-cleared; and, when checking is projected negative, move money from savings/brokerage to cover it. Recurring bills carry **per-scenario amounts** for maternity-leave regimes (Standard → PDL → CFRA → Post-CFRA), where income drops and discretionary contributions pause. (Full detail lives in the `mlp-definition-spreadsheet-workflow` bead memory.)

## Decision

**Two sequential gates. MLP first, Launch second.**

### Gate 1 — MLP (`personal-cfo-j0cg`, active, P1)

The app fully replaces the spreadsheet for the owner's daily driving, and feels lovable. Its tracked arcs:

1. **Fast batch balance update** (`wgpb`) — one grid to update all bank + card balances each cycle (the Dashboard replacement).
2. **The pay-&-confirm loop** (`5ie.9` ConfirmObligationEarly + `mc7f` autopay flag) — mark a bill/card occurrence paid (manual, or autopay-not-yet-cleared), advance it to next due date, update balances.
3. **Credit-card payment at the statement balance** (`6wk.9` + `r7sb`) — pay the statement balance, reduce card outstanding (≠ statement) + the paying account.
4. **Cash-runway** (`3v6d` comfort band + `r7sb` transfer) — descriptive shortfall + a **user-steered "cover it" tool** (see the ADR 0018 amendment).
5. **Scenario UI** (`vru6` + `jgid`) — model maternity-leave income + paused-contribution regimes.
6. **Polish pass** (`2pcx`) — make the daily surfaces lovable (ADR 0031 design loop), with owner sign-off.

Net-pay entry is sufficient for MLP; gross→net withholding modelling (`3b8.1`) is post-MLP.

### Gate 2 — Launch (`personal-cfo-2owr`, after MLP, P2)

Only once MLP is met: signed + notarized build + DMG + real signed auto-update (`867.1`), vault password change (`zxq`), tested recovery guide (`vdmb`), CSV export (`hbd8`), a real-bank importer (OFX, `fr79`), the private→public **OSS fork** + companion website + buy-me-a-coffee donations. (Product vision + monetization north-star — subscriptions, Plaid, cloud sync, mobile — is captured in the `product-vision-launch-plan` memory and is explicitly *not* near-term.)

### Amendment to ADR 0018 (non-advice)

The owner's workflow ends in "move $X from savings to cover the shortfall." ADR 0018 forbids prescriptive advice. We resolve this as **descriptive + a user-steered tool**: the app states the projected shortfall descriptively ("checking dips to −$X on Jul 17"), and offers a **"cover it" helper** that proposes the amount and lets the user pick which reserve account to pull from and confirm the transfer. The app never *tells the user what to do* unprompted; it gives a tool the user drives. (An addendum is added to ADR 0018.)

## Consequences

- **Positive:** the roadmap now matches intent — we finish the product the owner will love and use daily before spending effort on signing/distribution. "Done" is a concrete, tracked MLP gate anchored to a real workflow, not a feeling. The graph reflects the two-phase reality.
- **Negative / trade-offs:** genuinely useful distribution work (signing, portability) is explicitly deferred behind MLP. The "lovable/polished" bar (`2pcx`) is partly subjective and gated on owner sign-off. Launch criterion 1 (signing) will still need the owner's Apple Developer account when we reach it.

## Revisit if

- Dogfooding reveals the MLP arc set is wrong or incomplete (expected — the owner is providing more dogfooding notes to refine arc 1 especially).
- The owner decides a distribution capability (e.g. a signed build in his own hands) is needed *before* full MLP, in which case that item is pulled forward into the MLP gate explicitly.
