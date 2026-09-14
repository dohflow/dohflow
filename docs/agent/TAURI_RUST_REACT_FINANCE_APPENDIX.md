# Tauri v2 + Rust + React/TypeScript Personal Finance Appendix

Use this as the project-specific appendix for a Tauri v2 desktop app that handles financial data.

## Stack profile

- Desktop shell: Tauri v2
- Core/backend: Rust
- Frontend: React + TypeScript
- Frontend build tool: Vite unless the project chooses otherwise
- Package manager: TBD; choose once, then keep exactly one lockfile
- Data sensitivity: high; financial and personal data must be treated as private by default

## Architecture boundaries

- Rust owns filesystem access, database access, encryption, import/export, financial calculations, and OS integration.
- React/TypeScript owns UI rendering, form state, navigation, and user interaction.
- The frontend must not directly handle unrestricted filesystem paths, raw secrets, bank credentials, or privileged OS operations.
- All frontend-to-Rust calls go through explicit Tauri commands.
- Every Tauri command must validate inputs, return structured errors, and avoid leaking sensitive values.
- Keep Tauri capabilities least-privilege and window-specific.
- Do not add broad wildcard capabilities without explicit justification.

## Privacy and security rules

- Never use real bank data in tests, fixtures, screenshots, logs, demos, or commits.
- Use synthetic test data that is clearly fake.
- Do not add telemetry, analytics, cloud sync, remote backup, crash reporting, or external network calls without explicit user approval.
- Do not log account numbers, transaction descriptions from real data, balances, imported file contents, tokens, or encryption keys.
- Prefer local-first storage unless the user explicitly approves sync/cloud features.

## Suggested quality gates

Adjust once the package manager and scripts exist.

```bash
# Rust
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Frontend placeholders; replace with the chosen package manager
<pm> run typecheck
<pm> run lint
<pm> test
<pm> run build

# Tauri build when appropriate
<pm> run tauri build
```

## Package manager decision

Choose one during scaffolding and document it in `docs/agent/PROJECT_PROFILE.md`:

- npm: simplest default for many users
- pnpm: efficient and strict dependency layout
- Bun: fast, but verify compatibility with project tooling
- Yarn: acceptable if intentionally chosen

After choosing, agents must not introduce any other lockfile.
