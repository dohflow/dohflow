import type { SVGProps } from "react";

import {
  MARK_BOUNDS,
  MARK_PATHS,
  WORDMARK_PATHS,
  WORDMARK_VIEWBOX,
  type MarkFillRole,
} from "./brand-paths";
import { lockupGeometry, type LockupVariant } from "./lockup";

/// In-app brand components (personal-cfo-4d8.28.3, ADR 0067).
///
/// The mark and the wordmark are rendered from the approved SVG geometry in
/// `docs/product/brand/` — carried verbatim through `brand-paths.ts`, never
/// re-drawn, never rasterized, and never set in a font at runtime (the wordmark
/// is Nunito 800 converted to outlines, per ADR 0063 and the brand sheet). The
/// lockups are composed with the same ratios as `scripts/lockup.py`, so what the
/// sidebar shows is the same lockup the website and the press kit show.
///
/// Color goes through tokens, never literals (design-token audit,
/// personal-cfo-4d8.27.4.7). Emerald follows `--primary`: the brand emerald in
/// light mode and the token's brighter green on dark surfaces, which is the one
/// dark-mode adaptation the brand direction allows. Terracotta, ink and white are
/// fixed brand values the theme never flips. The wordmark is `currentColor`, so
/// it takes the surrounding text color in both modes.
///
/// Decision, recorded on purpose (from the review of pull request 394): on dark surfaces the fixed
/// ink band (`--on-brand-fill`, the same ink as the light theme) sits close to the
/// dark background, so the D reads as a knockout gap between the emerald bowl and
/// the white wave rather than a fourth painted band. That is the same reverse
/// treatment a logo gets on a dark card, and the brand sheet has no mark-on-dark
/// variant yet. The owner judges it in the dark-mode dogfooding round; if a
/// dedicated dark treatment is wanted it lands as a brand decision
/// (personal-cfo-n76x.3.7), not as an ad-hoc token swap here.
const MARK_FILL: Record<MarkFillRole, string> = {
  emerald: "var(--primary)",
  terracotta: "var(--terracotta)",
  white: "var(--brand-white)",
  ink: "var(--on-brand-fill)",
};

type SvgRest = Omit<
  SVGProps<SVGSVGElement>,
  "height" | "width" | "viewBox" | "children" | "role" | "aria-label"
>;

const round = (v: number) => Number(v.toFixed(3));

function MarkPaths() {
  return (
    <>
      {MARK_PATHS.map((p) => (
        <path key={p.role} fill={MARK_FILL[p.role]} d={p.d} />
      ))}
    </>
  );
}

function WordmarkPaths() {
  return (
    <>
      {WORDMARK_PATHS.map((d, i) => (
        <path key={i} d={d} />
      ))}
    </>
  );
}

/// The mark alone (the four-band D), cropped to its artwork bounds.
export function BrandMark({ height = 28, ...rest }: { height?: number } & SvgRest) {
  const { x, y, width, height: h } = MARK_BOUNDS;
  return (
    <svg
      role="img"
      aria-label="DohFlow"
      viewBox={`${x} ${y} ${width} ${h}`}
      height={height}
      width={round((height * width) / h)}
      {...rest}
    >
      <MarkPaths />
    </svg>
  );
}

/// The wordmark alone, in `currentColor`.
export function BrandWordmark({ height = 16, ...rest }: { height?: number } & SvgRest) {
  const { x, y, width, height: h } = WORDMARK_VIEWBOX;
  return (
    <svg
      role="img"
      aria-label="DohFlow"
      viewBox={`${x} ${y} ${width} ${h}`}
      height={height}
      width={round((height * width) / h)}
      fill="currentColor"
      {...rest}
    >
      <WordmarkPaths />
    </svg>
  );
}

/// Mark + wordmark, horizontal (mark left, wordmark optically centered on it) or
/// stacked (wordmark under a centered mark). `height` is the rendered height of the
/// whole lockup; width follows from the composition's aspect ratio.
export function BrandLockup({
  variant = "horizontal",
  height = 28,
  ...rest
}: { variant?: LockupVariant; height?: number } & SvgRest) {
  const g = lockupGeometry(variant);
  return (
    <svg
      role="img"
      aria-label="DohFlow"
      viewBox={`0 0 ${g.width} ${g.height}`}
      height={height}
      width={round((height * g.width) / g.height)}
      {...rest}
    >
      <g transform={g.mark}>
        <MarkPaths />
      </g>
      <g fill="currentColor" transform={g.wordmark}>
        <WordmarkPaths />
      </g>
    </svg>
  );
}
