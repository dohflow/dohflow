export interface TickPosition {
  /** The tick's data value (e.g. a y-axis dollar amount). */
  value: number;
  /** The tick label's rendered pixel Y position. */
  y: number;
}

export interface LowAnnotationPlacement {
  /** Pixel Y at which to draw the "Low" annotation's label text. */
  y: number;
  /** Data value of the one y-axis tick to suppress, if any collision could
   *  not be resolved by moving the label alone. `undefined` means every
   *  tick renders normally. */
  hiddenTickValue?: number;
}

/// Collision-avoidance for the Cash Flow chart's "Low" annotation
/// (personal-cfo-4d8.29): the label sits a fixed gap above or below its
/// marker, and when the low value's on-chart position lands close to a
/// y-axis gridline, that label can overprint the tick text sitting at
/// nearly the same pixel row ("$30kow $34k" — the literal reported bug).
///
/// Pure and pixel-space rather than data-space: FutureCashChart.tsx supplies
/// real, Recharts-rendered pixel positions (via YAxis's custom `tick` prop
/// and ReferenceDot's custom `label` prop, both of which hand back the
/// actual computed coordinates) rather than this function re-deriving
/// Recharts' own internal scale/margin math, which would risk a second,
/// independent source of pixel truth silently drifting from the real one.
///
/// Strategy: try the label's preferred side (above or below the marker,
/// chosen by FutureCashChart based on the value's sign, unrelated to
/// collision) first; if it's within `collisionThreshold` px of any tick,
/// try the opposite side. If BOTH sides still collide, stay on the
/// preferred side and hide the one tick label it overlaps — moving the
/// annotation further from its own marker would risk it reading as
/// pointing at the wrong day, which is worse than losing one tick label
/// the adjacent ones already bracket.
export function placeLowAnnotation(
  markerY: number,
  preferredSide: "above" | "below",
  ticks: TickPosition[],
  options: { labelHeight?: number; gap?: number; collisionThreshold?: number } = {},
): LowAnnotationPlacement {
  const { labelHeight = 12, gap = 6, collisionThreshold = 12 } = options;
  const halfLabel = labelHeight / 2;
  const above = markerY - gap - halfLabel;
  const below = markerY + gap + halfLabel;

  const collidingTick = (y: number): TickPosition | undefined =>
    ticks.find((t) => Math.abs(t.y - y) <= collisionThreshold);

  const candidates = preferredSide === "above" ? [above, below] : [below, above];
  for (const y of candidates) {
    if (!collidingTick(y)) {
      return { y };
    }
  }

  // Neither side is clear — stay on the preferred side; hide the one tick
  // it still overlaps rather than push the label further from its marker.
  const y = candidates[0]!;
  return { y, hiddenTickValue: collidingTick(y)?.value };
}
