import { Fragment } from "react";

import type { ForecastExplanation } from "./explainEvent";

/// Renders a projected row's typed explanation — a descriptive summary over a
/// definition list of labelled facts. Presentational only: every value is a plain
/// string rendered as text, never raw HTML or Markdown (ADR 0003 trust boundary).
export function ForecastRowExplanation({
  explanation,
  id,
}: {
  explanation: ForecastExplanation;
  id?: string;
}) {
  return (
    <div id={id} className="border-t bg-muted/30 px-6 py-3">
      <p className="text-sm text-muted-foreground">{explanation.summary}</p>
      <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-6 gap-y-1 text-sm">
        {explanation.facts.map((fact) => (
          <Fragment key={fact.label}>
            <dt className="text-muted-foreground">{fact.label}</dt>
            <dd className="tabular-nums">{fact.value}</dd>
          </Fragment>
        ))}
      </dl>
    </div>
  );
}
