# Contributing to DohFlow

This is the source repository for
[dohflow/dohflow](https://github.com/dohflow/dohflow).

> **Status:** pre-1.0. Bug reports and discussion are very welcome; for code
> contributions, please open an issue to discuss the change **before** sending
> a PR — the 1.0 arc moves fast and unsolicited PRs may collide with work in
> flight. Code contributions are accepted under a Contributor License
> Agreement — [`CLA.md`](CLA.md), enforced by the **cla-assistant** bot, which
> asks for a one-time signature on your first PR — per the AGPL-3.0-only + CLA
> licensing decision (ADR 0043, 2026-09-02 addendum). The CLA lets the project
> sustainably fund itself with a hosted version while the self-hosted app
> stays free and AGPL forever.

This project is built by AI coding agents and a human maintainer working under
a shared operating contract. The authoritative process docs are:

- **`AGENTS.md`** — universal operating rules (Git discipline, non-destructive
  policy, security, task tracking, session protocol). Read this first.
- **`docs/agent/PROJECT_PROFILE.md`** — stack, architecture boundaries, quality
  gates, and project-specific constraints.
- **`docs/architecture/definition-of-done.md`** — the Definition of Done every
  feature must meet (test layers, logging, redaction).

## Workflow at a glance

1. **Cite the GitHub issue** your PR addresses (see the note above) — you do
   not need a bead ID, and no CI check requires one. Bead IDs (`personal-cfo-
   xxxxx` in some PR titles) are this project's own internal task-tracking
   convention for its AI coding agents and maintainer; they mean nothing to
   an outside contributor and you can ignore them entirely.
2. **Branch first** — never commit substantive work directly to `main`.
3. **Small, coherent commits.**
4. **Run the quality gates before pushing** (see below).
5. **Open a PR** with a summary, verification results, and risks, citing the
   issue it addresses.

### For maintainers

This project's AI coding agents track their own work as **beads** — under
`.beads/`, private and untracked (`ADR 0064`) — which is why none of the
above mentions `bd ready` or a bead ID; that tooling is irrelevant to an
outside contributor and none of this repo's CI checks it. If you're a
maintainer setting up a second checkout, task/bead tooling setup currently
lives in `docs/operations/beads-backup-and-restore.md`; per ADR 0082 it is
relocating to the private `dohflow/internal` repository as part of
`personal-cfo-r36ck` — check whether that move has already happened before
following a stale reference here.

## Local setup

See the "Development setup" section of [`README.md`](README.md) for toolchain
installation and the build/test commands.

Also run this once per clone, to install the repo's tracked git hooks
(guards against a direct push to `main`, among other things):

```sh
git config core.hooksPath scripts/git-hooks
```

This is a manual, documented step — it is never set automatically by an
install script. See `docs/agent/WORKFLOW_ROLES.md` for what the guard does
and does not protect against.

If you also use `bd` (beads) for task tracking on this repo: there is no
need to run `bd hooks install --beads` — the tracked hooks above already
invoke `bd hooks run <name>` directly (`personal-cfo-bb96v`). If it's run
anyway, it resets `core.hooksPath`; just re-run the one-liner above.

## Quality gates

Run these before opening a PR (CI enforces the same set — see
`.github/workflows/ci.yml`):

```bash
# Rust workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Frontend (apps/desktop)
pnpm install
pnpm typecheck
pnpm lint
pnpm test
pnpm -C apps/desktop build

# The desktop Tauri crate is its own workspace (build the frontend first so
# apps/desktop/dist exists)
cd apps/desktop/src-tauri
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

## Security

Never commit secrets, real financial data, vault files, or local databases.
See `SECURITY.md` for reporting vulnerabilities.
