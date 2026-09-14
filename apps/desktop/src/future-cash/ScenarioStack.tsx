import { Layers, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { NativeSelect } from "@/components/ui/native-select";
import { useScenarios } from "./useScenarios";

/// Stack further scenarios on top of the selected one (personal-cfo-4d8.27.6.4, ADR 0059).
///
/// **Order is the precedence**, so the stack is rendered as an ordered list rather than a
/// set of checkboxes: when two scenarios change the same bill, the one further down wins,
/// and a control that hid the order would leave the user unable to predict — or to see —
/// which change is in effect.
///
/// The *selected* scenario stays the one being edited; these compose over it.
export function ScenarioStack({
  primaryId,
  stacked,
  onChange,
}: {
  /// The scenario the rest stack on top of. Always first in precedence, so it loses any
  /// conflict with a stacked one.
  primaryId: string;
  /// Additional scenario ids, in precedence order (last wins).
  stacked: string[];
  onChange: (next: string[]) => void;
}) {
  const { scenarios } = useScenarios();

  const nameOf = (id: string) =>
    scenarios?.find((s) => s.id === id)?.name ?? "a deleted scenario";
  // Archived and expired scenarios are excluded: the backend drops them from the
  // selection anyway (ADR 0051), so offering one would promise a change that never lands.
  const available = (scenarios ?? []).filter(
    (s) =>
      s.id !== primaryId &&
      !stacked.includes(s.id) &&
      (s.status === "draft" || s.status === "active"),
  );

  if (available.length === 0 && stacked.length === 0) return null;

  return (
    <div className="flex flex-wrap items-center gap-2 text-sm">
      <span className="flex items-center gap-1.5 text-muted-foreground">
        <Layers className="size-4" aria-hidden />
        Also apply
      </span>

      {/* The stack, in precedence order. The numbering is the point: it says which change
          wins without the user having to open anything. */}
      {stacked.map((id, index) => (
        <span
          key={id}
          className="flex items-center gap-1 rounded-full border bg-muted/40 py-0.5 pl-2.5 pr-1 text-xs"
        >
          <span className="tabular-nums text-muted-foreground">{index + 2}.</span>
          {nameOf(id)}
          <button
            type="button"
            onClick={() => onChange(stacked.filter((s) => s !== id))}
            aria-label={`Remove ${nameOf(id)} from the stack`}
            className="rounded-full p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="size-3" aria-hidden />
          </button>
        </span>
      ))}

      {available.length > 0 && (
        <NativeSelect
          size="sm"
          aria-label="Add a scenario to the stack"
          value=""
          onChange={(e) => {
            if (e.target.value) onChange([...stacked, e.target.value]);
          }}
        >
          <option value="">Add a scenario…</option>
          {available.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}
            </option>
          ))}
        </NativeSelect>
      )}

      {stacked.length > 0 && (
        <>
          <Button size="sm" variant="ghost" onClick={() => onChange([])}>
            Clear
          </Button>
          <p className="basis-full text-xs text-muted-foreground">
            Applied in order. Where two change the same thing, the later one is what the
            forecast uses.
          </p>
        </>
      )}
    </div>
  );
}
