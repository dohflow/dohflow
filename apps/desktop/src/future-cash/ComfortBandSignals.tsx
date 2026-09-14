import { ArrowDownRight, ArrowUpRight } from "lucide-react";

import type { BandDriftSignalDto, ForecastDayDto } from "@/bindings";
import { formatIsoDate, formatMoney } from "@/lib/format";

import { comfortBandSignals, type SignalBand } from "./bandSignals";
import { useBandDriftSignal } from "./useBandDriftSignal";

const BELOW_CLASS =
  "flex items-start gap-2 rounded-md border border-loss/30 bg-loss/5 px-3 py-2 text-sm";
const ABOVE_CLASS =
  "flex items-start gap-2 rounded-md border border-gain/30 bg-gain/5 px-3 py-2 text-sm";

/// The rising-spend categories as a descriptive clause, e.g. "dining (up about $300/mo),
/// groceries (up about $150/mo)". Facts only — the attribution the backend computed (5ie.8).
function driftFactorsText(drift: BandDriftSignalDto): string {
  return drift.factors
    .map((f) => `${f.category_name} (up about ${formatMoney(f.delta)}/mo)`)
    .join(", ");
}

/// Descriptive comfort-band signals under the Future Cash chart (ADR 0018 addendum 915.1). Strictly
/// "what-would-it-take" facts — where the projection crosses the band and, when the crossing is a
/// spending drift, *why* (the rising categories, personal-cfo-5ie.8) — never a directive (no
/// should/recommend/consider/move; enforced by copy-review.test.ts).
export function ComfortBandSignals({
  days,
  band,
  currency,
}: {
  days: ForecastDayDto[];
  band: SignalBand | null;
  currency: string;
}) {
  const { drift } = useBandDriftSignal();
  const signals = comfortBandSignals(days, band);
  const above = signals.find((s) => s.kind === "above");
  const below = signals.find((s) => s.kind === "below");

  if (!drift && signals.length === 0) return null;

  return (
    <div className="flex flex-col gap-2">
      {drift ? (
        // The below-crossing WITH its attribution (5ie.8) — richer than, and replaces, the plain one.
        <p key="drift" role="status" className={BELOW_CLASS}>
          <ArrowDownRight className="mt-0.5 size-4 shrink-0 text-loss" aria-hidden />
          <span>
            Your projected balance crosses below the comfort band on{" "}
            <span className="font-medium">{formatIsoDate(drift.crossing_date)}</span>, driven mostly
            by {driftFactorsText(drift)}.
          </span>
        </p>
      ) : below ? (
        <p key="below" role="status" className={BELOW_CLASS}>
          <ArrowDownRight className="mt-0.5 size-4 shrink-0 text-loss" aria-hidden />
          <span>
            Your projected balance crosses below the comfort band on{" "}
            <span className="font-medium">{formatIsoDate(below.date)}</span>, reaching about{" "}
            <span className="font-medium tabular-nums">
              {formatMoney({ minor_units: below.lowestMinor, currency })}
            </span>{" "}
            at its lowest.
          </span>
        </p>
      ) : null}
      {above && (
        <p key="above" role="status" className={ABOVE_CLASS}>
          <ArrowUpRight className="mt-0.5 size-4 shrink-0 text-gain" aria-hidden />
          <span>
            You're projected to end the horizon about{" "}
            <span className="font-medium tabular-nums">
              {formatMoney({ minor_units: above.excessMinor, currency })}
            </span>{" "}
            above the comfort band.
          </span>
        </p>
      )}
    </div>
  );
}
