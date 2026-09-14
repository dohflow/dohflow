import { render, screen } from "@testing-library/react";

import lockupHorizontalSvg from "../../../../docs/product/brand/lockup-horizontal.svg?raw";
import lockupStackedSvg from "../../../../docs/product/brand/lockup-stacked.svg?raw";
import lockupPy from "../../../../docs/product/brand/scripts/lockup.py?raw";
import markSvg from "../../../../docs/product/brand/mark.svg?raw";
import wordmarkSvg from "../../../../docs/product/brand/wordmark.svg?raw";

import { BrandLockup, BrandMark, BrandWordmark } from "./Brand";
import { LOCKUP_RATIOS, MARK_BOUNDS, MARK_PATHS, WORDMARK_PATHS } from "./brand-paths";
import { lockupGeometry } from "./lockup";

/// The approved artwork is the source of truth. These tests re-derive what the
/// generated `brand-paths.ts` carries straight from the SVGs under
/// `docs/product/brand/`, so a hand edit to either side fails here (bead
/// personal-cfo-4d8.28.3: geometry verbatim, never re-drawn).
const pathData = (svg: string): string[] =>
  [...svg.matchAll(/<path\b[^>]*\bd="([^"]*)"/g)].map((m) => m[1] ?? "");
const pathFills = (svg: string): string[] =>
  [...svg.matchAll(/<path\b[^>]*\bfill="([^"]*)"/g)].map((m) => m[1] ?? "");
const viewBoxOf = (svg: string): { width: number; height: number } => {
  const raw = /viewBox="([^"]+)"/.exec(svg)?.[1] ?? "";
  const [, , width = NaN, height = NaN] = raw.split(/\s+/).map(Number);
  return { width, height };
};

/// lockup.py measures the mark's tight bounds as min/max over every number in
/// every `d`, alternating x and y. Same measurement, so the same ratios.
function measuredBounds(svg: string) {
  const xs: number[] = [];
  const ys: number[] = [];
  for (const d of pathData(svg)) {
    const nums = (d.match(/-?\d*\.?\d+/g) ?? []).map(Number);
    nums.forEach((n, i) => (i % 2 === 0 ? xs : ys).push(n));
  }
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, width: Math.max(...xs) - x, height: Math.max(...ys) - y };
}

describe("brand components (personal-cfo-4d8.28.3)", () => {
  it("carries the mark's four bands verbatim from mark.svg, in paint order", () => {
    expect(MARK_PATHS.map((p) => p.d)).toEqual(pathData(markSvg));
    expect(MARK_PATHS.map((p) => p.role)).toEqual(["emerald", "terracotta", "white", "ink"]);
    // The roles map onto the artwork's own fills — the brand emerald, Saltillo
    // terracotta, white, ink — which is what the tokens must resolve to in light mode.
    expect(pathFills(markSvg)).toEqual(["#006341", "#DB8F6B", "white", "#1c1815"]);
  });

  it("carries the wordmark's seven outlines verbatim from wordmark.svg", () => {
    expect(WORDMARK_PATHS).toEqual(pathData(wordmarkSvg));
    expect(WORDMARK_PATHS).toHaveLength(7);
    // Typeset outlines, not a font: the source has no <text> element.
    expect(wordmarkSvg).not.toMatch(/<text\b/);
  });

  it("measures the mark's artwork bounds the way lockup.py does", () => {
    const b = measuredBounds(markSvg);
    expect(MARK_BOUNDS.x).toBeCloseTo(b.x, 1);
    expect(MARK_BOUNDS.y).toBeCloseTo(b.y, 1);
    expect(MARK_BOUNDS.width).toBeCloseTo(b.width, 1);
    expect(MARK_BOUNDS.height).toBeCloseTo(b.height, 1);
  });

  it("composes the lockups to the approved lockup SVGs' proportions", () => {
    // The ratios must be the defaults lockup.py used to build the approved lockups,
    // read from the script itself so the Python and the TypeScript cannot diverge.
    const ratiosFrom = (fn: string) => {
      const m = new RegExp(`def ${fn}\\([^)]*gap_ratio=([0-9.]+)[^)]*wm_ratio=([0-9.]+)`).exec(lockupPy);
      return { gap: Number(m?.[1]), wordmark: Number(m?.[2]) };
    };
    expect(LOCKUP_RATIOS).toEqual({
      horizontal: ratiosFrom("horizontal"),
      stacked: ratiosFrom("stacked"),
    });
    const approvedHorizontal = viewBoxOf(lockupHorizontalSvg);
    const approvedStacked = viewBoxOf(lockupStackedSvg);
    const horizontal = lockupGeometry("horizontal");
    const stacked = lockupGeometry("stacked");
    expect(horizontal.width).toBeCloseTo(approvedHorizontal.width, 0);
    expect(horizontal.height).toBeCloseTo(approvedHorizontal.height, 0);
    expect(stacked.width).toBeCloseTo(approvedStacked.width, 0);
    expect(stacked.height).toBeCloseTo(approvedStacked.height, 0);
  });

  it("paints the mark through design tokens, never a literal color", () => {
    const { container } = render(<BrandMark />);
    const fills = [...container.querySelectorAll("path")].map((p) => p.getAttribute("fill"));
    expect(fills).toHaveLength(4);
    for (const fill of fills) expect(fill).toMatch(/^var\(--[a-z-]+\)$/);
    expect(container.innerHTML).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });

  it("renders the wordmark in currentColor so it follows the surrounding text", () => {
    const { container } = render(<BrandWordmark height={16} />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("fill")).toBe("currentColor");
    expect(svg?.querySelectorAll("path")).toHaveLength(7);
  });

  it("exposes every component as an image named DohFlow and never uses a raster asset", () => {
    const { container } = render(
      <>
        <BrandMark />
        <BrandWordmark />
        <BrandLockup variant="horizontal" />
        <BrandLockup variant="stacked" />
      </>,
    );
    expect(screen.getAllByRole("img", { name: "DohFlow" })).toHaveLength(4);
    expect(container.querySelector("img")).toBeNull();
  });

  it("sizes a lockup by its rendered height and keeps the composition's aspect ratio", () => {
    const { container } = render(<BrandLockup variant="horizontal" height={24} />);
    const svg = container.querySelector("svg");
    expect(svg).not.toBeNull();
    const g = lockupGeometry("horizontal");
    expect(svg?.getAttribute("height")).toBe("24");
    expect(Number(svg?.getAttribute("width"))).toBeCloseTo((24 * g.width) / g.height, 1);
    // The wordmark group inherits the text color; the mark keeps its own fills.
    const groups = [...(svg?.children ?? [])].filter((el) => el.tagName === "g");
    expect(groups).toHaveLength(2);
    expect(groups[0]?.getAttribute("fill")).toBeNull();
    expect(groups[1]?.getAttribute("fill")).toBe("currentColor");
  });
});
