import { contrastRatio, type Rgb } from "./contrast-math";

/// Browser-only helpers for the contrast audit: read what was actually PAINTED, not what the
/// stylesheet says. Colors are composited through a 1x1 canvas so `color-mix(...)` tints, alpha
/// borders and any color syntax the engine understands resolve exactly as they render.

let context: CanvasRenderingContext2D | null = null;
function canvas(): CanvasRenderingContext2D {
  if (context) return context;
  const el = document.createElement("canvas");
  el.width = 1;
  el.height = 1;
  const ctx = el.getContext("2d", { willReadFrequently: true });
  if (!ctx) throw new Error("no 2d canvas: cannot composite colors");
  context = ctx;
  return ctx;
}

const TRANSPARENT = new Set(["", "transparent", "rgba(0, 0, 0, 0)"]);

/// Paint `colors` back to front over an opaque white page and read the result.
export function composite(colors: string[]): Rgb {
  const ctx = canvas();
  ctx.fillStyle = "white";
  ctx.fillRect(0, 0, 1, 1);
  for (const color of colors) {
    if (TRANSPARENT.has(color)) continue;
    ctx.fillStyle = color;
    ctx.fillRect(0, 0, 1, 1);
  }
  const d = ctx.getImageData(0, 0, 1, 1).data;
  return [d[0]!, d[1]!, d[2]!];
}

/// The computed backgrounds of `el`'s ancestors, outermost first (and `el` itself last, if asked).
export function backgroundChain(el: Element, includeSelf: boolean): string[] {
  const out: string[] = [];
  let node: Element | null = includeSelf ? el : el.parentElement;
  while (node) {
    out.unshift(getComputedStyle(node).backgroundColor);
    node = node.parentElement;
  }
  return out;
}

export const ratio = (a: Rgb, b: Rgb): number => Math.round(contrastRatio(a, b) * 100) / 100;

export function engine(): string {
  return `${navigator.userAgent} | dpr ${window.devicePixelRatio}`;
}
