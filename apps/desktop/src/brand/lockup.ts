import { LOCKUP_RATIOS, MARK_BOUNDS, WORDMARK_VIEWBOX } from "./brand-paths";

export type LockupVariant = "horizontal" | "stacked";

const round = (v: number) => Number(v.toFixed(3));

/// Geometry of a lockup in "mark height = 100" units, exactly as
/// `docs/product/brand/scripts/lockup.py` lays it out: the mark's artwork scaled
/// to 100 tall, the wordmark scaled to the variant's ratio of that height,
/// separated by the variant's gap. Horizontal puts the wordmark to the right of
/// the mark, optically centered on it; stacked centers the wordmark under the mark.
export function lockupGeometry(variant: LockupVariant) {
  const MARK_H = 100;
  const ratios = LOCKUP_RATIOS[variant];
  const ms = MARK_H / MARK_BOUNDS.height;
  const mw = MARK_BOUNDS.width * ms;
  const wh = MARK_H * ratios.wordmark;
  const ws = wh / WORDMARK_VIEWBOX.height;
  const ww = WORDMARK_VIEWBOX.width * ws;
  const gap = MARK_H * ratios.gap;
  const wordmarkOrigin = `translate(${-WORDMARK_VIEWBOX.x} ${-WORDMARK_VIEWBOX.y})`;

  if (variant === "horizontal") {
    const totalWidth = mw + gap + ww;
    return {
      width: round(totalWidth),
      height: MARK_H,
      mark: `translate(${round(-MARK_BOUNDS.x * ms)} ${round(-MARK_BOUNDS.y * ms)}) scale(${round(ms)})`,
      wordmark: `translate(${round(mw + gap)} ${round((MARK_H - wh) / 2)}) scale(${round(ws)}) ${wordmarkOrigin}`,
    };
  }
  const totalWidth = Math.max(mw, ww);
  return {
    width: round(totalWidth),
    height: round(MARK_H + gap + wh),
    mark: `translate(${round((totalWidth - mw) / 2 - MARK_BOUNDS.x * ms)} ${round(-MARK_BOUNDS.y * ms)}) scale(${round(ms)})`,
    wordmark: `translate(${round((totalWidth - ww) / 2)} ${round(MARK_H + gap)}) scale(${round(ws)}) ${wordmarkOrigin}`,
  };
}
