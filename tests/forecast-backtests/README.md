# tests/forecast-backtests

Backtests that measure **forecast calibration and accuracy** — replaying the
deterministic Future Cash forecast (and later statistical layers) against known
synthetic histories and asserting the predicted balances match expected outcomes
within budget.

This is where forecast correctness is held honest over time: the product wedge
is a trustworthy forward-looking liquidity forecast (see
`docs/agent/PROJECT_PROFILE.md`), so regressions in calibration must fail CI.

Related: the Layer-1 deterministic engine (`personal-cfo-164u`), the forecast
property/invariant suites, and the performance budgets doc
(`docs/architecture/performance-budgets.md`, bead `personal-cfo-mtij`).

## Rules

- **No data is committed.** Backtest inputs come from the synthetic-data
  generator; expected-output snapshots are produced deterministically.
- Backtests must be **deterministic and reproducible**.
