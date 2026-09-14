import { describe, expect, it } from "vitest";

import { placeLowAnnotation } from "./labelCollision";

// Defaults exercised throughout: labelHeight=12, gap=6 -> halfLabel=6, so a
// marker at Y=100 candidates at above=88, below=112; collisionThreshold=12.
const MARKER_Y = 100;
const ABOVE = 88;
const BELOW = 112;

describe("placeLowAnnotation (personal-cfo-4d8.29)", () => {
  it("clear: no ticks nearby at all -> uses the preferred side untouched", () => {
    const result = placeLowAnnotation(MARKER_Y, "below", []);
    expect(result).toEqual({ y: BELOW });
  });

  it("clear: a tick exists but far from either candidate -> preferred side, untouched", () => {
    const result = placeLowAnnotation(MARKER_Y, "below", [{ value: 30_000, y: 50 }]);
    expect(result).toEqual({ y: BELOW });
  });

  it("near (preferred side collides, opposite is clear) -> flips to the opposite side", () => {
    // Tick sits 4px from BELOW (collides, <=12) but 20px from ABOVE (clear).
    const result = placeLowAnnotation(MARKER_Y, "below", [{ value: 30_000, y: 108 }]);
    expect(result).toEqual({ y: ABOVE });
  });

  it("near, mirrored (preferred side is 'above') -> flips to 'below'", () => {
    // Tick sits 4px from ABOVE (collides) but 20px from BELOW (clear).
    const result = placeLowAnnotation(MARKER_Y, "above", [{ value: 30_000, y: 92 }]);
    expect(result).toEqual({ y: BELOW });
  });

  it("on-tick: a single tick sits exactly on the marker -> both sides collide, hides that tick", () => {
    // The literal reported bug: the low value's pixel position coincides
    // with a gridline, so BOTH the above and below candidates (each 12px
    // from the marker) are exactly 12px from the one tick that sits AT the
    // marker's own Y — neither side is clear, and there is only one tick to
    // blame.
    const result = placeLowAnnotation(MARKER_Y, "below", [{ value: 30_000, y: MARKER_Y }]);
    expect(result).toEqual({ y: BELOW, hiddenTickValue: 30_000 });
  });

  it("both sides collide with DIFFERENT ticks -> stays on the preferred side, hides only that side's tick", () => {
    const result = placeLowAnnotation(MARKER_Y, "below", [
      { value: 40_000, y: BELOW }, // exactly blocks the preferred (below) candidate
      { value: 20_000, y: ABOVE }, // exactly blocks the fallback (above) candidate
    ]);
    // Preferred side ("below") is tried first, collides with the 40_000
    // tick; the flip to "above" also collides (with the 20_000 tick), so we
    // fall back to the preferred side and blame ITS tick, not the other one.
    expect(result).toEqual({ y: BELOW, hiddenTickValue: 40_000 });
  });

  it("boundary: exactly at the collision threshold counts as a collision", () => {
    // BELOW+12 = 124 is exactly threshold-distance from BELOW (collides) and
    // 36px from ABOVE (clearly clear) — an unambiguous single-sided case,
    // unlike BELOW-12 which happens to also sit exactly threshold-distance
    // from ABOVE (both symmetric around MARKER_Y with the same offset).
    const result = placeLowAnnotation(MARKER_Y, "below", [{ value: 1, y: BELOW + 12 }]);
    expect(result).toEqual({ y: ABOVE });
  });

  it("boundary: one pixel past the threshold does not collide", () => {
    const result = placeLowAnnotation(MARKER_Y, "below", [{ value: 1, y: BELOW + 13 }]);
    expect(result).toEqual({ y: BELOW });
  });

  it("respects custom labelHeight/gap/collisionThreshold options", () => {
    // gap=10, labelHeight=20 -> halfLabel=10 -> below = 100+10+10 = 120.
    const result = placeLowAnnotation(MARKER_Y, "below", [{ value: 1, y: 125 }], {
      gap: 10,
      labelHeight: 20,
      collisionThreshold: 4,
    });
    // |125-120| = 5 > 4 -> no collision under this tighter threshold.
    expect(result).toEqual({ y: 120 });
  });
});
