/// WCAG 2.x contrast math for the contrast audit (`personal-cfo-4d8.35`). Pure, so it is unit
/// tested without a browser; the audit page (`contrast-audit.tsx`) supplies the RENDERED colors.

export type Rgb = [number, number, number];

/// The floors the audit holds a value to. `HEADROOM` is planning's rule: the engine measured
/// (Chromium) is not the app's (WKWebView), so every value must clear its floor by this much.
export const TEXT_FLOOR = 4.5;
export const NON_TEXT_FLOOR = 3.0;
export const HEADROOM = 0.1;

const toLinear = (channel: number): number => {
  const c = channel / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
};

export function relativeLuminance([r, g, b]: Rgb): number {
  return 0.2126 * toLinear(r) + 0.7152 * toLinear(g) + 0.0722 * toLinear(b);
}

export function contrastRatio(a: Rgb, b: Rgb): number {
  const [hi, lo] = [relativeLuminance(a), relativeLuminance(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

/// `fg` at `alpha` over the opaque `bg` (sRGB), rounded to whole channels.
export function overAlpha(fg: Rgb, bg: Rgb, alpha: number): Rgb {
  return fg.map((v, i) => Math.round(v * alpha + bg[i]! * (1 - alpha))) as Rgb;
}

export function toHex([r, g, b]: Rgb): string {
  return "#" + [r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("");
}

/// The solid rings in a computed `box-shadow` (Tailwind's `ring-2 ring-offset-2` is two spread
/// shadows: the offset gap, then the ring). Blurred and transparent shadows (elevation) are ignored.
export function solidRings(boxShadow: string): { color: Rgb; spread: number }[] {
  const out: { color: Rgb; spread: number }[] = [];
  for (const m of boxShadow.matchAll(
    /rgba?\((\d+),\s*(\d+),\s*(\d+)(?:,\s*([\d.]+))?\)\s+0px\s+0px\s+0px\s+(\d+)px/g,
  )) {
    const alpha = m[4] === undefined ? 1 : Number(m[4]);
    if (alpha < 1) continue;
    out.push({ color: [Number(m[1]), Number(m[2]), Number(m[3])], spread: Number(m[5]) });
  }
  return out;
}

/// A rendered pair passes when it clears `floor` plus the headroom.
export const clears = (ratio: number, floor: number): boolean => ratio >= floor + HEADROOM;
