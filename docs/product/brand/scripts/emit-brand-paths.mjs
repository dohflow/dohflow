#!/usr/bin/env node
// Emit apps/desktop/src/brand/brand-paths.ts from the approved brand SVGs.
//
// The in-app brand components (bead personal-cfo-4d8.28.3) must carry the mark
// and wordmark geometry VERBATIM — never re-drawn, never rasterized, never set
// in a font. This script is the single path from docs/product/brand/*.svg to
// the TypeScript data module the components render, and the Brand test
// re-derives the same data from the SVGs so any drift fails CI.
//
// Bounds and ratios follow docs/product/brand/scripts/lockup.py exactly: the
// mark's tight artwork bounds are measured from its path coordinates (absolute
// commands, x/y alternating — the generator refuses any other command set), and
// the lockup ratios are read from lockup.py's own defaults so the Python and
// the TypeScript cannot drift apart silently.
//
// Run from the repository root:
//   node docs/product/brand/scripts/emit-brand-paths.mjs
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const brandDir = join(here, "..");
const outFile = join(here, "..", "..", "..", "..", "apps", "desktop", "src", "brand", "brand-paths.ts");

const mark = readFileSync(join(brandDir, "mark.svg"), "utf8");
const wordmark = readFileSync(join(brandDir, "wordmark.svg"), "utf8");
const lockupPy = readFileSync(join(here, "lockup.py"), "utf8");

const PATH_RE = /<path\b([^>]*)>/g;
const ATTR = (attrs, name) => {
  const m = new RegExp(`\\b${name}="([^"]*)"`).exec(attrs);
  return m ? m[1] : null;
};
const NUM = /-?\d*\.?\d+/g;

function paths(svg) {
  const out = [];
  for (const m of svg.matchAll(PATH_RE)) {
    out.push({ d: ATTR(m[1], "d"), fill: ATTR(m[1], "fill") });
  }
  return out;
}

function viewBox(svg) {
  const m = /viewBox="([^"]+)"/.exec(svg);
  const [x, y, width, height] = m[1].split(/\s+/).map(Number);
  return { x, y, width, height };
}

// The alternating-x/y measurement is only valid for absolute M/L/C (and Z).
// Relative commands, H/V, S/Q/T, or arcs would silently mis-measure, so refuse.
const ALLOWED_COMMANDS = new Set(["M", "L", "C", "Z"]);
function assertAbsoluteCommandsOnly(list, label) {
  for (const { d } of list) {
    const commands = new Set(d.match(/[A-Za-z]/g) ?? []);
    for (const c of commands) {
      if (!ALLOWED_COMMANDS.has(c)) {
        throw new Error(`${label}: path command "${c}" is outside {M, L, C, Z}; the bounds measurement assumes absolute commands with x/y pairs`);
      }
    }
  }
}

// Same measurement lockup.py makes: every number in every d attribute,
// alternating x and y.
function bounds(list) {
  const xs = [];
  const ys = [];
  for (const { d } of list) {
    const n = d.match(NUM).map(Number);
    for (let i = 0; i < n.length; i += 2) xs.push(n[i]);
    for (let i = 1; i < n.length; i += 2) ys.push(n[i]);
  }
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, width: Math.max(...xs) - x, height: Math.max(...ys) - y };
}

// lockup.py's defaults are the ratios that produced lockup-*.svg.
function ratiosFrom(py, fn) {
  const m = new RegExp(`def ${fn}\\([^)]*gap_ratio=([0-9.]+)[^)]*wm_ratio=([0-9.]+)`).exec(py);
  if (!m) throw new Error(`lockup.py: could not read the ${fn}() defaults`);
  return { gap: Number(m[1]), wordmark: Number(m[2]) };
}

const FILL_ROLE = {
  "#006341": "emerald",
  "#DB8F6B": "terracotta",
  white: "white",
  "#1c1815": "ink",
};

const markList = paths(mark);
assertAbsoluteCommandsOnly(markList, "mark.svg");
const markPaths = markList.map(({ d, fill }) => {
  const role = FILL_ROLE[fill];
  if (!role) throw new Error(`mark.svg has an unmapped fill: ${fill}`);
  return { role, d };
});
const wordmarkPaths = paths(wordmark).map(({ d }) => d);
const markBounds = bounds(markList);
const wordmarkBox = viewBox(wordmark);
const ratios = { horizontal: ratiosFrom(lockupPy, "horizontal"), stacked: ratiosFrom(lockupPy, "stacked") };

const r = (v) => Number(v.toFixed(2));
const q = (s) => JSON.stringify(s);

const markEntries = markPaths.map((p) => `  { role: ${q(p.role)}, d: ${q(p.d)} },`).join("\n");
const wordmarkEntries = wordmarkPaths.map((d) => `  ${q(d)},`).join("\n");

const ts = `// GENERATED FILE — do not edit by hand.
// Source: docs/product/brand/mark.svg, docs/product/brand/wordmark.svg, and the
// lockup ratios in docs/product/brand/scripts/lockup.py.
// Generator: docs/product/brand/scripts/emit-brand-paths.mjs (bead personal-cfo-4d8.28.3)
// The Brand test re-derives this data from the sources; edit the sources, then re-run the generator.

export type MarkFillRole = "emerald" | "terracotta" | "white" | "ink";

export interface ViewBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Tight bounds of the mark's artwork inside its 1024 canvas (measured, per lockup.py). */
export const MARK_BOUNDS: ViewBox = { x: ${r(markBounds.x)}, y: ${r(markBounds.y)}, width: ${r(markBounds.width)}, height: ${r(markBounds.height)} };

/** The four bands of the mark, in paint order, geometry verbatim from mark.svg. */
export const MARK_PATHS: ReadonlyArray<{ role: MarkFillRole; d: string }> = [
${markEntries}
];

/** The wordmark's own viewBox (Nunito 800 outlines, typeset — never a font at runtime). */
export const WORDMARK_VIEWBOX: ViewBox = { x: ${wordmarkBox.x}, y: ${wordmarkBox.y}, width: ${wordmarkBox.width}, height: ${wordmarkBox.height} };

/** The seven glyph outlines of "DohFlow", verbatim from wordmark.svg. */
export const WORDMARK_PATHS: ReadonlyArray<string> = [
${wordmarkEntries}
];

/** Lockup ratios read from lockup.py's defaults: gap and wordmark height as fractions of the mark height. */
export const LOCKUP_RATIOS = {
  horizontal: { gap: ${ratios.horizontal.gap}, wordmark: ${ratios.horizontal.wordmark} },
  stacked: { gap: ${ratios.stacked.gap}, wordmark: ${ratios.stacked.wordmark} },
} as const;
`;

writeFileSync(outFile, ts);
console.log(`wrote ${outFile}`);
console.log(`mark artwork ${markBounds.width.toFixed(0)} x ${markBounds.height.toFixed(0)} at (${markBounds.x.toFixed(1)}, ${markBounds.y.toFixed(1)}); ${markPaths.length} mark paths, ${wordmarkPaths.length} wordmark paths; ratios ${JSON.stringify(ratios)}`);
