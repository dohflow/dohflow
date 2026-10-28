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
