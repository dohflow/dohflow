# Security Policy

DohFlow is a local-first household finance application. Security and
privacy are the project's top-ranked principles: financial data is encrypted
at rest, stays on the user's device, and is never sent anywhere without an
explicit user action. For the plain-language version of what that means —
no legal or technical background required — see
[`docs/public/privacy.md`](docs/public/privacy.md).

## Reporting a vulnerability

Please report suspected vulnerabilities **privately** — never in a public
issue:

- Use GitHub's **private vulnerability reporting** ("Report a vulnerability"
  under the repository's Security tab). This is the preferred channel.
- Or email **security@dohflow.app** if you cannot use GitHub, or prefer not to.
  This address is also published in
  [`/.well-known/security.txt`](https://dohflow.app/.well-known/security.txt).

Include: the affected component, reproduction steps, an impact assessment,
and any relevant logs **with financial data redacted**. Never include real
account data, balances, credentials, or vault files in a report.

### What happens next

These are commitments from one maintainer, not a funded security team. They are
deliberately modest so they can actually be met.

| Stage | Target |
|---|---|
| Acknowledgement that a human read your report | **7 days** |
| Initial assessment — severity, whether it is confirmed | **14 days** |
| Fix released, or a written explanation of the delay | **90 days** |
| Public advisory | at fix release, or by day **90** |

If a deadline will be missed you will be told before it passes, with a reason.
Silence is a failure on our part, not a policy.

### Coordinated disclosure

**Please give us 90 days** from acknowledgement before publishing. If a fix lands
sooner, the advisory goes out sooner and you may publish immediately.

**You may publish after 90 days regardless of whether a fix exists.** That is a
commitment, not a concession — an unbounded embargo is how bugs stay unfixed. If
the issue is already being exploited, tell us and publish on whatever timeline
protects users; we will not object.

Credit is given by name or handle in the advisory unless you ask otherwise.
There is **no bug bounty** — this is an unfunded project and it would be
dishonest to imply otherwise.

### Safe harbour

If you make a good-faith effort to follow this policy, we will not pursue or
support legal action against you, and we will say so publicly if someone else
does. Specifically, we consider your research authorised, and will not treat it
as a violation of the AGPL, the CFAA, or any anti-circumvention rule.

Good faith means: **test only against your own data and your own installation**;
do not access, modify, or exfiltrate anyone else's financial data; do not degrade
the service for others; stop as soon as you have demonstrated the problem; and
report privately through the channels above.

Because DohFlow is local-first, almost all research happens on your own machine
against your own vault — there is very little that could go wrong for anyone else.

### PGP

**None is published, deliberately.** GitHub private vulnerability reporting is
end-to-end private and is the preferred channel; email to `security@dohflow.app`
runs over TLS. A PGP key that is never rotated and whose private half is stored
casually is worse than no key, because it implies a guarantee that is not being
kept. If you need encrypted email, say so in a first message with no sensitive
detail and a key will be provided for that exchange.

## Supported versions

Pre-1.0, only the **latest released version** receives security fixes. There are
no long-term support branches and no backports.

| Version | Supported |
|---|---|
| Latest release | **yes** |
| Anything older | no — upgrade |
| `main` between releases | fixes land here first |

This tightens after 1.0. Until then, running an old build means running known
bugs, and the honest advice is to stay current.

## Security posture (what is actually implemented)

- **Encryption at rest**: SQLCipher vault (pinned engine version with a
  cross-version compatibility test), Argon2id KDF (compiled-in parameter
  profiles at or above the OWASP floor, asserted by a CI test), zeroized key
  material, per-attachment content keys.
- **Trust boundary** (ADR 0003/0010): the WebView holds no keys, no database
  handles, and no provider credentials; every financial mutation goes through
  the typed Finance-Kernel command bus; Tauri command ACLs are deny-by-default
  with a CI drift test, and destructive commands live in a separate,
  deliberately short capability list.
- **Untrusted input** (ADR 0022): importers parse untrusted bytes inside
  bounded workers (size/row/time limits, panic containment) with no keys, DB,
  or network; raw bytes are never persisted (shred-after-parse).
- **Bank connections** (ADR 0004/0060): no project-owned provider secrets
  exist anywhere; the SimpleFIN flow is user-token only, credentials are
  stored inside the encrypted vault, never logged (type-level enforcement +
  redaction rules + tests), and the transport follows no redirects and pins
  HTTPS.
- **Log hygiene** (§6.6): a global redaction layer scrubs account numbers,
  balances, tokens, and credential-bearing URLs from all logs; merchant names
  and free-text descriptions are kept out of logs by source-level field
  discipline — both enforced by a release-blocking CI corpus test.
- **Supply chain**: CI runs gitleaks (full-history secret scan with a
  tightly-scoped allowlist), `cargo audit` on both Rust workspaces, and
  `pnpm audit`. A full-history audit ahead of the public fork is documented
  in `docs/research/public-fork-history-audit.md`.
- **Distribution**: signed + notarized macOS builds, with an in-app
  auto-update channel — a minisign-signed release feed pinned to a single
  endpoint, refusal on a tampered or mismatched-signature artifact, verified
  by a live smoke-tested round trip (ADR 0068).

Design references: `docs/adr/0002-local-encrypted-vault.md`,
`docs/adr/0003-trust-boundary.md`, `docs/adr/0004-connector-relay-boundary.md`,
`docs/security/`.

## Known limitations (pre-1.0)

- No external security review has been performed yet; the pre-release gate
  is an internal, evidence-based review instead (`docs/security/release-review-v0.1.md`)
  — an external pentest is deferred until revenue justifies it (bead
  `personal-cfo-5kua`).
- The threat-model document (`docs/security/threat-model.md`) exists and is
  reviewed as part of every pre-release cycle, but is a living document — it
  does not yet cover every surface shipped since it was written.
- macOS is the only supported platform today.

## Export control

DohFlow is publicly available encryption software using only standard, published
cryptography (AES-256-GCM, Argon2id, SQLCipher). Under 15 CFR 742.15(b)(1) it is
not subject to the EAR, and the BIS/NSA notification requirement does not apply
because it reaches only *non-standard* cryptography. No notification has been
filed and none is required.

The full analysis, with citations and the triggers that would change this
conclusion, is in [docs/security/export-control.md](docs/security/export-control.md).
