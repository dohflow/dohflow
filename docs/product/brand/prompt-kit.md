# DohFlow brand prompt kit

- **Bead:** `personal-cfo-n76x.3.2` · **Status:** draft, pending owner approval of §4
- **Source of truth for direction:** `docs/product/brand-direction.md`
- **Source of truth for colour:** `docs/design-system/design-tokens.json` (never retype hexes from memory)
- **Private until the fork-day flip.**

> **How to use this file.** It is written so that a session with *no other context*
> can produce on-brief work from it alone — that is the verification in §9. If you
> find yourself needing information that isn't here, that's a bug in this file:
> add it rather than working around it.

> **Every AI output is a draft.** Nothing generated goes into the product without
> human selection and edit, and every accepted asset gets a row in
> `PROVENANCE.md`. This is not ceremony: US Copyright Office guidance (Jan 2025)
> is that prompts alone earn no copyright, so the mark is protected by the
> trademark position and by the record of human authorship — not by which model
> drew the first draft.

---

## 1. Brand brief

**DohFlow** — cash flow, but cash became *dough* (the oldest slang for money),
and dough became *doh* to keep bread and bakeries out of the search results.

**What the product is:** a local-first personal finance app for macOS. The vault
is encrypted on the user's own machine. No telemetry, no account, no tracking.
Free and open source.

**What it's actually for — the differentiator:** forward planning. Realistic
scenarios of where cash is *headed*, not primarily backward-looking budget
tracking (though it does that too).

**The centrepiece metaphor**, and the thing every illustration should be built on:

> **Composing a scenario is _sculpting_. Applying it is _firing the kiln_. The
> realized outcome is the _pottery_.**

**Tone: finance and fun, held in balance.** Whimsy lives in illustration,
motion, and copy accents. **Financial numbers stay sober — never whimsical in
the digits, tables, or forecasts.** A person deciding whether they can make rent
must never feel the interface is being cute at them.

**Tagline in use:** "See your money before it happens."
Alternates, if a shorter or longer form is needed: "Know where you'll stand." ·
"Forecast first." · "Your money, before it happens."

**The clay wink is deliberate but narrow.** "Doh" nods at modeling clay because
users sculpt their financial future. **Explicit Play-Doh references are
off-limits in imagery and copy** — owner instruction, trademark caution. See §4.

---

## 2. Palette and type

Authoritative values live in `docs/design-system/design-tokens.json`. Reproduced
here for prompt use only; if they ever disagree, the JSON wins.

| Role | Light | Dark | Use |
|---|---|---|---|
| Emerald (primary) | `#006341` | `#12a06b` | The brand colour. Wordmark, primary actions, links |
| Terracotta (accent) | `#DB8F6B` | `#DB8F6B` | Illustration, accent rules, secondary chart series |
| Paper (ground) | `#faf8f6` | `#0e1411` | Backgrounds |
| Ink (text) | `#1c1815` | `#eef2ef` | Body copy |
| Chart series | `#006945` · `#d38058` · `#3a6fa5` · `#8e492e` | — | Categorical data |

The Mexican-flag green and Saltillo terracotta already read as a pottery homage.
**Expand the palette only if genuinely needed** — extra colours dilute it.

> ### ⚠️ Contrast rule — non-negotiable
> **Terracotta `#DB8F6B` on the light ground is ≈2.6:1. It is DECORATIVE ONLY.**
> Never body text, never links, never anything a user must read. Body and links
> use emerald or a neutral. This is carried from ADR 0054 and applies to
> generated imagery too — no terracotta lettering on pale ceramic.

### Type — two families, split on purpose (ADR 0063)

| Role | Family | Where |
|---|---|---|
| **Display** | **Nunito** (variable 200–1000) | Wordmark, headings, hero and marketing copy, illustration captions |
| **UI + data** | **IBM Plex Sans** (variable) | Body, tables, forecasts, balances, forms, **all numerals** |

The split encodes the brand principle in the type system itself: warmth where the
brand speaks, sobriety where the money is. Nunito's rounded terminals pair with
the claymation and pottery language; Plex keeps the digits sober, and its
**slashed zero** earns its place in columns of financial data.

**The enforceable rule:** *if a number can appear in it, it is IBM Plex Sans.* A
heading containing a figure — "Forecast: $4,210" — is data, not display.

Both self-hosted (`@fontsource-variable/*`). Nunito's variable default weight is
**200 (ExtraLight)** — always set a weight explicitly or it renders wispy.

**The wordmark is typeset in Nunito, never generated** — see §5.2.

---

## 3. Style vocabulary

Phrases that reliably land the intended look. Combine 3–5; more than that and
models start averaging them into mush.

**Material and craft**
`talavera ceramic` · `hand-thrown clay` · `matte glaze` · `unglazed terracotta`
· `kiln-fired earthenware` · `Mexican pottery motifs` · `Saltillo tile` ·
`thrown on a potter's wheel` · `visible thumb marks in the clay`

**Render and finish**
`soft 3D claymation render` · `stop-motion clay animation still` ·
`warm studio light` · `soft directional key light from upper left` ·
`shallow depth of field` · `matte finish, no specular highlights` ·
`subtle subsurface scattering`

**Composition**
`generous negative space` · `single subject, centred` · `isometric three-quarter
view` · `flat vector, single colour` (for marks) · `editorial illustration`

**Flow — the name is in the material, use it**
`glaze pooling and running` · `poured slip` · `thrown on the wheel` ·
`wet clay under the hands` · `a continuous unbroken stroke` ·
`water spiralling on the wheelhead` · `molten glaze catching the light` ·
`the rim rising in one motion`

These work because in pottery the flow is **literal** — glaze flows, slip pours,
the wheel spins. That is a real connection to the name, not a pun stretched over
an image. Prefer them over generic motion words (`dynamic`, `flowing lines`,
`swoosh`), which pull toward the fintech clichés in §4.

**Palette phrasing**
`deep emerald green and warm terracotta` · `Mexican folk-art colour palette` ·
`muted earth tones on warm off-white`

---

## 4. Universal NEGATIVE list — applies to EVERY prompt

> **Owner approval required before this kit is used.** This section is the one
> the acceptance criteria single out, because it is what keeps the brand out of
> legal and tonal trouble.

Append to every image prompt, verbatim or as the model's negative-prompt field:

```
NEGATIVE: Play-Doh, play doh, modeling compound, plastic tubs or cans of clay,
squishy toy compound, extruded spaghetti clay, primary-colour toy plastic;
bread, dough, baking, bakery, pastry, flour, rolling pin, wheat;
Simpsons, Homer, "D'oh", yellow cartoon characters;
coins, dollar signs, piggy banks, stacks of cash, gold bars, treasure chests;
fintech gradients, glassmorphism, neon purple-to-blue, crypto aesthetics,
stock-photo handshakes, upward-arrow growth clichés;
text, lettering, words, numerals, watermarks, signatures.
```

**Why each block exists — do not quietly drop one:**

1. **Play-Doh / compound** — trademark caution. The clay wink is a *material*
   reference, not a toy-brand one. Owner instruction, and the strongest legal
   reason on this list.
2. **Bread / bakery** — the entire reason the name is "doh" and not "dough" was
   to stay out of baking results. Generating bread imagery re-creates the
   problem the name was designed to avoid.
3. **Simpsons / "D'oh"** — third-party IP, and tonally wrong for a finance tool.
4. **Coins / piggy banks / dollar signs** — the visual language of every generic
   finance product. Using it forfeits the differentiation the pottery metaphor buys.
5. **Fintech gradients** — same reason, current decade.
6. **Text in generated images** — models render lettering badly, and our wordmark
   is typeset in Nunito by hand. Never let a model attempt it.

---

## 5. Per-asset templates

### 5.1 Logo mark — Recraft V4/V4.1 **Vector**

**The only tool for this is Recraft**, because it outputs true SVG. Raster
upscaled to "vector" is not vector and will fail at 16 px.

```
A minimal flat vector logo mark for a personal finance app called DohFlow.
Single colour, deep emerald green (#006341), on transparent background.
[CONCEPT: e.g. "a hand-thrown clay vessel whose rim becomes a rising line" /
"a potter's wheel seen from above as a concentric flow" /
"a simple ceramic bowl formed from a single continuous stroke"]
Geometric, confident, no gradients, no shadows, no outline strokes of varying
width. Reads clearly at 16 pixels. Square 1:1 composition, centred, generous
margin. NO TEXT.
NEGATIVE: <§4 list>
```

- **Aspect 1:1**, transparent, SVG out.
- Generate **40–60 concepts**, shortlist **3**, then test each at **16 px and
  1024 px** before choosing. A mark that only works large is not a logo.
- Hand-authored geometric SVG is an equally valid path — several strong marks
  are faster to draw than to prompt.

### 5.2 Wordmark — **NEVER GENERATED**

Typeset in **Nunito** by Claude Code (ADR 0063). Adjust tracking optically, not numerically.
Lockups (mark + wordmark, horizontal and stacked) are assembled by hand.
**If a model is asked to render "DohFlow" as text, that is a mistake — stop.**

### 5.3 App icon — macOS 26 Liquid Glass + legacy `.icns`

Two paths, neither a generation task:

- **Legacy `.icns` + PNG ladder** — hand-composed `apps/desktop/src-tauri/icons/source/dohflow-icon.svg` (Apple classic grid, paper plate, mark verbatim from `mark.svg`), regenerated in place with `pnpm -C apps/desktop tauri icon src-tauri/icons/source/dohflow-icon.svg`. Independent of the rename (`4d8.28.2`, see the PROVENANCE row).
- **Liquid Glass `.icon`** — built in Icon Composer on macOS 26 from the final mark. Lands *after* the code rename (`fkt5.3`) and never blocks the flip (`n76x.3.4`).

### 5.4 Hero and scenario scenes — Nano Banana Pro (raster is fine)

```
A soft 3D claymation-style still of [SCENE]. Hand-thrown clay and talavera
ceramic in deep emerald green and warm terracotta, matte glaze, warm studio
light from the upper left, shallow depth of field, generous negative space,
muted warm off-white background. Editorial illustration, single subject,
no text.
NEGATIVE: <§4 list>
```

The three scenario scenes map to the metaphor — keep them recognisably a set:

| Scene | Subject |
|---|---|
| **Sculpt** | Hands shaping wet clay on a wheel — composing a scenario |
| **Kiln** | A warm kiln, glow spilling out, a vessel inside — applying it |
| **Pottery** | The finished glazed piece on a shelf, calm and settled — the outcome |

- **16:9, 2K** for hero and section imagery.
- ~$0.13/image. Budget ~$8 of API credit for the set including rejects.

### 5.5 Derivative sizes

| Asset | Size | Notes |
|---|---|---|
| Favicon | 16/32/48 px `.ico` + `icon.svg` | SVG needs a `prefers-color-scheme` swap |
| Apple touch icon | 180×180 | White mark on emerald, rounded |
| OG card | 1200×630 | Wordmark + tagline + accent rule. **Must not be mostly empty** |
| GitHub social preview | 1280×640 | |
| README header | 1280×320 | |
| DMG background | 1320×800 | Arrow to Applications, mark top-left |
| Product Hunt thumbnail | 240×240 | Must read at thumbnail size |
| Product Hunt gallery | 1270×760 | |

---

## 6. Reference-image chaining

Once one scene is on-brief, **stop describing the style and start showing it.**

1. Lock the first accepted scene as the **style anchor**.
2. For each subsequent scene, pass the anchor as a reference image and describe
   **only what changes** — the subject and composition, not the material,
   lighting, or palette.
3. Keep the §4 negative list on every call regardless; reference images do not
   suppress unwanted content.
4. If the set drifts, re-anchor to the original rather than to the most recent
   output — drift compounds.
5. Record which anchor produced which asset in `PROVENANCE.md`.

---

## 7. Provenance

**Every accepted asset gets a row in [`PROVENANCE.md`](./PROVENANCE.md)** before
it ships. Log the tool and version, the prompt, the reference chain, and — most
importantly — **what the human changed**.

That last column is the point. Prompts alone earn no copyright; the human
selection and edit is what there is to record. It also makes a freelancer
handoff or a trademark question answerable a year from now.

---

## 8. Freelancer brief (only after the go decision)

Capped at **USD 300**, and only for vector polish or the layered macOS icon —
never for concept work, which we do in-house.

> **DohFlow — vector polish brief.**
> DohFlow is a free, open-source, local-first personal finance app for macOS.
> Attached: our chosen logo concept as SVG, the palette, and the wordmark set in
> Nunito.
> **Scope:** refine the curves and optical balance of the mark; deliver clean SVG
> with correct path direction and no stray nodes; supply 16 px and 1024 px proofs;
> plus horizontal and stacked lockups.
> **Out of scope:** new concepts, the wordmark (typeset, do not alter), colour
> changes.
> **Brand notes:** the mark references hand-thrown pottery and talavera ceramic —
> a material reference, not a toy one. No Play-Doh adjacency, no bread or bakery
> imagery, no coins or piggy banks.
> **Deliverables:** source file, optimised SVG, PNG proofs.
> **Rights:** full assignment of all rights to the client, in writing, before payment.

That last line is not boilerplate — without written assignment the project does
not own its own logo.

---

## 9. Verification — the dry run

This kit is not done until it has been proven to work **without its author**.

1. A **separate session**, given *only* this file, generates a small batch of
   concepts for one asset.
2. The **owner marks which are on-brief.**
3. **Pass condition: at least 3 outputs marked on-brief**, and the owner has
   approved §4.
4. Record the result — including failures and what was missing — in
   `personal-cfo-n76x.3.2`'s notes. A failed dry run is a finding about this
   file, not about the model.
