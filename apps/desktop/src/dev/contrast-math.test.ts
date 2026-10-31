import {
  clears,
  contrastRatio,
  HEADROOM,
  NON_TEXT_FLOOR,
  overAlpha,
  relativeLuminance,
  solidRings,
  TEXT_FLOOR,
  toHex,
  type Rgb,
} from "./contrast-math";

const WHITE: Rgb = [255, 255, 255];
const BLACK: Rgb = [0, 0, 0];

describe("contrast math (personal-cfo-4d8.35)", () => {
  it("matches the WCAG reference values", () => {
    expect(contrastRatio(WHITE, BLACK)).toBeCloseTo(21, 5);
    expect(contrastRatio(WHITE, WHITE)).toBeCloseTo(1, 5);
    // #777777 on white is the textbook "just under 4.5:1" gray.
    expect(contrastRatio([119, 119, 119], WHITE)).toBeCloseTo(4.48, 2);
    expect(contrastRatio([118, 118, 118], WHITE)).toBeGreaterThan(TEXT_FLOOR);
  });

  // The sRGB transfer function has TWO branches (a short linear segment under 0.04045 and a power law
  // above it). The first cases above use channels 0, 118, 119, 128 and 255, so a wrong threshold between
  // the branches (a typo like 0.4045) passed them all. These channels sit in the range a wrong threshold
  // would misread, and the ratios are the ones the browser itself produced in the rendered audit.
  it("handles the mid-dark channels where the two sRGB branches meet", () => {
    expect(contrastRatio([64, 64, 64], WHITE)).toBeCloseTo(10.37, 2);
    // Dark theme: the control boundary on the page background (rendered 3.35 in the audit).
    expect(contrastRatio([0x5f, 0x6b, 0x63], [0x0e, 0x14, 0x11])).toBeCloseTo(3.35, 2);
    // ...and on the dark card (rendered 3.11).
    expect(contrastRatio([0x5f, 0x6b, 0x63], [0x15, 0x1c, 0x18])).toBeCloseTo(3.11, 2);
  });

  it("uses the LINEAR branch for near-black channels (the dark theme's page background is near black)", () => {
    // Channel 3 is under the 0.04045 break, so it is divided by 12.92; a break set too low (0.0045)
    // would push it through the power law and read 1.0266 here.
    expect(contrastRatio([3, 3, 3], BLACK)).toBeCloseTo(1.0182, 3);
    expect(contrastRatio([6, 6, 6], BLACK)).toBeCloseTo(1.0364, 3);
  });

  it("matches the rendered focus rings and control boundaries in both themes", () => {
    // Light `--ring` on `--background` / `--card`, and Dark `--ring` on each (all measured, Chromium 152).
    expect(contrastRatio([0x2e, 0x94, 0x70], [0xfa, 0xf8, 0xf6])).toBeCloseTo(3.55, 2);
    expect(contrastRatio([0x2e, 0x94, 0x70], WHITE)).toBeCloseTo(3.76, 2);
    expect(contrastRatio([0x12, 0xa0, 0x6b], [0x0e, 0x14, 0x11])).toBeCloseTo(5.56, 2);
    expect(contrastRatio([0x12, 0xa0, 0x6b], [0x15, 0x1c, 0x18])).toBeCloseTo(5.17, 2);
    // Light `--input` on `--background` (3.12) and `--card` (3.31).
    expect(contrastRatio([0x94, 0x8c, 0x84], [0xfa, 0xf8, 0xf6])).toBeCloseTo(3.12, 2);
    expect(contrastRatio([0x94, 0x8c, 0x84], WHITE)).toBeCloseTo(3.31, 2);
  });

  it("is symmetric and orders luminance", () => {
    const a: Rgb = [20, 40, 60];
    const b: Rgb = [200, 180, 160];
    expect(contrastRatio(a, b)).toBeCloseTo(contrastRatio(b, a), 10);
    expect(relativeLuminance(BLACK)).toBe(0);
    expect(relativeLuminance(WHITE)).toBeCloseTo(1, 10);
  });

  it("mixes in sRGB, rounding to whole channels", () => {
    expect(overAlpha(BLACK, WHITE, 0.5)).toEqual([128, 128, 128]);
    expect(overAlpha(BLACK, WHITE, 0)).toEqual(WHITE);
    expect(overAlpha(BLACK, WHITE, 1)).toEqual(BLACK);
    expect(toHex([15, 122, 84])).toBe("#0f7a54");
  });

  it("reads the solid rings out of a computed box-shadow and ignores elevation", () => {
    const real =
      "rgb(250, 248, 246) 0px 0px 0px 2px, rgb(46, 148, 112) 0px 0px 0px 4px, rgba(0, 0, 0, 0.1) 0px 1px 3px 0px";
    expect(solidRings(real)).toEqual([
      { color: [250, 248, 246], spread: 2 },
      { color: [46, 148, 112], spread: 4 },
    ]);
    expect(solidRings("none")).toEqual([]);
    expect(solidRings("rgba(0, 0, 0, 0) 0px 0px 0px 0px")).toEqual([]);
  });

  it("requires the floor plus headroom", () => {
    expect(HEADROOM).toBe(0.1);
    expect(clears(3.1, NON_TEXT_FLOOR)).toBe(true);
    expect(clears(3.09, NON_TEXT_FLOOR)).toBe(false);
    expect(clears(4.6, TEXT_FLOOR)).toBe(true);
    expect(clears(4.5, TEXT_FLOOR)).toBe(false);
  });
});
