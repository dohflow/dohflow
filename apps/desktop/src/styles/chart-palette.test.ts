/**
 * Palette guard for the categorical chart slots (ADR 0054, personal-cfo-4d8.27.4.5).
 *
 * The `--chart-*` tokens are not a taste choice — they are a set of colours that has to
 * stay distinguishable, including to a reader with colour-vision deficiency. That is
 * measurable, so it is measured here rather than re-argued in review. Changing a
 * `--chart-*` value in `globals.css` without clearing the gates fails this suite.
 *
 * This exists because the ORIGINAL five-slot palette, chosen by eye, shipped with
 * `--chart-1` (emerald) and `--chart-2` (terracotta) collapsing to ΔE 1.6 under
 * protanopia in dark mode — and `DebtBurndownChart` painted avalanche and snowball with
 * exactly that pair. Nobody caught it for months because nothing checked.
 *
 * The thresholds AND the CVD simulation model live here on purpose (ADR 0054 §5): the
 * standard belongs in the repo, not in whichever tool happened to be installed the day
 * the palette was written. ΔE is Euclidean distance in OKLab ×100 throughout; the CVD
 * thresholds are calibrated to the Machado–Oliveira–Fernandes (2009) transforms at
 * severity 1.0 below, so swapping the model would mean recalibrating the numbers.
 */

// Served by the `globals-css-raw` plugin in `vite.config.ts`. Vitest stubs CSS imports
// to an empty string, and that stub beats `?raw` / `?inline` / `import.meta.glob` — so
// the obvious import would hand this guard an EMPTY palette and every check below would
// pass vacuously. See the plugin for the full explanation.
import globalsCss from "virtual:globals-css-raw";

// ── thresholds ────────────────────────────────────────────────────────────────
const BAND = { light: [0.43, 0.77], dark: [0.48, 0.67] } as const; // OKLCH L
const CHROMA_FLOOR = 0.1; // OKLCH C — below this a hue reads as gray
const CVD_TARGET = 8.0; // adjacent pairs, min(protan, deutan)
const NORMAL_FLOOR = 15.0; // adjacent pairs, unsimulated vision
const CONTRAST_MIN = 3.0; // WCAG vs the surface the marks sit on
/// Charts render inside a `Card`, so the surface is `--card`, not `--background`.
const SURFACE = { light: "#ffffff", dark: "#151c18" } as const;

// Machado, Oliveira & Fernandes (2009) CVD transforms at severity 1.0 (linear RGB).
const MACHADO = {
  protan: [
    [0.152286, 1.052583, -0.204868],
    [0.114503, 0.786281, 0.099216],
    [-0.003882, -0.048116, 1.051998],
  ],
  deutan: [
    [0.367322, 0.860646, -0.227968],
    [0.280085, 0.672501, 0.047413],
    [-0.01182, 0.04294, 0.968881],
  ],
} as const;

// ── colour conversions ────────────────────────────────────────────────────────
const s2lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const lin = (hex: string): number[] =>
  [0, 2, 4].map((i) => s2lin(parseInt(hex.slice(1 + i, 3 + i), 16) / 255));

function oklabFromLin([r, g, b]: number[]): [number, number, number] {
  const l = Math.cbrt(0.4122214708 * r! + 0.5363325363 * g! + 0.0514459929 * b!);
  const m = Math.cbrt(0.2119034982 * r! + 0.6806995451 * g! + 0.1073969566 * b!);
  const s = Math.cbrt(0.0883024619 * r! + 0.2817188376 * g! + 0.6299787005 * b!);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}
const oklch = (hex: string): [number, number] => {
  const [L, a, b] = oklabFromLin(lin(hex));
  return [L, Math.hypot(a, b)];
};
const relLum = (hex: string) => {
  const [r, g, b] = lin(hex);
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
};
const contrast = (a: string, b: string) => {
  const [hi, lo] = [relLum(a), relLum(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
};
function simulate(hex: string, kind: keyof typeof MACHADO): number[] {
  const [r, g, b] = lin(hex);
  const M = MACHADO[kind];
  return M.map((row) =>
    Math.max(0, Math.min(1, row[0]! * r! + row[1]! * g! + row[2]! * b!)),
  );
}
/// Euclidean distance in OKLab ×100. No `kind` → unsimulated (normal) vision.
function deltaE(h1: string, h2: string, kind?: keyof typeof MACHADO): number {
  const a = oklabFromLin(kind ? simulate(h1, kind) : lin(h1));
  const b = oklabFromLin(kind ? simulate(h2, kind) : lin(h2));
  return 100 * Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}

// ── the tokens, read from the stylesheet that actually ships ──────────────────
/// Pull `--chart-N` out of a block of `globals.css`. Reading the real file (rather than
/// restating the hexes here) is the point: a test with its own copy of the palette would
/// pass while the app shipped something else.
function chartTokens(block: string): string[] {
  const out: string[] = [];
  for (let i = 1; ; i++) {
    const m = new RegExp(`--chart-${i}:\\s*(#[0-9a-fA-F]{6})`).exec(block);
    if (!m) break;
    out.push(m[1]!.toLowerCase());
  }
  return out;
}
const rootBlock = /:root\s*\{([\s\S]*?)\n\}/.exec(globalsCss)?.[1] ?? "";
const darkBlock = /\.dark\s*\{([\s\S]*?)\n\}/.exec(globalsCss)?.[1] ?? "";
const PALETTE = { light: chartTokens(rootBlock), dark: chartTokens(darkBlock) };

describe("categorical chart palette (ADR 0054)", () => {
  it("parses four slots from globals.css in both modes", () => {
    // Guards the guard: a regex that silently matched nothing would make every check
    // below vacuously pass.
    expect(PALETTE.light).toHaveLength(4);
    expect(PALETTE.dark).toHaveLength(4);
    // ADR 0054 §1 — the fifth slot was removed, not renamed.
    expect(globalsCss).not.toContain("--chart-5");
  });

  for (const mode of ["light", "dark"] as const) {
    describe(mode, () => {
      const palette = PALETTE[mode];
      const surface = SURFACE[mode];

      it("keeps every slot inside the lightness band", () => {
        const [lo, hi] = BAND[mode];
        const offBand = palette.filter((c) => {
          const L = oklch(c)[0];
          return L < lo || L > hi;
        });
        expect(offBand, `outside L ${lo}–${hi}`).toEqual([]);
      });

      it("keeps every slot above the chroma floor", () => {
        // Below the floor a hue reads as gray and stops doing identity work at all.
        const gray = palette.filter((c) => oklch(c)[1] < CHROMA_FLOOR);
        expect(gray, `below C ${CHROMA_FLOOR}`).toEqual([]);
      });

      it("separates adjacent slots under protanopia and deuteranopia", () => {
        // THE check. Adjacent pairs are what a stacked bar or a multi-line chart puts
        // side by side, and slot order is the safety mechanism — which is why ADR 0054
        // forbids reshuffling the order per chart.
        for (let i = 0; i < palette.length - 1; i++) {
          const [a, b] = [palette[i]!, palette[i + 1]!];
          const worst = Math.min(deltaE(a, b, "protan"), deltaE(a, b, "deutan"));
          expect(
            worst,
            `${a} ↔ ${b} ΔE ${worst.toFixed(1)} — indistinguishable to a colourblind reader`,
          ).toBeGreaterThanOrEqual(CVD_TARGET);
        }
      });

      it("separates adjacent slots for full-colour vision too", () => {
        for (let i = 0; i < palette.length - 1; i++) {
          const [a, b] = [palette[i]!, palette[i + 1]!];
          const d = deltaE(a, b);
          expect(d, `${a} ↔ ${b} ΔE ${d.toFixed(1)}`).toBeGreaterThanOrEqual(
            NORMAL_FLOOR,
          );
        }
      });

      it("holds 3:1 against the card surface the charts sit on", () => {
        for (const c of palette) {
          const ratio = contrast(c, surface);
          expect(ratio, `${c} on ${surface} = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(
            CONTRAST_MIN,
          );
        }
      });
    });
  }

  it("catches a regression to the palette this ADR replaced", () => {
    // The shipped dark pair that motivated ADR 0054: emerald vs terracotta, ΔE 1.6 under
    // protanopia. If someone reverts the tokens, the checks above must fail — this
    // asserts the measurement itself still detects it, so the guard cannot rot into
    // something that passes everything.
    const worst = Math.min(
      deltaE("#12a06b", "#db8f6b", "protan"),
      deltaE("#12a06b", "#db8f6b", "deutan"),
    );
    expect(worst).toBeLessThan(CVD_TARGET);
  });
});
