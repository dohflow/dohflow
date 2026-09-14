import { describe, expect, it } from "vitest";

import globalsCss from "virtual:globals-css-raw";

import {
  BAND_EDGE_DASH,
  BAND_EDGE_OPACITY,
  BAND_FILL_FLOOR,
  BAND_FILL_OPACITY,
} from "./bandLegibility";

/// The uncertainty band stays visible against the surfaces it is actually drawn on
/// (personal-cfo-s0iz).
///
/// Computed, not eyeballed. The band is how the projected half declares itself uncertain;
/// if it washes out the chart silently becomes a confident line. So this composites the
/// band over the SHIPPED `--card` of each theme using the SHIPPED `--chart-*` tokens and
/// measures the result — which means darkening a surface, dulling a token, or lowering the
/// opacity fails here rather than quietly erasing the band.

/// The theme blocks in globals.css. `:root` is light; `.dark` overrides it.
function themeBlock(css: string, selector: string): string {
  const at = css.indexOf(selector);
  if (at < 0) return "";
  const open = css.indexOf("{", at);
  const close = css.indexOf("}", open);
  return css.slice(open, close);
}

function token(block: string, name: string): string | null {
  const m = block.match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{3,8})`));
  return m?.[1] ?? null;
}

function toRgb(hex: string): [number, number, number] {
  const h = hex.replace("#", "");
  const full =
    h.length === 3
      ? h
          .split("")
          .map((c) => c + c)
          .join("")
      : h;
  return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16) / 255) as [
    number,
    number,
    number,
  ];
}

const toLinear = (c: number) =>
  c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;

/// sRGB → OKLab. Perceptual, so a distance means the same thing at both ends of the ramp —
/// which plain RGB distance does not give, and is why a light-mode check cannot stand in
/// for a dark-mode one.
function oklab([r, g, b]: [number, number, number]): [number, number, number] {
  const [R, G, B] = [toLinear(r), toLinear(g), toLinear(b)];
  const l = Math.cbrt(0.4122214708 * R + 0.5363325363 * G + 0.0514459929 * B);
  const m = Math.cbrt(0.2119034982 * R + 0.6806995451 * G + 0.1073969566 * B);
  const s = Math.cbrt(0.0883024619 * R + 0.2817188376 * G + 0.6299787005 * B);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function deltaE(a: [number, number, number], b: [number, number, number]) {
  const [A, B] = [oklab(a), oklab(b)];
  return 100 * Math.hypot(A[0] - B[0], A[1] - B[1], A[2] - B[2]);
}

/// What the eye actually receives: a translucent fill composited over the card.
function over(
  fg: [number, number, number],
  bg: [number, number, number],
  alpha: number,
): [number, number, number] {
  return fg.map((c, i) => c * alpha + (bg[i] ?? 0) * (1 - alpha)) as [
    number,
    number,
    number,
  ];
}

const THEMES = [
  { name: "light", block: themeBlock(globalsCss, ":root") },
  { name: "dark", block: themeBlock(globalsCss, ".dark") },
];

describe("forecast band legibility (personal-cfo-s0iz)", () => {
  it("reads the real stylesheet", () => {
    // Guards the guard: Vitest stubs CSS imports to "" by default, and that stub beats
    // ?raw — which is why this file reads a virtual module. Without this assertion every
    // check below would pass against an empty string.
    expect(globalsCss.length).toBeGreaterThan(500);
    for (const theme of THEMES) {
      expect(theme.block, `${theme.name} block found`).toContain("--card:");
      expect(token(theme.block, "chart-1"), `${theme.name} chart-1`).toBeTruthy();
    }
  });

  it("keeps the band fill above the perceptual floor on BOTH surfaces", () => {
    const measured: string[] = [];
    for (const theme of THEMES) {
      const card = token(theme.block, "card");
      expect(card, `${theme.name} --card`).toBeTruthy();
      for (const n of [1, 2, 3, 4]) {
        const chart = token(theme.block, `chart-${n}`);
        expect(chart, `${theme.name} --chart-${n}`).toBeTruthy();
        const surface = toRgb(card!);
        const separation = deltaE(
          over(toRgb(chart!), surface, BAND_FILL_OPACITY),
          surface,
        );
        measured.push(
          `${theme.name} chart-${n}: ${separation.toFixed(2)}`,
        );
        expect(
          separation,
          `${theme.name} --chart-${n} band is too faint to read (${separation.toFixed(2)} < ${BAND_FILL_FLOOR})`,
        ).toBeGreaterThanOrEqual(BAND_FILL_FLOOR);
      }
    }
    // Non-vacuous: eight real measurements, not an empty loop reporting success.
    expect(measured).toHaveLength(8);
  });

  it("draws the edge far stronger than the fill, and dotted", () => {
    // The edge is what survives when the wash is lost against a gradient or a gridline, so
    // it must not merely match the fill.
    expect(BAND_EDGE_OPACITY).toBeGreaterThan(BAND_FILL_OPACITY * 2);
    // Dotted, because a percentile edge is not a promise about where the range ends.
    expect(BAND_EDGE_DASH).toMatch(/^\d+ \d+$/);
  });
});
