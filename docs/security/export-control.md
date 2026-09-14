# Export control (US EAR) — classification and why no notification was filed

**Status:** Current as of 2026-09-05. Revisit triggers are at the end.
**Short version:** DohFlow is publicly available encryption software using only
standard, published cryptography. Under **15 CFR 742.15(b)(1)** it is **not
subject to the EAR**, and the email notification to BIS and the ENC Encryption
Request Coordinator is **not required**, because that requirement reaches only
*non-standard cryptography*. **No notification has been filed, and none is owed.**

This document is engineering's written basis for that position. It is not legal
advice.

## 1. What DohFlow is, for classification purposes

DohFlow encrypts a local financial database and its backups. That functionality
puts it in **ECCN 5D002** ("information security" software) — the starting point,
not the conclusion.

## 2. The rule

### 2.1 Source code

> **15 CFR 742.15(b)(1)** — publicly available encryption source code classified
> under ECCN 5D002 is **not subject to the EAR**.

"Publicly available" takes its meaning from **§ 734.3(b)(3)** (published
information and software), which routes through **§ 734.7** for what "published"
means. DohFlow's source is published in a public repository under AGPL-3.0, with
no licensing fee or royalty required for commercial production or sale of a
product developed from it — the condition § 742.15(b)(1) attaches.

### 2.2 The notification, and why it does not reach us

> **15 CFR 742.15(b)(2)** — notification to BIS and the ENC Encryption Request
> Coordinator is required where the source code **"provides or performs
> 'non-standard cryptography'"** as defined in Part 772.

**"Non-standard cryptography"** (15 CFR 772.1) means cryptography incorporating
**proprietary or unpublished** cryptographic functionality — algorithms or
protocols that have *not* been adopted or approved by a recognised international
standards body (IEEE, IETF, ISO, ITU, ETSI, 3GPP, TIA, GSMA) and have not
otherwise been published.

Every primitive DohFlow uses fails that test — that is, all of them are standard:

| Primitive | Where used | Standard |
|---|---|---|
| **AES-256-GCM** | key wrapping (`crates/vault-crypto/src/envelope.rs`, `Aes256Gcm`) | NIST FIPS 197; SP 800-38D |
| **Argon2id** | password → KEK derivation (`crates/vault-crypto/src/lib.rs`) | **RFC 9106** (IETF) |
| **SQLCipher (AES-256)** | database at rest (`crates/db-worker`) | FIPS 197; HMAC-SHA-512 per FIPS 198-1 / 180-4 |

No proprietary cipher, no unpublished construction, no home-rolled primitive.
DohFlow keys SQLCipher with a **raw key** (`PRAGMA key = "x'…'"`), supplying a
DEK derived by Argon2id rather than using SQLCipher's own KDF — this changes
which *published* KDF is in play, not whether the cryptography is standard.

**Therefore § 742.15(b)(2) does not apply, and no notification is owed.**

### 2.3 Object code — the binaries we actually ship

This is the part that is easy to get wrong, because § 742.15(b) is written about
*source* code and we distribute a compiled `.dmg`.

The governing text is the **Note to paragraphs (b)(2) and (b)(3) of § 734.3**:

> Publicly available encryption object code "software" classified under ECCN
> 5D002 is not subject to the EAR **when the corresponding source code meets the
> criteria specified in § 742.15(b)** of the EAR.

**The binary's exemption is conditional on the source being public.** It is not
independent. That has a consequence worth stating plainly, because it is a
licensing decision with an export-control side effect:

> **If DohFlow ever ships a binary whose corresponding source is not publicly
> available — a closed-source build, a paid tier with private code, a
> pre-release binary distributed before the repo is public — the object-code
> exemption does not apply to it**, and that build needs its own analysis.

## 3. Conclusion

1. DohFlow's source is publicly available 5D002 encryption source code → **not
   subject to the EAR** (§ 742.15(b)(1)).
2. It implements only standard cryptography → the § 742.15(b)(2) notification
   **does not apply**. None was filed. None is required.
3. Published binaries are likewise not subject to the EAR **for as long as the
   corresponding source stays public** (Note to § 734.3(b)(2)–(b)(3)).

Apple requires no export questionnaire for software distributed outside the App
Store. A future App Store submission asks export-compliance questions and will
need answering from this document rather than from memory.

## 4. Revisit triggers

Re-run this analysis if any of these becomes true:

- **A non-standard primitive is introduced** — any cipher, KDF, or protocol not
  adopted by a recognised standards body, including a "clever" tweak to a
  standard construction. This is the trigger that would create an actual
  notification obligation under § 742.15(b)(2).
- **A binary ships whose source is not public** (closed tier, private fork,
  pre-public release). See § 2.3.
- **The licence or repository visibility changes** such that the source stops
  being publicly available on § 734.7 terms.
- **Cryptanalytic functionality is added** — password cracking, key recovery.
  That is a different ECCN with a different regime.
- **App Store distribution begins** — answer Apple's export questions from here.
- **The EAR text changes.** Cites above were verified against the current CFR on
  2026-09-05.

## 5. Sources

- [15 CFR 742.15 — Encryption items](https://www.law.cornell.edu/cfr/text/15/742.15)
- [15 CFR 734.3 — Items subject to the EAR](https://www.law.cornell.edu/cfr/text/15/734.3)
- [15 CFR 772.1 — Definitions](https://www.law.cornell.edu/cfr/text/15/772.1)
