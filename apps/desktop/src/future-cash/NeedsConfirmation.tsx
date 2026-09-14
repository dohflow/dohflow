import { useState } from "react";
import { ChevronDown } from "lucide-react";

import type { ForecastEventDto, UnconfirmedOccurrenceDto } from "@/bindings";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { cn } from "@/lib/utils";
import { formatIsoDate, formatMoney } from "@/lib/format";

import { MarkObligationPaid } from "./MarkObligationPaid";
import { useUnconfirmedPastDue } from "./useUnconfirmedPastDue";

/// Adapt an occurrence to the shape `MarkObligationPaid` already takes, so a confirm from
/// this section is the SAME audited write as a confirm from Projected Activity — one path,
/// one behaviour, and the undo bar keeps working (ADR 0058, consequences).
///
/// The amount stays a positive magnitude: the control runs it through `Math.abs` for the
/// prefill and sends whatever the user submits, so the forecast-event sign convention does
/// not reach the ledger from here.
function asForecastEvent(o: UnconfirmedOccurrenceDto): ForecastEventDto {
  return {
    source_event_id: o.recurring_event_id,
    name: o.name,
    kind: "recurring_bill",
    amount: { minor_units: o.expected_amount_minor, currency: o.currency },
    assumption_basis: { kind: "recurring_schedule", frequency: null },
  };
}

const COLLAPSE_KEY = "pcfo.needsConfirmCollapsed";

function readCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSE_KEY) === "1";
  } catch {
    return false;
  }
}

/// Obligations whose scheduled date has passed with nothing recorded against them
/// (personal-cfo-4d8.27.7.6, ADR 0058).
///
/// This sits on Cash Flow, above the projection, because the projection is what is at
/// stake: until each of these is resolved the forecast is wrong in one direction or the
/// other — either the money already left, or it is still going to. The fix belongs next to
/// the figure it corrects.
///
/// Copy is descriptive (ADR 0018): it states what is unresolved and what the forecast
/// currently assumes. It does not tell the user to pay anything and does not call them
/// late.
export function NeedsConfirmation() {
  const { occurrences, error } = useUnconfirmedPastDue();
  // Collapsible so a long queue doesn't bury the Projected Activity table it
  // sits above (owner dogfooding, nwwy); sticky like the Money Inbox toggle.
  const [collapsed, setCollapsed] = useState(readCollapsed);

  // Nothing to show while loading, on error, or when the queue is empty. An empty state
  // here would be a permanent empty card on the surface a healthy household sees most.
  if (error !== null || occurrences === null || occurrences.length === 0) return null;

  function toggle() {
    setCollapsed((prev) => {
      const next = !prev;
      try {
        localStorage.setItem(COLLAPSE_KEY, next ? "1" : "0");
      } catch {
        // Per-viewer convenience only — a storage failure just loses stickiness.
      }
      return next;
    });
  }

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-base">
          <button
            type="button"
            aria-expanded={!collapsed}
            aria-controls="needs-confirmation-list"
            onClick={toggle}
            className="flex w-full items-center gap-2 text-left"
          >
            <ChevronDown
              aria-hidden
              className={cn("size-4 shrink-0 transition-transform", collapsed && "-rotate-90")}
            />
            {occurrences.length === 1
              ? "1 bill still needs confirming"
              : `${occurrences.length} bills still need confirming`}
          </button>
        </CardTitle>
        {!collapsed && (
          <p className="text-sm text-muted-foreground">
            Their due date has passed and nothing has been recorded against them, so the
            forecast below still counts them as money that has not left yet.
          </p>
        )}
      </CardHeader>
      {collapsed ? null : (
        <CardContent id="needs-confirmation-list" className="p-0">
        <ul className="divide-y">
          {occurrences.map((o) => (
            <li
              key={`${o.recurring_event_id}:${o.scheduled_date}`}
              className="flex flex-wrap items-center gap-x-3 gap-y-1 px-4 py-3"
            >
              <span className="min-w-0 flex-1 truncate text-sm font-medium">{o.name}</span>
              <span className="text-xs text-muted-foreground">
                {`Due ${formatIsoDate(o.scheduled_date)} · ${
                  o.days_overdue === 1 ? "1 day ago" : `${o.days_overdue} days ago`
                }`}
              </span>
              <span className="text-sm tabular-nums">
                {formatMoney({
                  minor_units: o.expected_amount_minor,
                  currency: o.currency,
                })}
              </span>
              <MarkObligationPaid
                event={asForecastEvent(o)}
                scheduledDate={o.scheduled_date}
              />
            </li>
          ))}
        </ul>
        </CardContent>
      )}
    </Card>
  );
}
