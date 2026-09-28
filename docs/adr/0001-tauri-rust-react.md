# ADR 0001: Use Tauri v2 + Rust core + React/TypeScript frontend

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-yy6`](../../.beads/issues.jsonl)
- **Related plan sections:** §0 (decision 1), §2.1, §2.5, §3.1
- **Supersedes:** None

## Context

Personal CFO is a security-sensitive, local-first household finance desktop app, macOS-first, with future cross-platform intent. The architecture must keep financial data encrypted at rest, hold the security boundary clearly inside one process, and stay maintainable by a small team (initially solo). It must support complex ledger / table / chart UI, expose a typed boundary between trusted code and presentation, and not introduce cloud dependencies for local manual use.

Two top candidates were evaluated:

1. **Tauri v2 + Rust core + React/TypeScript WebView frontend.**
2. **Native macOS SwiftUI app** with a Swift/Objective-C trusted core.

A third option — Electron + Node — was rejected up front: Node is a poor fit for the trusted security boundary we want, the bundle size is far larger than Tauri, and Node's supply-chain surface is unattractive for a finance app.

## Decision

Use **Tauri v2** as the desktop shell, with a **Rust** trusted core (the "Finance Kernel," ADR 0006) as the security boundary, and a **React + TypeScript (strict)** frontend rendered in the system WebView. The frontend is presentation only; it has no DB access, no provider credentials, no vault keys. All frontend → backend calls go through typed Tauri commands.

## Consequences

### Positive

- Rust gives memory safety in the trusted core, a strong type system at the security boundary, and excellent crypto/SQLCipher/serde ecosystems.
- React + TypeScript gives access to the AG Grid / TanStack Table / Recharts / Nivo / shadcn ecosystem for ledger and forecast UI — far more mature for our shape than SwiftUI's.
- Tauri v2's [capability model](https://v2.tauri.app/security/capabilities/) supports per-window/per-webview deny-by-default permission scoping, which fits a least-privilege design.
- Future Linux and Windows support is a recompile and platform-shim, not a rewrite.
- Larger and more accessible contributor pool than SwiftUI for an eventual OSS release.

### Negative

- The WebView is a real attack surface that SwiftUI does not have. We mitigate via strict CSP, Tauri capability isolation (ADR 0010), parser/document isolation (ADR 0022), and explicit trust-boundary rules (ADR 0003).
- macOS-native ergonomics (Touch ID, Keychain, App Sandbox, notarization, accessibility) require Rust bindings or `cocoa`/`objc2` interop rather than first-class SwiftUI APIs. We accept this and abstract platform integrations behind Rust traits (`SecretStore`, `BiometricAuth`, etc., per §3.3.1).
- Tauri's release-tooling, codesigning, and notarization story is less polished than Xcode's. Mitigation: pin Tauri v2 minor versions (`personal-cfo-io42`) and treat Tauri upgrades as their own PRs.

## Rejected alternatives

### SwiftUI native macOS

- ✓ Best macOS-native UX.
- ✓ First-class Keychain, LocalAuthentication, App Sandbox, notarization.
- ✗ Smaller and less diverse contributor pool for an OSS finance project.
- ✗ Weaker chart/table ecosystem for the kind of ledger and forecast UIs we plan to build.
- ✗ Cross-platform later means a rewrite, not a port.
- ✗ Memory-safety story for the trusted core is good with Swift but not as strong as Rust's, and the Swift ecosystem for crypto / SQLCipher / migration tooling is thinner.

### Electron + Node

- ✗ Node is not a credible security boundary for a vault application.
- ✗ Bundle size and resource consumption inappropriate for a small desktop app.
- ✗ Supply-chain attack surface (npm transitive deps) is unattractive for finance data.
- ✗ Process model encourages keys/secrets in JS-land; we want them in Rust-land.

### Pure web app (PWA / hosted)

- ✗ Local-first and encrypted-vault are non-negotiables.
- ✗ Browser storage / IndexedDB is not an acceptable home for unencrypted financial state.
- ✗ Conflicts with the §1.5 principle "manual mode must always work" without a server.

## Revisit if

- The Tauri WebView creates unsolved accessibility, performance, App Store, sandboxing, or security obstacles we cannot mitigate.
- Tauri v2's capability model regresses or the project is abandoned upstream.
- A specific macOS-native capability (e.g., Keychain biometric flow) becomes impossible to reach from Rust without unacceptable friction.

## Implementation notes

- Frontend bundle: Vite + React + TS strict + TanStack Router + TanStack Query + Tailwind + shadcn-style accessible primitives + React Hook Form + Zod.
- Rust workspace under `crates/` and `apps/desktop/src-tauri/`; no second JS package manager lockfile.
- Capability files under `apps/desktop/src-tauri/capabilities/` deny by default; the main window has no remote-URL navigation in production and no Tauri global exposure.

## Linked beads

- `personal-cfo-rhci` (Scaffold Tauri desktop app)
- `personal-cfo-dily` (React + TypeScript frontend shell)
- `personal-cfo-tif` (ADR 0010: Tauri window/capability isolation)
- `personal-cfo-1al` (ADR 0003: trust boundary)

## Accepted addendum (2026-09-28): verified Rust compiler support

- **Status:** Accepted
- **Tier:** Public — contributor build requirements and security engineering.
- **Decider:** Project owner
- **Bead:** `personal-cfo-g3m.5`

The supported bundled SQLCipher update requires a newer Rust compiler than the
previously declared minimum of 1.82. The upstream `rusqlite` 0.40 releases use
[`cfg_select!`](https://doc.rust-lang.org/stable/releases.html#version-1950-2026-04-16),
stabilized in Rust 1.95. Prefer a supported upstream security update over a
locally maintained database fork or source patch.

The declared minimum may advance to the **lowest verified compiler version in
the range 1.95–1.96**. Verification must use that actual compiler against the
complete locked workspace and standalone desktop dependency graphs, including
the relevant supported targets. A successful 1.96 build does not demonstrate
1.95 compatibility. Record the selected floor and its verification in
`docs/architecture/stack.md`, keep both manifests consistent, and enforce it in
CI. If the graph needs a compiler above 1.96, a supported target regresses, or
the required evidence cannot be obtained, obtain a new decision before
continuing. This addendum authorizes the bounded policy change; it does not
assert an untested minimum already works.

The **pinned build toolchain remains 1.96.0**. Contributors using older Rust
must update their compiler to build the updated dependency graph. This changes
source-build support only, not the supported OS or hardware floor. The bundled
SQLCipher/vendored OpenSSL architecture, Tauri pins, vault and backup formats,
key hierarchy, KDF, migration rules, and compatibility/security gates remain
unchanged. Preserve outgoing-engine synthetic fixtures before updating the
engine. Compiler approval is not permission to convert user data, waive a
release gate, or ship an unreviewed artifact.

Revisit this addendum if a later dependency requires a higher compiler floor
or changes platform support. Do not silently raise `rust-version` to make a
dependency build pass.
