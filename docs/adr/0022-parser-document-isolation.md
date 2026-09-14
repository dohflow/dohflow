# ADR 0022: Parser / document isolation boundary

- **Status:** Accepted
- **Date:** 2026-06-24
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-v4l`](../../.beads/issues.jsonl)
- **Related plan sections:** §8.1.2 (parser isolation)
- **Builds on:** ADR 0003 (Rust ↔ WebView trust boundary), ADR 0010 (capability isolation), ADR 0014 (the ingestion pipeline these parsers feed)

## Context

Importing — and, later, parsing statements — means taking **untrusted files**
(CSV/OFX/QFX/QIF, then PDF/image) from outside the trust boundary and turning them into
records. A malformed or hostile file attacks the *parser*, not the user: PDF-library
memory bugs, XML external-entity (XXE) and "billion laughs" entity expansion,
decompression/zip bombs, and CSV **formula injection** (`=cmd|...` cells that execute
if the data ever reaches a spreadsheet).

A **virus scanner is the wrong tool here** (ratified with the project owner): signature
AV defends against *known malware* — not a bug in our own parser — and it depends on
regularly **downloaded signature updates**, which cuts against the offline-first, no-
network design (and the threat model in `docs/security/threat-model.md`). The right
defense is **isolation + hardened parsing + treating output as untrusted data**.

## Decision

### 1. Untrusted-file parsing runs in a narrowly-scoped, sandboxed worker

A parser receives **bytes** and returns **typed records or a typed error** — nothing
more. Inside the parsing boundary there are **no vault keys, no database handles, no
network, no frontend IPC, and no ambient filesystem authority**. A parser library bug
therefore cannot reach the things worth protecting.

### 2. Every parse is explicitly bounded

Max file size, max memory, max page count (PDF), max rows/records, and a wall-clock
timeout. Exceeding any bound fails the parse **cleanly** — a zip bomb or an
entity-expansion attack hits a limit, not an out-of-memory crash.

### 3. Hardened, memory-safe parsers

Rust parsers; XML external entities and entity expansion **disabled** (no XXE, no
billion-laughs); archive/decompression bounded; and CSV/spreadsheet cells treated as
**data only** — never formulas, never executed.

### 4. Parsed output is untrusted data

It is never executed. It is **sanitized before it is ever rendered** (`personal-cfo-jah`
covers display sanitization of parsed text and any later LLM markdown). It re-enters the
trusted core only as **typed, validated records** through the ingestion staging pipeline
(ADR 0014) — the same chokepoint that handles dedupe, provenance, and triage.

### 5. The mechanism graduates with the attack surface

The **isolation principle applies to every parser**; how heavy the sandbox is scales
with risk:

- **R2 structured importers** (CSV/OFX/QFX/QIF — simple grammars, small attack surface):
  hardened, bounded parsing within the worker boundary above.
- **The later documents tier** (PDF/image/OCR — large attack surface): stronger
  process- or WASM-level sandboxing, revisited when the statement parser (`h0il`) and
  the OCR-engine decision (`nqii`) land.

### 6. No antivirus dependency (ratified)

Signature-based AV is **not** part of the model. If a user has an OS-level scanner, an
**optional, off-by-default** shell-out hook may be offered later — but it is never a
core or networked dependency.

## Consequences

- A bug in any parser library is contained: it cannot read keys, touch the database,
  reach the network, or script the UI.
- Resource-exhaustion attacks fail safe against explicit limits.
- Offline-first is preserved; no signature feeds, no network.
- The boundary is reusable for every future untrusted-input path (connectors, documents).
- Cost: parsing happens behind a worker boundary with a typed bytes-in/records-out
  contract — importers can't take shortcuts straight into the core.

## Alternatives considered

- **Antivirus scanning of uploads.** Rejected — see Context: wrong threat, and it
  breaks offline-first.
- **In-process parsing with no isolation.** Rejected: a single parser bug becomes full
  compromise (keys, DB, network).
- **Parsing in the WebView/frontend.** Rejected: the wrong side of the trust boundary
  (ADR 0003); untrusted bytes must not be handled where the UI runs.

## Decisions ratified (2026-06-24)

- **No antivirus** dependency; isolation + bounded, hardened parsing + untrusted-data
  discipline is the model.
- Parsers run sandboxed (no keys / DB / network / IPC / ambient FS) with explicit
  resource limits; the sandbox strengthens for the high-surface documents tier.
