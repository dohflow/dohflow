# ADR 0066-A — Pinned service endpoints, credentials, and operational logs

- **Status:** Accepted
- **Tier:** Public (ADR 0082)
- **Date:** 2026-09-27
- **Bead:** `personal-cfo-bc8iv`
- **Decider:** Project owner (endpoint policy and 2026-09-27 operational-log approval)
- **Builds on:** [ADR 0066](0066-business-model-free-app-paid-services.md)
  §2, [ADR 0010](0010-tauri-window-capability-isolation.md),
  [ADR 0043](0043-oss-license.md), and
  [ADR 0074](0074-dohflow-sync-architecture.md) Decisions 1, 2, and 8–9;
  [ADR 0017](0017-sync-identity-and-clock.md) details device identity
- **Consumers:** `personal-cfo-elciw` (S3-1 Sync client), the future BACK-1
  backup-upload client, `personal-cfo-v98vd` (S2-1 Sync service), and later
  AI-proxy and extension-registry clients

## Context

ADR 0066 §2 promises that no local-app code path checks a license key,
subscription, or account to unlock behavior. ADR 0010 denies the main WebView
a general network opener; the shipped, narrow egresses are listed in
[`threat-model.md` TB3](../security/threat-model.md). Its current-state
statement that there is no app-managed Sync or backup upload is accurate
**today**. Optional future services will need a deliberate endpoint and
credential boundary, not an unnoticed widening of TB3.

The approved program plan v0.8.1 §4.2 and §2 invariant 11 make a paid service
credential an endpoint credential, never a local feature unlock. Risk R41 in
§17 identifies user-editable hosts as a phishing and data-egress surface.
ADR 0043 makes the client and its protocol public when they ship; a sole
hard-coded host would make an interoperable self-hosted endpoint require a
source fork. ADR 0074 commits Sync to client-side ciphertext and signed device
membership, but its service still handles opaque protocol metadata and can
deny or withhold availability. Those facts bound the endpoint and logging
contract here. This ADR specifies **future** service behavior; it does not
claim any DohFlow-operated Sync, backup-upload, AI, or registry endpoint is
currently reachable from the app.

## Decision

### 1. Pinned default for every DohFlow-operated service

Each DohFlow-operated endpoint—Sync, managed backup upload, a future AI proxy,
or a future extension registry—has a compiled-in default pinned to the
project host. The default host changes only through an app update. The
service is off until the user enables it; installing or updating the local
app does not silently enable an endpoint. Connector providers and the
already-shipped updater/opener retain their separately documented boundaries.
The endpoint and service credential are not a license check on local app
functionality.

### 2. Device-local override only after a Settings trust prompt

For each service, a user may configure **at most one** alternative egress in
Settings. The override is off by default. It is a `settings` `device.*`
value (class 3 under ADR 0074), never a replicated `user.*` value, and is
excluded from vault backup, logical Sync snapshot, and envelope tail. Import,
paste into ordinary content, deep link, URL scheme, file, and connector
payload cannot set or change an endpoint. The main WebView gains no general
network capability: a typed Settings action in the trusted app must mediate
the change and show the actual parsed destination host before saving it.

The confirmation uses this copy, substituting the actual service, host, and
service-specific data descriptions rather than hiding a URL behind a label:

> **Use another host for {service}?**
>
> Host: **{host}**
>
> This host will receive: {protocol data and credential description}.
>
> It can see: {service-visible metadata description}.
>
> Only continue if you trust the operator of this host with that data.
>
> **Cancel** · **Use this host**

For Sync, the descriptions explicitly name the service credential, encrypted
snapshots/envelopes/artifacts, and visible vault/device routing metadata,
sequence, sizes, and timestamps. The prompt does not suggest that encryption
prevents the host from delaying or withholding data. Service-specific clients
must supply equally concrete descriptions, including any plaintext the
particular protocol requires; they cannot reuse a misleading ciphertext-only
Sync description. The approved host is shown again in Settings. An endpoint
change never arrives through a backup or another device.

### 3. Future TB3 and threat-model rule

When the first managed endpoint ships, replace the current TB3 summary of
three shipped paths with this rule (listing the paths that actually ship at
that time):

> The app has N pinned paths plus at most one user-configured egress per
> service, off by default. A user-configured service host is set only in
> Settings after a trust prompt that displays the host and the data sent.

The current three-path TB3 statement and current-state public privacy page
remain unchanged in this docs-only change. Each endpoint-shipping bead adds
its own shipped threats-table row and revises public claims when its network
path exists. The planned Sync row is:

| Threat | Vector | Mitigation and remaining risk |
|---|---|---|
| Sync service reads or misuses data, equivocates membership/nonce state, or withholds availability | Pinned host or an owner-approved alternate host receives encrypted protocol objects and minimal routing metadata | ADR 0074 client encryption, client-verifiable signed membership, and client-derived per-device AEAD keys/nonces prevent the service from decrypting, adding a recipient, choosing a nonce, or causing duplicate derived-AEAD-key/nonce use by honest clients. Opaque epoch-scoped artifact IDs/manifests expose no bare plaintext digest or content hash. The service can still deny, delay, or withhold objects and observe necessary routing metadata. Fresh-key revocation protects future objects, not ciphertext already obtained. The Settings host/credential prompt addresses the alternate-host phishing risk, not service availability. |

For clarity, the planned service-visible protocol boundary is the following
contract; it is not an operational-log allowlist:

<!-- BEGIN SERVICE-VISIBILITY CONTRACT -->
| Surface | Fields or objects |
|---|---|
| Service-visible Sync protocol metadata | `vault_id`, enrolled `device_id` and fingerprint, accepted `seq`, object IDs, opaque epoch-scoped artifact IDs and closure sidecars, ciphertext sizes, timestamps, acknowledgements, and ADR 0074's authenticated clear envelope header including `nonce_domain` and `invocation_counter` |
| Service-visible accepted-envelope retry index | `command_id`, `idempotency_key_tag`, `canonical_envelope_digest`, immutable-metadata commitment, and prior `seq`/result reference plus recovery bindings required by ADR 0074 Decision 2 |
| Never service-visible local crypto or plaintext artifact fields | origin/receiver `StorageId`, wrapped/content-key fields, local content-key or materialization nonces, blob paths, local materialization mappings, plaintext artifact bytes, bare plaintext digest/content hash, and decrypted `SyncArtifactTransportV1` fields |
<!-- END SERVICE-VISIBILITY CONTRACT -->

`SyncArtifactTransportV1` is ciphertext-only at the service boundary: its
portable descriptor and bytes become readable only after client decryption.
The **Sync envelope** nonce fields are public authenticated header fields
under ADR 0074; the excluded nonces above are local attachment/content-key
and materialization fields. The service may store ciphertext, opaque IDs,
manifests, and the minimal routing/retry data required by the protocol, but
none of that gives it confidentiality or nonce authority.

### 4. A credential, never a local unlock

The local app executes the same local capabilities whether a subscription is
active, lapsed, or absent. A remote service can accept or reject the
credential presented to its configured endpoint; that affects only that
service request, not a local code path or existing local data. The future
service terms must say so. The first account UI implementation must add
`scripts/check-client-service-entitlement-branches.sh`: a CI check that
rejects subscription/plan/tier gating of client behavior outside the account
UI, with explicit review of domain-language false positives. Its integration
test must lapse a service credential while offline/local vault features stay
unchanged and usable. This ADR names those obligations; it adds no account UI
or runtime check now.

### 5. Protocol observability, no client telemetry

An enabled Sync client may send only its protocol traffic: service
credentials, signed membership and sealed-key records, encrypted snapshots
and command envelopes, opaque encrypted artifact objects and closure
manifests, receipts/pins, pull/push acknowledgements, and the clear
authenticated routing/header fields required by ADR 0074. It sends no
analytics, usage telemetry, automatic crash report, or automatic diagnostic
bundle. The service necessarily sees protocol `vault_id`, enrolled device
identifiers, `seq`, object sizes, timestamps, and acknowledgements; it never
sees plaintext financial or artifact contents. A service's **protocol
records** are not operational logs: ADR 0074 retains active/retained
snapshot and artifact roots under its published recovery windows and retains
the opaque accepted-envelope retry index for as long as any device may retry
its outbox, without automatic expiry. The seven-day operational-log rule
never expires protocol roots or accepted-envelope retry records.

For separate **operational logs**, the owner-approved platform default for
Sync and later backup upload, AI proxy, and extension-registry services is:

- Persist only failures and service lifecycle events; no routine per-request
  success or access history.
- Allow only timestamp, service/build version, fixed operation name,
  status/error category, and coarse duration bucket.
- Exclude vault/device/account IDs, sequence numbers, object sizes, ack
  history, IP addresses, user agents, credentials, full URLs, payloads, raw
  exception messages, and stable hashes of any excluded identifier.
- Delete automatically **within seven days**, with no separate log archive
  or backup. Access is limited to designated reliability/security operators
  investigating incidents, not analytics, marketing, or routine support.
- User-linked service records follow their separately approved deletion
  lifecycle. These non-user-indexed operational logs cannot be selectively
  retrieved or deleted by account; they expire within seven days. This
  limitation must be disclosed when the service ships.

This is a platform-wide default, not permission to collect other service
data. Any exception, including a second egress or extra log field, requires
explicit approval **before** collection. Hosting and CDN access-log settings
must conform; infrastructure that cannot meet the rule blocks deployment
until escalated. Billing records, security audit records, and future
service-specific data are not implicitly authorized by this policy. Seven
days is a product decision, not a claim of deployed compliance or a legal
requirement.

The only diagnostic export is a **user-initiated, previewed, redacted** bundle
through `personal-cfo-lyd` (local observability), `personal-cfo-ryjx`
(redaction round-trip), `personal-cfo-fps` (crash-report redaction), and
`personal-cfo-3cw` (privacy modes). The future `/help` page must say that
no client diagnostic upload happens automatically. Support diagnoses service
state from ciphertext-safe protocol records and the approved minimal failure
logs, or asks the user to send a redacted bundle. It does not get plaintext
vault data through the service.

### 6. The same boundary for AI and extensions

The free local app may use a user-supplied API key (BYO key) through the
existing Keychain-style secret boundary; it never needs a service
subscription to run local behavior. A future brokered-key AI proxy is a
separate service accepting a credential, not an unlock flag inside the app.
An extension registry may require an account credential to **acquire** a
package, but an already acquired extension runs without a payment check in
the local execution path. These clients inherit Decisions 1–5, including the
per-service trust prompt, egress rule, and operational-log default. The
later endpoint and extension designs still own their protocol details.

### 7. Openness is not a server-publication decision

ADR 0043's AGPL client and its published protocol permit interoperable
third-party servers. The Settings override makes them usable without a
client fork. Whether DohFlow publishes or self-host-enables its own server
implementation is the separate ADR 0078 decision. This ADR neither assumes
its answer nor changes ADR 0066's free-local-app boundary.

## Consequences and downstream checks

- S3-1 (`personal-cfo-elciw`), BACK-1, and every later endpoint client block
  on this accepted contract. S2-1 (`personal-cfo-v98vd`) implements the Sync
  service side. Add explicit blocker edges for future endpoint beads when
  they are created.
- Each endpoint-shipping implementation must test synthetic failure/lifecycle
  logs against the field allowlist and exclusions, seven-day deletion across
  live storage and hosting/CDN copies, designated-operator access, and the
  inability to collect routine success/access histories. It must prove log
  cleanup leaves ADR 0074 protocol roots and retry records intact.
- Client tests must prove an untrusted input cannot change a configured
  endpoint and that no diagnostic bundle uploads without preview and user
  action. A service deployment unable to enforce the log policy stops for
  approval rather than silently loosening it.
- The bead shipping the first managed endpoint updates live TB3, its threat
  row, and public privacy/fact-sheet claims. This ADR is docs-only and does
  not change current network behavior or current-state statements.

## Rejected alternatives

- **Empty or user-editable default host:** removes the pinned trust anchor
  and makes first setup an avoidable phishing surface.
- **Client subscription or license gating:** contradicts ADR 0066 §2 and
  makes local data use depend on an account state.
- **Config-file, URL-scheme, or payload endpoint changes:** bypass the
  Settings host disclosure and let untrusted content redirect egress.
- **Hidden reliability telemetry:** violates the local-first consent rule;
  failure/lifecycle service logs and user-initiated diagnostics supply the
  approved bounded evidence instead.
- **Hard-coded host with no override:** forces an AGPL client fork to use an
  interoperable third-party server.

## Revisit if

- A service needs a second egress (for example, a CDN for blobs): make an
  explicit reviewed row and trust disclosure, not a blanket network grant.
- The project host must move: rotate the pinned default through an app
  update, not a silent override of a previously approved host.
- Data residency requires regional placement: ADR 0075 governs the service
  region behind the pinned host; it is not a client-side endpoint override.

## Linked beads

- `personal-cfo-elciw` — S3-1 Sync client
- `personal-cfo-v98vd` — S2-1 Sync service
- BACK-1 — future managed backup-upload destination
- `personal-cfo-lyd`, `personal-cfo-ryjx`, `personal-cfo-fps`,
  `personal-cfo-3cw` — user-initiated redacted diagnostics
- `personal-cfo-klr.4` — ADR 0077 Sync wire format
- `personal-cfo-6kn` — ADR 0017 device identity and nonce-loss contract
