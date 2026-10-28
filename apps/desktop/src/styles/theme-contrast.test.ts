/**
 * Dark-mode token guard (personal-cfo-17u1): every `:root` custom property has a
 * `.dark` counterpart (except a small, deliberate, named allowlist — see each
 * exception's own doc comment in globals.css), and the text/semantic token pairs clear
 * WCAG contrast in BOTH modes. Distinct from `chart-palette.test.ts`, which guards the
 * categorical `--chart-*` slots' colour-vision-deficiency separation specifically —
 * this file is about parity between the two theme blocks and plain contrast, not CVD.
 */

import globalsCss from "virtual:globals-css-raw";
// The component sources, so the badge checks below read the classes the app ACTUALLY ships
// (same reason as globals.css above: a test carrying its own copy of the markup would keep passing
// while the app rendered something else).
import sourceTree from "virtual:source-tree";

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

// ── personal-cfo-4d8.35: semantic badge text and form-control boundaries ──────────────────────
//
// The audit measured, as RENDERED in both themes on both supported surfaces (--background and
// --card), that coloured text on its own 10% tint was below 4.5:1 for 12px text (light, on
// --background: gain 4.36, warning 3.14, info 4.32, loss 4.30) and that the control boundary
// (--input) was 1.27:1 light / 1.46:1 dark. The treatment: semantic Badge labels are neutral
// foreground text with a semantic MARK, and --input clears 3:1. These tests pin both, reading the
// real stylesheet and the real badge.tsx.
//
// Source math, not a render: the tint is mixed in sRGB here and in oklab by the browser, which moved
// the measured ratios by about 0.04. Every assertion below therefore has more headroom than that.

const THEMES = [
  { name: "light", block: LIGHT },
  { name: "dark", block: DARK },
] as const;
const SURFACES = ["background", "card"] as const;
const SEMANTIC = ["gain", "warning", "info", "loss"] as const;
const NON_TEXT_FLOOR = 3.0;
/// The browser engine these were measured in (Chromium) is not the app's (WKWebView), so every value
/// introduced by 4d8.35 must clear its floor by at least this much (planning's rule). A change that
/// eats the headroom fails even though it would still be 3.0:1 on paper.
const HEADROOM = 0.1;

const BADGE_SRC = sourceTree.find((f) => f.path === "components/ui/badge.tsx")?.text ?? "";

/// `rgb` of a #rrggbb, 0 to 255.
const rgb = (hex: string): [number, number, number] => {
  const h = hex.replace("#", "");
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)) as [number, number, number];
};
/// `fg` at `alpha` over the opaque `bg`, as hex (what a `bg-<token>/10` fill looks like on a surface).
function over(fg: string, bg: string, alpha: number): string {
  const [f, b] = [rgb(fg), rgb(bg)];
  return (
    "#" +
    f
      .map((v, i) => Math.round(v * alpha + b[i]! * (1 - alpha)))
      .map((v) => v.toString(16).padStart(2, "0"))
      .join("")
  );
}
/// The class string of one `badgeVariants` variant, as written in badge.tsx.
function variantClasses(name: string): string {
  const m = new RegExp(`\\b${name}:\\s*"([^"]*)"`).exec(BADGE_SRC);
  return m?.[1] ?? "";
}

describe("semantic badge text and marks (personal-cfo-4d8.35)", () => {
  it("reads the real badge.tsx", () => {
    expect(BADGE_SRC.length).toBeGreaterThan(300); // guards the guard
    for (const name of SEMANTIC) expect(variantClasses(name), name).not.toBe("");
  });

  it("each semantic variant is neutral foreground text, a 10% tint of its token, and a mark of its own token", () => {
    for (const name of SEMANTIC) {
      const classes = variantClasses(name);
      expect(classes, `${name}: label colour`).toContain("text-foreground");
      expect(classes, `${name}: not coloured text`).not.toMatch(new RegExp(`(^|\\s)text-${name}\\b`));
      expect(classes, `${name}: tint`).toMatch(new RegExp(`bg-${name}/10\\b`));
      expect(classes, `${name}: mark`).toContain(`before:bg-${name}`);
    }
  });

  it("keeps the label at 4.5:1 and the mark at 3:1, each plus headroom, against the tint, in both themes on both surfaces", () => {
    let checked = 0;
    for (const theme of THEMES) {
      const foreground = token(theme.block, "foreground")!;
      for (const surfaceName of SURFACES) {
        const surface = token(theme.block, surfaceName)!;
        for (const name of SEMANTIC) {
          const tokenHex = token(theme.block, name)!;
          const tint = over(tokenHex, surface, 0.1);
          const label = contrast(foreground, tint);
          const mark = contrast(tokenHex, tint);
          checked++;
          expect(
            label,
            `${theme.name} ${name} badge label on --${surfaceName} tint = ${label.toFixed(2)}:1 (need ${TEXT_PAIR_FLOOR + HEADROOM}:1)`,
          ).toBeGreaterThanOrEqual(TEXT_PAIR_FLOOR + HEADROOM);
          expect(
            mark,
            `${theme.name} ${name} badge mark on --${surfaceName} tint = ${mark.toFixed(2)}:1 (need ${NON_TEXT_FLOOR + HEADROOM}:1: ${NON_TEXT_FLOOR}:1 plus ${HEADROOM} headroom)`,
          ).toBeGreaterThanOrEqual(NON_TEXT_FLOOR + HEADROOM);
        }
      }
    }
    expect(checked).toBe(THEMES.length * SURFACES.length * SEMANTIC.length); // guards the guard
  });

  it("would FAIL as coloured text: the treatment is needed, not decorative", () => {
    // The pre-fix design: the token itself as the label. Light-mode warning is the clearest case
    // (3.17:1 here, 3.14:1 rendered); if this ever passes, the treatment is no longer load-bearing.
    const surface = token(LIGHT, "background")!;
    const warning = token(LIGHT, "warning")!;
    expect(contrast(warning, over(warning, surface, 0.1))).toBeLessThan(TEXT_PAIR_FLOOR);
  });
});

describe("form-control boundary and focus ring (personal-cfo-4d8.35)", () => {
  it("keeps the control boundary (--input) at 3:1, plus headroom, against every surface a control sits on, in both themes", () => {
    let checked = 0;
    for (const theme of THEMES) {
      const boundary = token(theme.block, "input")!;
      for (const surfaceName of SURFACES) {
        const surface = token(theme.block, surfaceName)!;
        const ratio = contrast(boundary, surface);
        checked++;
        expect(
          ratio,
          `${theme.name}: --input on --${surfaceName} = ${ratio.toFixed(2)}:1 (need ${NON_TEXT_FLOOR + HEADROOM}:1: ${NON_TEXT_FLOOR}:1 plus ${HEADROOM} headroom)`,
        ).toBeGreaterThanOrEqual(NON_TEXT_FLOOR + HEADROOM);
      }
    }
    expect(checked).toBe(THEMES.length * SURFACES.length);
  });

  it("keeps the focus ring (--ring) at 3:1 against both surfaces, in both themes", () => {
    for (const theme of THEMES) {
      const ring = token(theme.block, "ring")!;
      for (const surfaceName of SURFACES) {
        const ratio = contrast(ring, token(theme.block, surfaceName)!);
        expect(
          ratio,
          `${theme.name}: --ring on --${surfaceName} = ${ratio.toFixed(2)}:1 (need ${NON_TEXT_FLOOR + HEADROOM}:1)`,
        ).toBeGreaterThanOrEqual(NON_TEXT_FLOOR + HEADROOM);
      }
    }
  });

  it("leaves the decorative --border (cards, dividers) alone: only controls need a 3:1 boundary", () => {
    // Documents the scope: --border is a divider, not what identifies a control. If this ever
    // starts failing because --border was raised to 3:1, that was a decision, not an accident.
    const ratio = contrast(token(LIGHT, "border")!, token(LIGHT, "background")!);
    expect(ratio).toBeLessThan(NON_TEXT_FLOOR);
  });
});

describe("hover state of the shared controls (personal-cfo-4d8.35)", () => {
  // The Studio's review caught that `NativeSelect`'s `hover:border-ring/60` overrode the improved
  // `--input` and rendered at 2.05:1 Light / 2.74:1 Dark: a token test that never looks at the HOVER
  // class cannot see that. So read every shared control's source, find any `hover:border-*` override,
  // composite it the way the browser paints it (over the control's own `--background` fill), and hold
  // it to the same 3:1 plus headroom as the resting boundary.
  const SHARED = [
    "components/ui/input.tsx",
    "components/ui/native-select.tsx",
    "components/ui/combobox.tsx",
    "components/ui/pagination.tsx",
    "components/ui/emoji-picker.tsx",
  ];
  const text = (path: string) => sourceTree.find((f) => f.path === path)?.text ?? "";

  it("reads the real shared control sources", () => {
    for (const path of SHARED) expect(text(path).length, path).toBeGreaterThan(200); // guards the guard
  });

  it("keeps every hover border override on a shared control at 3:1 plus headroom, in both themes on both surfaces", () => {
    let overrides = 0;
    for (const path of SHARED) {
      for (const m of text(path).matchAll(/hover:border-([a-z-]+)(?:\/(\d+))?/g)) {
        overrides++;
        const [, name, percent] = m;
        for (const theme of THEMES) {
          const fill = token(theme.block, "background")!;
          const hoverBorder = over(token(theme.block, name!)!, fill, percent ? Number(percent) / 100 : 1);
          for (const surfaceName of SURFACES) {
            const surface = token(theme.block, surfaceName)!;
            for (const [what, adjacent] of [["outside", surface], ["fill", fill]] as const) {
              const ratio = contrast(hoverBorder, adjacent);
              expect(
                ratio,
                `${path}: hover:border-${name}${percent ? `/${percent}` : ""} on ${theme.name} --${surfaceName} (${what}) = ${ratio.toFixed(2)}:1 (need ${NON_TEXT_FLOOR + HEADROOM}:1)`,
              ).toBeGreaterThanOrEqual(NON_TEXT_FLOOR + HEADROOM);
            }
          }
        }
      }
    }
    // NativeSelect is the one shared control with a hover border today. If it stops having one this
    // guard has nothing to hold, which is fine, but it must not silently find LESS than it should.
    expect(overrides).toBeGreaterThanOrEqual(1);
  });

  it("would have FAILED the 60% hover the Studio caught (the guard is load-bearing)", () => {
    const fill = token(LIGHT, "background")!;
    const old = over(token(LIGHT, "ring")!, fill, 0.6);
    expect(contrast(old, fill)).toBeLessThan(NON_TEXT_FLOOR);
  });
});
