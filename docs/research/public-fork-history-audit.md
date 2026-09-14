# Public-fork history audit (fkt5 checklist item)

- **Date:** 2026-08-28
- **Scope:** the full git history — 757 commits, ~104.7 MB scanned — ahead of
  the public OSS fork (`personal-cfo-fkt5`, Launch gate `2owr` item 6).
- **Verdict: CLEAN. A history-preserving fork is safe**, subject to the one
  owner call below.

## What was scanned

1. **Secrets** — gitleaks 8.30.1 over all history (`gitleaks git .`), using
   the repo's `.gitleaks.toml` (built-in ruleset + the long-standing
   allowlist for generated bead data and redaction-test fixtures).
2. **Binary files ever committed** — every `*.png/jpg/pdf/zip/vault/db/sqlite`
   added in any commit, to verify no feedback screenshot, statement PDF, or
   vault file ever slipped in.
3. **Personal data** — content-level history search for the owner's email and
   any `@gmail.com` occurrence across tracked files.

## Findings

- **Secrets: none.** The only raw findings (3) were the connector arc's own
  `CFO-CANARY-…` leak-canary test fixtures — strings that exist to *prove*
  credentials never serialize. The canary pattern is now allowlisted in
  `.gitleaks.toml` (regex-scoped, nothing broader), and the full-history scan
  reports **no leaks found**. The CI secret-scan job uses the same config, so
  the public repo's scans run clean from day one.
- **Binaries: clean.** The only binaries in all of history are the stock
  Tauri app icons. No PDFs, screenshots, exports, or vault files were ever
  committed.
- **Personal data: only the git identity itself.** The owner's personal email
  appears (a) as the commit author/committer email on part of the history and
  (b) inside `.beads/issues.jsonl`, where the bead tooling records the same
  identity as issue owner. There are no other personal identifiers, no
  financial data (the repo has only ever held synthetic fixtures), and no
  third-party personal data.

## The one owner decision: fork mechanics

The email findings are **not leaks** — they are the git identity, and they are
inherent to any history-preserving fork (exactly like every OSS maintainer who
commits under a personal address). The choice `fkt5` already frames:

1. **History-preserving fork** (public repo keeps this history): free, keeps
   provenance and the ADR/commit narrative, exposes the author email — which
   any `git log` of any contributor shows anyway. *This audit finds no other
   blocker to it.*
2. **Scrubbed-snapshot fork** (fresh repo from a squashed snapshot): hides the
   email and the bead-graph history, at the cost of losing the public commit
   narrative. Only worth it if the owner wants the address out of the public
   record — in which case future commits also need a noreply identity.

Either way, `.beads/issues.jsonl` ships the owner identity by design; a
scrubbed fork would also want the export excluded or rewritten.

## Remaining fkt5 items (unchanged by this audit)

LICENSE file swap to AGPL-3.0-only + DCO (decision already made, ADR 0043),
SECURITY.md, contributor README/CONTRIBUTING, and the `sg8` pre-public
checklist — plus the fork-mechanics call above.
