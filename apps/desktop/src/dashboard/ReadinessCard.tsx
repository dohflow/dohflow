import { useEffect, useRef, useState } from "react";
import { Gauge, Loader2, X } from "lucide-react";

import type { ForecastReadinessDto, ReadinessFactorDto } from "@/bindings";
import { Card, CardContent } from "@/components/ui/card";
import { cn } from "@/lib/utils";
import { useForecastReadiness } from "./useForecastReadiness";

/// Forecast Readiness band (ADR 0026 §13): label + the Tailwind text/bar colors.
function band(score: number): { label: string; text: string; bar: string } {
  if (score >= 75)
    return { label: "Good", text: "text-gain", bar: "bg-gain" };
  if (score >= 40)
    return { label: "Fair", text: "text-warning", bar: "bg-warning" };
  return { label: "Getting started", text: "text-loss", bar: "bg-loss" };
}

/// The Forecast Readiness card (personal-cfo-6vj9, ADR 0026 §13): a 0–100 trust
/// indicator for the deterministic forecast.
///
/// Compact by design (personal-cfo-4d8.27.2): it shows the score, its band, and the
/// single most useful next action in one row — the full per-factor breakdown opens in a
/// centered dialog rather than expanding in place. The in-flow expansion pushed the rest
/// of the dashboard down, which is what made this widget dominate a screen it is only
/// meant to annotate. Self-contained — owns its query — so it loads independently of the
/// dashboard forecast.
export function ReadinessCard() {
  const { readiness, error } = useForecastReadiness();
  const [open, setOpen] = useState(false);

  return (
    <Card>
      <CardContent className="pt-6">
        {error ? (
          <p role="alert" className="text-sm text-loss">
            {error}
          </p>
        ) : readiness === null ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" aria-hidden />
            Loading…
          </div>
        ) : (
          <Readiness readiness={readiness} onOpen={() => setOpen(true)} />
        )}
      </CardContent>
      {open && readiness && (
        <ReadinessDialog readiness={readiness} onClose={() => setOpen(false)} />
      )}
    </Card>
  );
}

function Readiness({
  readiness,
  onOpen,
}: {
  readiness: ForecastReadinessDto;
  onOpen: () => void;
}) {
  const { score, factors } = readiness;
  const { label, text } = band(score);
  // The most useful next action: the lowest-scoring factor's detail. When every
  // factor is maxed, the forecast is as trustworthy as R1 inputs allow.
  const weakest = factors.reduce<ReadinessFactorDto | null>(
    (lowest, f) => (lowest === null || f.score < lowest.score ? f : lowest),
    null,
  );
  const headline =
    weakest && weakest.score < 100
      ? weakest.detail
      : "Your forecast is as trustworthy as today's inputs allow.";

  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
      <Gauge className={cn("size-6 shrink-0", text)} aria-hidden />
      <span className="text-sm font-medium text-muted-foreground">
        Forecast readiness
      </span>
      <span className="text-xl font-semibold tabular-nums">
        <span>{score}</span>
        <span className="text-sm font-normal text-muted-foreground">/100</span>
        <span className={cn("ml-2 text-sm font-medium", text)}>{label}</span>
      </span>
      {/* The next action sits inline and truncates — the full copy is in the dialog. */}
      <p className="min-w-0 flex-1 truncate text-sm text-muted-foreground" title={headline}>
        {headline}
      </p>
      <button
        type="button"
        onClick={onOpen}
        className="shrink-0 rounded-md px-2 py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        Details
      </button>
    </div>
  );
}

/// The per-factor breakdown, in a centered dialog so it never displaces the dashboard
/// (personal-cfo-4d8.27.2). Follows the app's existing hand-rolled modal idiom
/// (CardReviewModal / NewLoanDialog); a shared Dialog primitive is 4d8.6's job.
function ReadinessDialog({
  readiness,
  onClose,
}: {
  readiness: ForecastReadinessDto;
  onClose: () => void;
}) {
  const dialogRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    // `aria-modal` hides everything outside the dialog from assistive tech, so focus has
    // to come WITH it — otherwise the still-focused trigger becomes invisible and the
    // dialog announces nothing (matches CardReviewModal).
    dialogRef.current?.focus();
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const { label, text } = band(readiness.score);
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-foreground/40 p-6"
      onClick={onClose}
      role="presentation"
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label="Forecast readiness detail"
        onClick={(event) => event.stopPropagation()}
        className="max-h-[80vh] w-full max-w-md overflow-y-auto rounded-lg border bg-card p-5 shadow-lg"
      >
        <div className="mb-4 flex items-start justify-between gap-4">
          <div>
            <h2 className="text-base font-semibold">Forecast readiness</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              What each input contributes to how far you can trust the forecast.
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          >
            <X className="size-4" aria-hidden />
          </button>
        </div>
        <p className="mb-4 text-2xl font-semibold tabular-nums">
          {readiness.score}
          <span className="text-base font-normal text-muted-foreground">/100</span>
          <span className={cn("ml-2 text-sm font-medium", text)}>{label}</span>
        </p>
        <ul className="flex flex-col gap-3">
          {readiness.factors.map((factor) => (
            <FactorRow key={factor.key} factor={factor} />
          ))}
        </ul>
      </div>
    </div>
  );
}

function FactorRow({ factor }: { factor: ReadinessFactorDto }) {
  const { bar } = band(factor.score);
  return (
    <li className="flex flex-col gap-1">
      <div className="flex items-center justify-between text-sm">
        <span className="font-medium">{factor.label}</span>
        <span className="tabular-nums text-muted-foreground">{factor.score}</span>
      </div>
      <div
        className="h-1.5 w-full overflow-hidden rounded-full bg-muted"
        role="presentation"
      >
        <div
          className={cn("h-full rounded-full", bar)}
          style={{ width: `${factor.score}%` }}
        />
      </div>
      <p className="text-xs text-muted-foreground">{factor.detail}</p>
    </li>
  );
}
