# ADR 0075 — DohFlow account backup (public stub)

- **Status:** Accepted (owner decision, 2026-10-18)
- **Tier:** Public stub (internal decision)
- **Date:** 2026-10-19
- **Bead:** `personal-cfo-2bxkg.10`; the decision itself is `personal-cfo-oh3hg`
  and its 2026-10-18 note, `personal-cfo-2bxkg.8`
- **Decider:** Owner
- **Related:** [ADR 0066](0066-business-model-free-app-paid-services.md) (the
  local app is free and unchanged), [ADR 0066-A](0066-A-dohflow-endpoints-pinned-credential-not-unlock.md)
  (an account is a credential, never a local unlock),
  [ADR 0074](0074-dohflow-sync-architecture.md) (the service holds ciphertext
  only), [ADR 0082](0082-public-disclosure-boundary.md) (why this is a stub)

ADR 0075 is an internal-tier decision (ADR 0082, decision 3). Its full text,
reasoning and figures live in the private `dohflow/internal` repository and are
not reproduced here. This stub states only the part that a person using DohFlow
can see, in the words the app uses. It grows when more of ADR 0075 becomes
visible to users.

## Decision

### Backup to your DohFlow account

DohFlow Sync includes encrypted backup to your DohFlow account, and restoring a
backup from your account. It is part of Sync, not a separate product.

### What the account keeps

The account keeps about a year of your backups: the newest backup from each of
the last 7 days, the last 4 weeks and the last 12 months. It also always keeps
your newest backup that was verified after upload, and never silently deletes it.
Older backups are pruned by the service. All of this fits within your plan's
storage, and the app tells you plainly when a backup would not fit.

### Your backups stay private

Backups are encrypted on your Mac before they leave it. The service stores
ciphertext only and never holds a key that can open a backup (ADR 0074).

### The free local backup is unchanged

The scheduled backup to a folder you choose stays free, needs no account, and
keeps every backup. Nothing about an account changes how the local app works.
Losing or ending an account never locks or limits your local data (ADR 0066,
ADR 0066-A).

## Consequences

- The Settings copy that describes account backups (`personal-cfo-2bxkg.5`) uses
  these words, and the claims check holds the app and the site to them.
- The service's retention behavior is implemented and tested in
  `personal-cfo-2bxkg.5`; this stub fixes the promise, not the mechanism.

## Revisit if

Another user-visible part of ADR 0075 ships. Extend this stub then, with the
user-visible outcome only.
