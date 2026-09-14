import type { ForecastDayDto } from "@/bindings";

/// A descriptive comfort-band crossing (ADR 0018 addendum 915.1, personal-cfo-3v6d). Pure facts
/// about where the projection meets the band — never a directive.
export type BandSignal =
  | { kind: "below"; date: string; lowestMinor: number }
  | { kind: "above"; excessMinor: number };

/// The band edges to compare against, in minor units; `upper` is `null` when unset.
export type SignalBand = { lower: number; upper: number | null };

/// Descriptive band-crossing signals over the projection (915.1 "what-would-it-take" boundary):
/// the first day the projected balance dips below the lower edge (with its lowest point), and
/// whether it ends above the upper edge (excess). Deterministic; empty when there's no band or no
/// crossing. The copy that renders these must stay descriptive (no directives) per ADR 0018.
export function comfortBandSignals(
  days: ForecastDayDto[],
  band: SignalBand | null,
): BandSignal[] {
  if (!band || days.length === 0) return [];
  const nets = days.map((d) => ({ date: d.date, net: d.closing.p50.minor_units }));
  const signals: BandSignal[] = [];

  // Lower edge: the first day the projection crosses below it, plus its lowest point.
  const firstBelow = nets.find((d) => d.net < band.lower);
  if (firstBelow) {
    const lowestMinor = Math.min(...nets.map((d) => d.net));
    signals.push({ kind: "below", date: firstBelow.date, lowestMinor });
  }

  // Upper edge: ending above it is excess (only when an upper edge is set).
  if (band.upper !== null) {
    const endMinor = nets[nets.length - 1]!.net;
    if (endMinor > band.upper) {
      signals.push({ kind: "above", excessMinor: endMinor - band.upper });
    }
  }
  return signals;
}
