# Recraft session 3 — the illustration set (`n76x.3.6`)

Four images: one hero, three scenes. Copy-paste ready.

## Settings — identical for all four

| | |
|---|---|
| **Model** | **V4 Styles** (reference-image based) |
| **Variant** | **Standard** (raster). Not Vector, not Utility. |
| **Style refs** | `mark.svg` + your two "clay-like" explorations (1–10 allowed) |
| **Aspect ratio** | **16:9** |
| **Resolution** | **2K** (use Pro variant if 2K is not offered on Standard) |
| **Per prompt** | Generate **10–20**, keep **one** |

**Do step 0 first.** Styles → Create style → V4 → upload the references → name it
`DohFlow clay`. Then generate all four under that style. Prompting the four
independently is how a set ends up as four unrelated pictures.

Not V4.1 — it leans photorealistic. V4's stylisation is what this brief wants.

---

## Negative prompt — paste into every one

```
Play-Doh, modeling compound, plastic tub of clay, primary-colour toy plastic, bread, dough, baking, pastry, flour, rolling pin, wheat, Simpsons, yellow cartoon character, coins, dollar signs, piggy bank, stacks of cash, gold bars, fintech gradient, glassmorphism, neon purple, crypto aesthetic, upward arrow, handshake, text, lettering, words, numerals, watermark, signature, glossy, high shine, specular highlights
```

---

## 1 · HERO — homepage

```
A soft 3D claymation still of two hands lifting the rim of a tall ceramic vessel on a spinning potter's wheel, the clay wall rising in one continuous unbroken upward motion. The vessel is warm terracotta clay with a deep emerald green glaze pooling and running down its outer wall. Visible thumb marks in the wet clay. Warm studio lighting with a soft directional key light from the upper left, matte glaze finish, subtle subsurface scattering, shallow depth of field. The vessel and hands sit in the right two thirds of the frame, with large empty warm off-white negative space filling the left third. Muted earth tones, Mexican folk-art colour palette of deep emerald green and warm terracotta. Editorial illustration, single subject, clean uncluttered background.
```

Left third stays empty — the headline goes there.

---

## 2 · SCENE A — `/features/forecast-and-scenarios`

```
A soft 3D claymation still of a pair of hands shaping a wide, shallow clay bowl on a spinning potter's wheel, the form still open and unfinished, water spiralling across the wheelhead. Hand-thrown warm terracotta earthenware with deep emerald green glaze beginning to pool in the base. Visible thumb marks and ridges in the wet clay. Warm studio lighting, soft directional key light from the upper left, matte finish, subtle subsurface scattering, shallow depth of field. Single subject centred on a warm off-white background with generous negative space. Mexican folk-art colour palette of deep emerald green and warm terracotta, muted earth tones. Editorial illustration.
```

---

## 3 · SCENE B — `/features/vault-and-backups`

```
A soft 3D claymation still of a single finished talavera ceramic vessel resting inside a warm kiln, glowing gently from the surrounding heat. Kiln-fired earthenware with a matte glaze in deep emerald green and warm terracotta, hand-painted Mexican pottery motifs across its surface. Warm interior firelight from below and behind, soft shadows, subtle subsurface scattering, matte finish. Isometric three-quarter view, single subject centred, generous negative space, warm off-white surround beyond the kiln opening. Muted earth tones, Mexican folk-art colour palette. Editorial illustration.
```

---

## 4 · SCENE C — `/features/money-inbox-import`

```
A soft 3D claymation still of small square unglazed terracotta tiles being sorted into neat, orderly rows on a warm off-white workbench, with a handful of tiles still scattered loosely at the right edge of the frame. Saltillo tile patterns, matte unglazed clay surfaces, a few tiles glazed in deep emerald green among the terracotta. Warm studio lighting with a soft directional key light from the upper left, soft shadows, shallow depth of field, matte finish. Isometric three-quarter view, generous negative space above the tiles. Muted earth tones, Mexican folk-art colour palette. Editorial illustration.
```

---

## Reject and regenerate if you see

- Anything on the negative list — especially a **tub or can** of clay
- **Text or numerals** — models fake them as glaze marks on pottery
- **Wrong finger counts** — the risk in prompts 1 and 2
- **Gloss or specular highlights** — matte throughout
- Palette drifted to orange, teal or mustard
- A busy background where site text needs to go
- Anything that reads as a photograph rather than an illustration

## When you have your four

Send them over. I verify the palette against `design-tokens.json`, deliver
PNG 2400 px + WebP into `dohflow-site/src/assets/illustrations/`, and log the
prompts in `PROVENANCE.md` recording that you made the selection.
