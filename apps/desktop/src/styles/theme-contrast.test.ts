/**
 * Dark-mode token guard (personal-cfo-17u1): every `:root` custom property has a
 * `.dark` counterpart (except a small, deliberate, named allowlist — see each
 * exception's own doc comment in globals.css), and the text/semantic token pairs clear
 * WCAG contrast in BOTH modes. Distinct from `chart-palette.test.ts`, which guards the
 * categorical `--chart-*` slots' colour-vision-deficiency separation specifically —
 * this file is about parity between the two theme blocks and plain contrast, not CVD.
 */

import globalsCss from "virtual:globals-css-raw";

// ── allowlists — both intentional, both documented at the source ──────────────
/// `:root` tokens that deliberately do NOT flip with the theme — see each one's own
/// doc comment in globals.css for why. Kept here as a named, exhaustive list (not "any
/// mismatch is fine") so an ACCIDENTALLY forgotten `.dark` value still fails loudly.
const PARITY_ALLOWLIST = new Set(["radius", "on-brand-fill", "brand-white"]);

/// Chart/semantic tokens whose LIGHT-mode contrast against `--card` falls short of the
/// 3:1 floor. Empty today: `--warning` was the one member (2.9463:1, a hue too close in
/// luminance to the light theme's near-white surfaces) until personal-cfo-2qonl moved it
/// to #b3780a (3.7448:1 against --card, 3.5349:1 against --background — computed, not
/// guessed; same hue, ~39° — see globals.css's own comment on that token). Kept as a
/// named Set rather than deleted outright: the "would fail without the exemptions" test
/// below iterates it and is a no-op while it's empty, ready to hold a future genuine gap
/// the same documented way rather than needing new scaffolding.
///
/// `--terracotta` carries an analogous, SEPARATE exemption in
/// `scripts/sync-tokens.mjs`'s own banner ("~2.6:1 on white, decorative only, body text
/// and links use emerald or a neutral") — decorative-only, not a chart/semantic token,
/// so it isn't in `CHART_GROUP` and isn't affected by 2qonl.
///
/// `--chart-2` (also a terracotta-family hue) was considered for 2qonl's fix too — it
/// clears the floor today by a razor-thin 3.0007:1 — but was deliberately left alone:
/// the "would fail without the exemptions" test already proved it does not actually need
/// one, and tightening a token that already passes is optional polish, not a fix for a
/// real gap. Worth revisiting if this margin ever proves fragile in practice.
const LIGHT_CHART_CONTRAST_EXEMPTIONS = new Set<string>([]);

const TEXT_PAIR_FLOOR = 4.5;
const CHART_CONTRAST_FLOOR = 3.0;
/// Charts render inside a `Card` — matching `chart-palette.test.ts`'s own stated
/// rationale ("Charts render inside a Card, so the surface is --card, not
/// --background") — not directly on `--background`.
const CHART_SURFACE_TOKEN = "card";
const CHART_GROUP = [
  "chart-1",
  "chart-2",
  "chart-3",
  "chart-4",
  "gain",
  "loss",
  "warning",
  "info",
];

// ── colour math (WCAG relative luminance + contrast ratio; matches the sRGB→linear
// step every other palette/legibility test in this repo already uses) ─────────────
const s2lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
function relLum(hex: string): number {
  const h = hex.replace("#", "");
  const [r, g, b] = [0, 2, 4].map((i) => s2lin(parseInt(h.slice(i, i + 2), 16) / 255));
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}
function contrast(a: string, b: string): number {
  const [hi, lo] = [relLum(a), relLum(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

// ── read the real stylesheet — same "read the shipped file, don't restate the
// palette" discipline chart-palette.test.ts documents, for the same reason. ────────
function themeBlock(selector: string): string {
  const m = new RegExp(`${selector}\\s*\\{([\\s\\S]*?)\\n\\}`).exec(globalsCss);
  return m?.[1] ?? "";
}
function tokenNames(block: string): string[] {
  const names = new Set<string>();
  const re = /--([\w-]+):/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(block))) names.add(m[1]!);
  return [...names];
}
function token(block: string, name: string): string | null {
  const m = block.match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{3,8})`));
  return m?.[1] ?? null;
}

const LIGHT = themeBlock(":root");
const DARK = themeBlock("\\.dark");

describe("dark-mode token parity (personal-cfo-17u1)", () => {
  it("reads the real stylesheet", () => {
    // Guards the guard: Vitest stubs CSS imports to "" by default, and that stub beats
    // ?raw — without this, every check below would pass against an empty string.
    expect(globalsCss.length).toBeGreaterThan(500);
    expect(LIGHT.length).toBeGreaterThan(200);
    expect(DARK.length).toBeGreaterThan(200);
  });

  it("every :root token has a .dark counterpart, except the named allowlist", () => {
    const light = tokenNames(LIGHT);
    const dark = new Set(tokenNames(DARK));
    expect(light.length).toBeGreaterThan(25); // guards the guard
    const missing = light.filter(
      (name) => !dark.has(name) && !PARITY_ALLOWLIST.has(name),
    );
    expect(
      missing,
      `:root token(s) with no .dark value and not on the allowlist: ${missing.join(", ")}`,
    ).toEqual([]);
  });

  it("has no .dark-only tokens (reverse drift)", () => {
    const light = new Set(tokenNames(LIGHT));
    const dark = tokenNames(DARK);
    const extra = dark.filter((name) => !light.has(name));
    expect(extra, `.dark token(s) with no :root value: ${extra.join(", ")}`).toEqual([]);
  });

  it("the allowlist itself stays exactly the three documented exceptions", () => {
    // If a future edit adds a fourth allowlisted token without updating this constant,
    // the parity test above would silently start ignoring it too — this pins the
    // allowlist's own size so that requires touching this file, not just globals.css.
    expect([...PARITY_ALLOWLIST].sort()).toEqual([
      "brand-white",
      "on-brand-fill",
      "radius",
    ]);
  });
});

describe("theme contrast (personal-cfo-17u1)", () => {
  const THEMES = [
    { name: "light", block: LIGHT },
    { name: "dark", block: DARK },
  ] as const;

  it("keeps every text/foreground pair at 4.5:1 or better in both modes", () => {
    const pairs: [string, string][] = [
      ["foreground", "background"],
      ["muted-foreground", "background"],
      ["card-foreground", "card"],
      ["primary-foreground", "primary"],
    ];
    let checked = 0;
    for (const theme of THEMES) {
      for (const [fg, bg] of pairs) {
        const fgHex = token(theme.block, fg);
        const bgHex = token(theme.block, bg);
        expect(fgHex, `${theme.name} --${fg}`).toBeTruthy();
        expect(bgHex, `${theme.name} --${bg}`).toBeTruthy();
        const ratio = contrast(fgHex!, bgHex!);
        checked++;
        expect(
          ratio,
          `${theme.name}: --${fg} on --${bg} = ${ratio.toFixed(2)}:1 (need ${TEXT_PAIR_FLOOR}:1)`,
        ).toBeGreaterThanOrEqual(TEXT_PAIR_FLOOR);
      }
    }
    expect(checked).toBe(pairs.length * THEMES.length); // guards the guard
  });

  it("keeps chart/semantic tokens at 3:1 or better against the card surface, except the documented light-mode exemptions", () => {
    let checked = 0;
    for (const theme of THEMES) {
      const surface = token(theme.block, CHART_SURFACE_TOKEN);
      expect(surface, `${theme.name} --${CHART_SURFACE_TOKEN}`).toBeTruthy();
      for (const name of CHART_GROUP) {
        if (theme.name === "light" && LIGHT_CHART_CONTRAST_EXEMPTIONS.has(name)) {
          continue;
        }
        const hex = token(theme.block, name);
        expect(hex, `${theme.name} --${name}`).toBeTruthy();
        const ratio = contrast(hex!, surface!);
        checked++;
        expect(
          ratio,
          `${theme.name}: --${name} on --${CHART_SURFACE_TOKEN} = ${ratio.toFixed(2)}:1 (need ${CHART_CONTRAST_FLOOR}:1)`,
        ).toBeGreaterThanOrEqual(CHART_CONTRAST_FLOOR);
      }
    }
    // 8 tokens x 2 themes, minus the light-mode exemption count (0 today,
    // personal-cfo-2qonl) = 16. Guards the guard: a typo in the exemption set
    // or the group list would silently check fewer/more pairs.
    expect(checked).toBe(CHART_GROUP.length * THEMES.length - LIGHT_CHART_CONTRAST_EXEMPTIONS.size);
  });

  it("would fail without the light-mode exemptions — so the exemption is real, not a leftover", () => {
    // A no-op today: LIGHT_CHART_CONTRAST_EXEMPTIONS is empty (personal-cfo-2qonl fixed
    // its one member, --warning), so this loop runs zero iterations. Kept rather than
    // deleted so a FUTURE exemption gets this same enforcement automatically: if some
    // token's light-mode hex ever changes to clear the floor while still listed here,
    // this starts failing (in the GOOD direction) — the signal to delete the exemption
    // and its follow-up bead, not leave a stale allowance in place.
    const surface = token(LIGHT, CHART_SURFACE_TOKEN)!;
    for (const name of LIGHT_CHART_CONTRAST_EXEMPTIONS) {
      const hex = token(LIGHT, name)!;
      const ratio = contrast(hex, surface);
      expect(
        ratio,
        `--${name} no longer needs its light-mode exemption (${ratio.toFixed(4)}:1 >= ${CHART_CONTRAST_FLOOR}) — remove it from LIGHT_CHART_CONTRAST_EXEMPTIONS and close the follow-up bead`,
      ).toBeLessThan(CHART_CONTRAST_FLOOR);
    }
  });
});
