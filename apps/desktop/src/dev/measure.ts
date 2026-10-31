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

/// Where and how a result was captured, so a saved file speaks for itself (personal-cfo-4d8.35, Studio F2):
/// the exact source commit, when, the engine, the locale and zone the status stamps were formatted in, and
/// the viewport and pixel ratio.
export function captureMeta() {
  return {
    sourceSha: __SOURCE_SHA__,
    capturedAt: new Date().toISOString(),
    userAgent: navigator.userAgent,
    locale: navigator.language,
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
    devicePixelRatio: window.devicePixelRatio,
  };
}

/// Save a result to a local collector, when asked: `?post=http://127.0.0.1:PORT/save&name=file.json`. This is how a
/// capture from an engine with no console (the iOS simulator's Safari) leaves the page, and how a committed raw
/// result is taken from a clean source without hand-copying. Only a loopback address is accepted, so a dev page can
/// never send a result anywhere else.
export function postResultIfAsked(payload: unknown): void {
  const params = new URLSearchParams(window.location.search);
  const target = params.get("post");
  const name = params.get("name");
  if (!target || !name) return;
  const url = new URL(target);
  if (url.protocol !== "http:" || !["127.0.0.1", "localhost"].includes(url.hostname)) return;
  url.searchParams.set("name", name);
  void fetch(url, { method: "POST", body: JSON.stringify(payload) });
}
