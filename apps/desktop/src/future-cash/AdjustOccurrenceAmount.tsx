import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type ForecastEventDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { dollarsToMinorUnits } from "@/lib/format";

/// Which assumption kind overrides this event's amount; `null` for kinds whose amount
/// is not a scheduled entity amount (a manual entry is edited where it was created; a
/// card/loan payment is derived from the debt, not stored on the event).
function amountKindFor(event: ForecastEventDto): string | null {
  if (event.kind === "recurring_bill") return "bill_amount";
  if (event.kind === "income") return "income_amount";
  return null;
}

/// Adjust ONE upcoming occurrence's projected amount (personal-cfo-4d8.27.7.4).
///
/// The owner's case: "our recurring bill system knows the typical range, but a heatwave
/// means this month's electric bill is ~$30 higher — I should be able to update that so
/// it reflects properly on the upcoming cash forecast."
///
/// This writes a **base** (non-scenario) `bill_amount`/`income_amount` assumption whose
/// window is exactly this date (`effective_date == end_date == the row's date`), which
/// `EntityOverride::amount_for` treats as covering that occurrence only — later
/// occurrences fall back to the entity's stored amount, which is what "just this month"
/// means. A scenario would keep it hypothetical; the owner wants their real forecast to
/// move, so base is correct here.
export function AdjustOccurrenceAmount({
  event,
  scheduledDate,
}: {
  event: ForecastEventDto;
  scheduledDate: string;
}) {
  const kind = amountKindFor(event);
  const queryClient = useQueryClient();
  // An adjustment writes to the BASE forecast, so it must be visible and reversible —
  // otherwise it is a one-way door on the user's real numbers (adversarial review).
  const overrides = useQuery({
    queryKey: ["forecast", "assumptions", "base"],
    queryFn: () => commands.forecastAssumptionList(null),
  });
  const existing = (
    overrides.data?.status === "ok" ? overrides.data.data : []
  ).filter((a) => {
    if (a.target_entity_id !== event.source_event_id) return false;
    if (a.kind !== "bill_amount" && a.kind !== "income_amount") return false;
    try {
      const params = JSON.parse(a.params_json) as Record<string, unknown>;
      return params.effective_date === scheduledDate;
    } catch {
      return false;
    }
  });
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);

  const save = useMutation({
    mutationFn: (minor: number) => {
      // Narrowed here rather than at the top: hooks must run before the `kind === null`
      // early return, so the guard cannot precede this definition.
      if (kind === null) throw new Error("this event has no adjustable amount");
      return commands.createForecastAssumption({
        kind,
        scenario_id: null,
        target_entity_id: event.source_event_id,
        amount: null,
        date: null,
        label: null,
        // Bills and income store a POSITIVE magnitude; the row's amount is signed.
        new_amount_minor: minor,
        new_anchor_date: null,
        // The window IS the single occurrence (inclusive both ends).
        effective_date: scheduledDate,
        end_date: scheduledDate,
      });
    },
    onSuccess: (result) => {
      if (result.status === "error") {
        setError("Couldn't save that amount.");
        return;
      }
      setEditing(false);
      setError(null);
      refreshForecast();
    },
    onError: () => setError("Couldn't save that amount."),
  });

  /// Remove this occurrence's override(s), restoring the entity's base amount.
  const reset = useMutation({
    mutationFn: async () => {
      for (const a of existing) await commands.deleteForecastAssumption(a.id);
    },
    onSuccess: () => {
      setEditing(false);
      setError(null);
      refreshForecast();
    },
    onError: () => setError("Couldn't reset that amount."),
  });

  function refreshForecast() {
    // Everything derived from the forecast, plus cash availability — which honors base
    // assumptions but lives OUTSIDE the ["forecast"] prefix, so the dashboard's
    // safe-to-spend would otherwise go stale (adversarial review).
    void queryClient.invalidateQueries({ queryKey: ["forecast"] });
    void queryClient.invalidateQueries({ queryKey: ["cash-availability"] });
  }

  if (kind === null) return null;

  const current = Math.abs(event.amount.minor_units);
  if (!editing) {
    return (
      <div className="flex flex-wrap items-center gap-2 px-4 pb-3">
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            setValue((current / 100).toFixed(2));
            setEditing(true);
          }}
        >
          {existing.length > 0 ? "Change this amount" : "Adjust this amount"}
        </Button>
        {existing.length > 0 && (
          <>
            <span className="text-xs text-muted-foreground">
              Adjusted for this date
            </span>
            <Button
              variant="ghost"
              size="sm"
              disabled={reset.isPending}
              onClick={() => reset.mutate()}
            >
              {reset.isPending ? "Resetting…" : "Reset"}
            </Button>
          </>
        )}
        {error && (
          <p role="alert" className="text-xs text-loss">
            {error}
          </p>
        )}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2 px-4 pb-3">
      <Label htmlFor={`adjust-${event.source_event_id}-${scheduledDate}`}>
        Amount for this occurrence only
      </Label>
      <div className="flex items-center gap-2">
        <Input
          id={`adjust-${event.source_event_id}-${scheduledDate}`}
          inputMode="decimal"
          className="w-36 tabular-nums"
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
        <Button
          size="sm"
          disabled={save.isPending}
          onClick={() => {
            const minor = dollarsToMinorUnits(value);
            if (minor === null || minor <= 0) {
              setError("Enter an amount greater than zero.");
              return;
            }
            save.mutate(minor);
          }}
        >
          {save.isPending ? "Saving…" : "Save"}
        </Button>
        <Button variant="ghost" size="sm" onClick={() => setEditing(false)}>
          Cancel
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Applies to {scheduledDate} only — later occurrences are unchanged.
      </p>
      {error && (
        <p role="alert" className="text-xs text-loss">
          {error}
        </p>
      )}
    </div>
  );
}
