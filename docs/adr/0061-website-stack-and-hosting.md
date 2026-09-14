# ADR 0061 — Website stack and hosting: Astro static on Cloudflare Workers, own repo, git-backed CMS

- **Status:** Accepted (owner decision, 2026-09-04)
- **Date:** 2026-09-04
- **Bead:** `personal-cfo-n76x.4` (this record); folds in `personal-cfo-h9q0x` (content authoring)
  and `personal-cfo-n76x.11` (analytics and waitlist)
- **Builds on:** ADR 0010 (capability isolation / opener scope), ADR 0043 (OSS licence),
  ADR 0054 (categorical chart palette — the contrast caveat below)
- **Related:** `personal-cfo-n76x.9` (deploy), `personal-cfo-n76x.10` (zone hardening),
  `personal-cfo-n76x.15` (IA), `personal-cfo-b5z1n` (CMS auth)

## Context

DohFlow needs a public marketing and documentation site before launch: a coming-soon
placeholder first, then a full site carrying `/download`, `/privacy`, help content, a blog,
and a press kit. The app itself is a local-first, zero-telemetry desktop application
distributed free under the AGPL, and the site is the first thing a Hacker News reader will
judge that claim against.

Three constraints shaped the decision. The site must be **portable** — we are asking people
to trust a local-first product, so the marketing site should not be hostage to a proprietary
CMS. It must be **agent-editable**, because most content changes will be made by a coding
agent. And it must be **owner-editable without an agent**: the owner explicitly requires the
ability to publish a blog post or fix copy from a browser, without a git commit and without
waiting on a session.

## Decision

### 1. Astro, static output, no adapter

Astro 6 with `output: 'static'` and **no** adapter. The build product is a `dist/` folder of
plain HTML, CSS and JS.

This is the portability clause, and it is load-bearing: if Cloudflare ever disappoints, that
folder deploys unchanged to Netlify, GitHub Pages, S3, or a VPS. No rewrite, no migration,
no lock-in. Zero JavaScript ships by default; interactivity is opt-in per component.

### 2. Cloudflare Workers static assets, not Pages

Cloudflare's own guidance is to start new projects on Workers; Pages remains supported with
no announced deprecation date. Static asset requests are free and unlimited, with limits of
20,000 files and 25 MiB per file, and `_headers` / `_redirects` support (100 rules,
2,000 characters per line). Workers Builds gives Git integration and per-branch preview URLs
on a free tier of 3,000 build-minutes per month.

**Why Cloudflare over the obvious alternatives.** Vercel's Hobby plan is non-commercial and a
site carrying donation links is arguably commercial, which puts it on a paid seat for a
static marketing page. Webflow and Wix were never candidates: content lives in a proprietary
CMS, export is lossy, and no real repository can sit underneath — the exact opposite of
clause 1. Cloudflare additionally consolidates registrar, DNS, DNSSEC, TLS, analytics and
Access-gated previews into the account that already holds the domain, which means fewer
accounts and fewer secrets in custody (`personal-cfo-7ie.7`).

### 3. Its own repository, `dohflow/dohflow-site`

Separate from the app repo so site deploys never touch app CI and a docs typo never queues a
Rust build. Created private under the `dohflow` org (`personal-cfo-fkt5.1`) rather than the
owner's personal account, so there is one org to administer rather than two. (This sentence
originally continued "...so it needs no transfer when it goes public on fork day" — that
assumed this repository would itself go public alongside the app repo. See the Amendment
below: `dohflow-site` stays private permanently, so no such transfer question ever arises.)

Production is `main`; every other branch gets a preview URL, and previews sit behind
Cloudflare Access until the full-site cutover (`personal-cfo-n76x.7`).

### 4. Content authoring: Sveltia CMS at `/admin`, git-backed

The owner must be able to publish without an agent. As originally specified the site had
content collections editable only by commit, which failed that requirement outright.

**Sveltia CMS**, self-hosted at `/admin`, GitHub backend, authenticated through the
first-party `sveltia-cms-auth` Cloudflare Worker. Every save is an ordinary git commit, so
owner edits and agent edits touch the same files with no divergent workflow.

**Chosen over Keystatic**, which is the closer competitor and better in one respect. Four
reasons, the first decisive:

1. **Framework-decoupled.** Sveltia is a static page plus one JS bundle and never enters
   Astro's dependency graph, so an Astro major upgrade cannot break the owner's editor.
   Keystatic couples to the Astro major — Astro 6 support arrived only in August 2026, after
   a fifteen-month gap in its Astro package. A CMS that breaks on framework upgrades defeats
   the independence this decision exists to create.
2. **Stack fit.** `sveltia-cms-auth` is a first-party Cloudflare Workers OAuth proxy, and we
   are already on Workers. Keystatic's browser editing needs a GitHub App wizard or paid cloud.
3. **Preserves zero-JS.** Keystatic is React; adopting it pulls React and react-dom into the
   site build. Sveltia adds nothing.
4. **Escape hatch.** Decap-compatible `config.yml` ports to Decap or a fork if Sveltia stalls.

**Accepted trade-off.** Sveltia's schema lives in YAML, duplicated against Astro's zod
content schemas — a real drift risk, and the one place Keystatic is clearly better with its
single TypeScript source of truth. **Mitigation, required:** a CI job parses every content
file against its zod schema, so a CMS-authored post with a missing or malformed field fails
on the pull request rather than in production.

Sveltia is also still 0.x. Its release cadence is high (multiple releases per week), and the
escape hatch in point 4 is the answer if that changes.

### 5. Analytics: Cloudflare Web Analytics, manual snippet. No waitlist.

Cookieless, no consent banner, no third-party tracker, and no cross-site identifier. The
manual snippet is used rather than the automatic proxy injection so the script is visible in
source and covered by the Content-Security-Policy hashes.

**No waitlist by default.** Collecting emails for a product with no ship date is a liability
we have no use for.

**This pairs with the app's zero-telemetry stance deliberately.** The app collects nothing
(ADR 0010, `personal-cfo-gj9e`); the site collects aggregate page views with no cookies. A
reader who arrives from Hacker News should find one consistent story — a privacy-first
application behind a tracker-laden marketing site would undermine the strongest page we have.
`/privacy` must describe exactly what is loaded, and the deployed page must match it.

Revisit only on a genuine need: per-page funnel analysis (Plausible, ~$9/mo) or an actual
reason to collect email (Buttondown, ~$9/mo). Neither applies at launch.

### 6. Security posture

Astro 6 hash-based CSP: `default-src 'self'`, `img-src 'self' data:`, `font-src 'self'`,
`frame-ancestors 'none'`, `base-uri 'self'`, `object-src 'none'`, plus the Cloudflare
Insights host only as clause 5 requires. `public/_headers` carries HSTS
(`max-age=31536000; includeSubDomains; preload` — the whole `.app` TLD is preloaded),
`nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: strict-origin-when-cross-origin`,
a `Permissions-Policy` denying camera, microphone, geolocation and payment, `COOP: same-origin`,
and immutable caching for `/_astro/*`.

**The `/admin` carve-out is path-scoped and must stay that way.** Sveltia is a client-side
application and needs a relaxed policy; that relaxation applies to `/admin` alone. The CI
header-assertion script asserts both the site-wide policy and that the carve-out does not
leak beyond `/admin`. External scanners run against production only, since previews sit
behind Access.

### 7. Design tokens are generated, not copied

`scripts/sync-tokens.mjs` in the app repo generates `tokens.css` from
`docs/design-system/design-tokens.json`, so the site and the app cannot drift into different
greens. **Contrast caveat, carried from ADR 0054:** terracotta `#db8f6b` on white is roughly
2.6:1 and is decorative only — body text and links use emerald `#006341` or a neutral.

## Consequences

**Good.** No lock-in at any layer. One vendor for domain, DNS, mail, hosting and analytics.
The owner can publish without an agent, and the agent can edit without a CMS. The privacy
story is consistent from app to site. Site deploys never touch app CI.

**Costs.** Two repositories to keep coherent. A YAML/zod schema duplication guarded by CI
rather than by types. A dependency on a 0.x CMS, mitigated by config portability. And the
`/admin` CSP carve-out is a permanent thing to keep honest — hence the assertion.

**Rejected alternatives.** Vercel (non-commercial free tier; a second vendor). Webflow and
Wix (proprietary content, lossy export, no repository). A hosted CMS such as Sanity or
Contentful (monthly fee, content leaves the repository, reintroduces the lock-in clause 1
exists to prevent). GitHub's web UI as the only editor (no preview, poor media handling —
it remains the fallback if `/admin` is ever down).

## Amendment (2026-09-11): the site repository stays private

**Decision.** `dohflow/dohflow-site` is never made public. Only the application
repository, `dohflow/dohflow`, goes public — and only as a snapshot on go-live
day (ADR 0062's amendment). The site repository stays private permanently and
keeps its full history. (Bead: `personal-cfo-pedp9`.)

This does not change the deployed *website*: `dohflow.app` is already live and
publicly reachable today (the coming-soon placeholder, `personal-cfo-n76x.9`) —
Cloudflare Workers Builds serves it from this repository regardless of the
repository's GitHub visibility. What stays private forever is the **source
repository**, not the site a visitor sees.

**Why, recorded so it is not relitigated:**

1. The site is marketing, not the product — the AGPL positioning and the
   open-source commitment cover the application. Nothing about "DohFlow is
   open source" implies the marketing site must be too.
2. Publishing would expose the implementation and full bug history of
   `workers/cms-auth`, the project's only server-side component — a GitHub
   OAuth proxy whose git history contains six fixed bugs, one a reflected
   XSS. A clean present-day implementation does not erase a public history of
   past vulnerabilities in the same file.
3. Publishing would also publish draft blog posts and in-flight launch copy
   sitting in content collections ahead of their own release.
4. It buys nothing: the zero-telemetry claim central to the site's own pitch
   is already independently verifiable from the deployed site's own
   `view-source:` output and from `/privacy` — a public repository adds no
   evidence a skeptical reader doesn't already have.
5. Publishing is the irreversible direction — once history is public it
   cannot be un-published — the same asymmetry that drove the snapshot
   decision (rather than a rewrite-and-transfer) for the app repository.

**Provenance of the old assumption.** The original line above ("so it needs no
transfer when it goes public on fork day") *assumed* future publication rather
than deciding it — no ADR ever made that an explicit decision, and nothing
examined it again until this amendment.

**Consequences:**

- The Astro 6-to-7 upgrade (`personal-cfo-nwzri`) loses its pre-launch driver.
  Its eight advisories were independently shown unreachable in this static,
  adapter-less site (see that bead's reachability analysis), and — since no
  public audit of this repository is now possible — there is no longer a
  "before a stranger can read the source" deadline either. It becomes ordinary
  post-launch maintenance, still worth doing, no longer urgent.
- Community contributions to site *content* are not a path this project
  offers — the Sveltia CMS at `/admin` and direct requests to the owner are.
  The Contributor License Agreement and the contributor agreement apply to
  the application repository only; nobody outside the owner ever has a reason
  to open a pull request against `dohflow-site`.
