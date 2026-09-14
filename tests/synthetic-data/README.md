# tests/synthetic-data

The **synthetic household data generator** and its output.

Generates realistic-but-fake households (accounts, balances, income schedules,
recurring bills, transactions) used to build the golden fixture vault and to
dogfa­ood the app without touching real financial data. Synthetic-only
dogfooding is the Week-8 starting point; real-data dogfooding is blocked until
the real-data safety gate passes (beads `personal-cfo-w01i`, `personal-cfo-rtez`).

The generator is owned by bead **`personal-cfo-9ujs`** (Synthetic household data
generator).

## Rules

- **No generated data is committed.** Output is produced on demand and is
  git-ignored. Only the generator code and its README live here.
- Generation must be **deterministic** given a seed, so fixtures are reproducible.
