# services/connector-relay

**Placeholder.** No implementation yet.

This directory will hold the optional self-hosted / managed **connector relay**
that brokers third-party financial-data connectors so provider secrets never
live in the desktop app. It is **not** required for the MVP — the app is fully
useful in manual-only mode, and a connected account is an enhancement, not a
dependency (see `docs/agent/PROJECT_PROFILE.md`).

Implementation is owned by bead **`personal-cfo-pxi.1`** (FEATURE:
services/connector-relay implementation), with architecture in
`docs/architecture/connector-relay.md` (bead `personal-cfo-2lnn`).

This crate/service is intentionally **excluded** from the root Rust workspace
until it is implemented.
