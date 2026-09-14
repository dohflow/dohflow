# R1 (MVP) acceptance checklist

The single must-pass checklist for **Gate R1** — the first useful, private, offline
cash-forecast release. Source of truth: the plan's §22 (R1 deliverables) and §25.3
(MVP definition), aggregated by the gate epic **`personal-cfo-qmqk`** and the MVP
bead **`personal-cfo-wscx`**.

Status legend: ✅ done · 🔄 in progress · ⏳ remaining · ➡️ deferred (with target).

> Scope note (dogfooding R2 re-map, 2026-06-23): the MVP definition in §25.3 predates
> the re-map that moved **import / categorization / tags / splits** to **R2**
> (`personal-cfo-1xz3`) under the additive-balance direction. Those lines are marked
> ➡️ R2 below; they are **not** R1 must-pass.

## Gate R1 success criteria (`qmqk`)

| # | Criterion | Status | Evidence |
|---|-----------|--------|----------|
| 1 | Encrypted vault create / unlock / lock | ✅ | vault-crypto chain `j0o`→`1t0`→`vhv`→`tg5`; UI `3ry`/`8v2` |
| 2 | Manual liquid accounts (+ edit/archive) | ✅ | `6wgi`, `0eft`, `4d8.3` |
| 3 | Opening balances are equity postings | ✅ | `CreateAccount` opening-equity posting (db-worker / ADR 0007) |
| 4 | Manual salary income (+ edit/archive) | ✅ | `le79`, `tch0` |
| 5 | Manual recurring bills (+ edit/delete) | ✅ | `esmy`, `apso`, `zl1l` |
| 6 | Deterministic 30 / 90-day Future Cash | ✅ | `164u` (pure engine), `l8oh` (per-account); golden + determinism `rdg9`/`tv3w` |
| 7 | Dashboard: cash, next income, next bills, forecast chart | ✅ | `1vd7`, `d5qy` |
| 8 | Encrypted backup + verified restore | ✅ | `ef3`, `au3`, `dvxm`, restore drill `7pfu` |
| 9 | Log-redaction tests pass | ✅ | `2vs` (redactor), `zobt` (release-blocking corpus) |
| 10 | Real-data safety gate (no real data before backup/restore + redaction) | ✅ | `zxvl`, `zobt`, `7igv`, `c545`, `ef3`/`au3`/`7pfu` — **gate passed** |

## §25.3 MVP definition — additional bars

| Criterion | Status | Evidence / note |
|-----------|--------|-----------------|
| Manually enter data + see a useful forecast | ✅ | accounts/income/bills → Future Cash; first-run wizard `uipt` |
| Vault encrypted + password-protected | ✅ | ADR 0002; password-strength meter `00xl` |
| Useful offline | ✅ | local-first Tauri, no network (ADR 0001/0003; strict CSP) |
| Forecast assumptions editable | ✅ | assumption events `5u2`, scenarios `6zep`, manual entries `q6gh` |
| Dashboard: current cash, upcoming bills/income, forecast | ✅ | `1vd7`, `d5qy`; safe-to-spend `fqbm`; readiness `6vj9` |
| No plaintext financial data in logs | ✅ | `zobt` |
| Backup / export works | ✅ backup · ➡️ CSV export | backup `ef3`/`au3`; plaintext CSV export `hbd8` is convenience (optional for R1) |
| Categorization rule-based + correctable | ➡️ R2 | demoted to `1xz3` under the additive-balance direction |

## Beyond the original list — R1 hardening shipped this cycle

Additive balance model (`mkq1`/ADR 0027, `xmc`, `ueg6`, `hxjj`) · cash availability
(`fqbm`/ADR 0029) · Forecast Readiness (`6vj9`/ADR 0026 §13) · First Forecast Wizard
(`uipt`) · rich Future Cash viz (`l8oh`/`ygjs`/`l916`/`d5qy`) · vault resilience
(health check `n9w`, recovery wizard `5ivp`, password meter `00xl`) · golden-vault +
determinism suites (`rdg9`/`tv3w`).

## Remaining for the gate

| Item | Kind | Disposition |
|------|------|-------------|
| `w01i` — real-data dogfooding | Usage (P0) | **The gate-close step.** Its blocker (the safety gate) has passed, so it is unblocked; its "synthetic-only / blocked" wording is stale. This is a usage period, not a build. |
| `l28f` — chargebacks/refunds/reversals test suite | Test | ➡️ Best done with R2 transaction-handling — reversals are not an R1 manual-entry concern. |
| `jah` — sanitize document/markdown/LLM text | Security | ➡️ Premature for R1 — nothing renders parsed-document or LLM text yet (parser isolation/agents are later). |
| `xuu` — Keychain + Touch ID unlock | Convenience | Optional polish; not a must-pass criterion. |
| `hbd8` — plaintext CSV export | Convenience | Optional; encrypted backup already covers data safety. |
| `hlh` — Phase 1 epic | Aggregator | Closes when its tracked children do. |

## Verdict

**All ten `qmqk` success criteria and the §25.3 MVP functional + safety bar are met.**
R1 is functionally complete; the gate closes after the `w01i` real-data dogfooding
period. The remaining tracked beads (`l28f`, `jah`, `xuu`, `hbd8`) are convenience or
R2 work, not R1 must-pass.
