import { describe, expect, it } from "vitest";

import globalsCss from "virtual:globals-css-raw";

import type { ScenarioDto } from "@/bindings";

import { appliedRowProps } from "./appliedRow";

const scenario = (over: Partial<ScenarioDto> = {}) =>
  ({
    id: "s1",
    name: "Rent hike",
    description: null,
    status: "draft",
    created_at: "2026-08-01",
    updated_at: "2026-08-01",
    expires_on: null,
    event_count: 1,
    applied_at: null,
    ...over,
  }) as ScenarioDto;

/// Perceptual separation of an applied row from a plain one, in OKLab ΔE × 100.
///
/// Lower than the forecast band's floor on purpose. Thresholds depend on the size of the
/// field being judged, and a full table row is a far larger stimulus than a 1px stipple —
/// and unlike the band, the tint is not the load-bearing signal here. The spine is, which
/// is why it is asserted first.
const TINT_FLOOR = 4;

function themeBlock(selector: string): string {
  const at = globalsCss.indexOf(selector);
  const open = globalsCss.indexOf("{", at);
  return globalsCss.slice(open, globalsCss.indexOf("}", open));
}
const token = (block: string, name: string) =>
  block.match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`))?.[1] ?? null;

const toRgb = (hex: string) =>
  [0, 2, 4].map((i) => parseInt(hex.slice(1 + i, 3 + i), 16) / 255) as [
    number,
    number,
    number,
  ];
const toLinear = (c: number) =>
  c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
function oklab([r, g, b]: [number, number, number]) {
  const [R, G, B] = [toLinear(r), toLinear(g), toLinear(b)];
  const l = Math.cbrt(0.4122214708 * R + 0.5363325363 * G + 0.0514459929 * B);
  const m = Math.cbrt(0.2119034982 * R + 0.6806995451 * G + 0.1073969566 * B);
  const s = Math.cbrt(0.0883024619 * R + 0.2817188376 * G + 0.6299787005 * B);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ] as [number, number, number];
}
const deltaE = (a: [number, number, number], b: [number, number, number]) => {
  const [A, B] = [oklab(a), oklab(b)];
  return 100 * Math.hypot(A[0] - B[0], A[1] - B[1], A[2] - B[2]);
};

describe("an applied scenario reads as brand (personal-cfo-kkxu)", () => {
  it("marks only applied rows", () => {
    expect(appliedRowProps(scenario())).toEqual({});
    expect(
      appliedRowProps(scenario({ applied_at: "2026-08-02T10:00:00Z" })).className,
    ).toBeTruthy();
  });

  it("carries a solid brand SPINE — the signal doing the work", () => {
    const { className } = appliedRowProps(
      scenario({ applied_at: "2026-08-02T10:00:00Z" }),
    );
    expect(className).toContain("border-l-primary");
    // Full opacity: no `/NN` suffix on the spine colour. The tint may be faint; the spine
    // must not be, since it is what survives a dim screen.
    expect(className).not.toMatch(/border-l-primary\/\d/);
  });

  it("never uses a warning or loss token", () => {
    // The rule this exists to protect: applying is a CHOSEN, reversible action (ADR 0055).
    // Amber would read as a problem to fix and push users to revert something working as
    // intended. Easy to break later by copying a row style from elsewhere.
    const { className = "" } = appliedRowProps(
      scenario({ applied_at: "2026-08-02T10:00:00Z" }),
    );
    expect(className).not.toMatch(/warning|destructive|loss/);
  });

  it("separates an applied row from a plain one on BOTH shipped surfaces", () => {
    // Guards the guard: Vitest stubs CSS imports to "", and that stub beats ?raw.
    expect(globalsCss.length).toBeGreaterThan(500);

    const tint = Number(
      appliedRowProps(scenario({ applied_at: "2026-08-02T10:00:00Z" }))
        .className?.match(/bg-primary\/(\d+)/)?.[1],
    );
    expect(tint).toBeGreaterThan(0);

    for (const [name, selector] of [
      ["light", ":root"],
      ["dark", ".dark"],
    ] as const) {
      const block = themeBlock(selector);
      const card = token(block, "card");
      const primary = token(block, "primary");
      expect(card, `${name} --card`).toBeTruthy();
      expect(primary, `${name} --primary`).toBeTruthy();

      const surface = toRgb(card!);
      const alpha = tint / 100;
      const composited = toRgb(primary!).map(
        (c, i) => c * alpha + surface[i]! * (1 - alpha),
      ) as [number, number, number];
      const separation = deltaE(composited, surface);
      expect(
        separation,
        `${name}: applied row is indistinguishable from a plain one (${separation.toFixed(2)} < ${TINT_FLOOR})`,
      ).toBeGreaterThanOrEqual(TINT_FLOOR);
    }
  });
});
