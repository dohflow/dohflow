import { ChevronDown, ChevronUp } from "lucide-react";

import { cn } from "@/lib/utils";

import { useScenarios } from "./useScenarios";
import { useStackedEvents } from "./useStackedEvents";
import { collisions, topFirst } from "./scenarioPile";
import { describeConflict } from "./describeConflict";
import { useBaseCurrency } from "@/settings/useBaseCurrency";

/// The stack as a pile of sheets (personal-cfo-88o4, Scenarios mock variant 1a).
///
/// The chips in `ScenarioStack` say *which* scenarios are on and in what order. This says
/// what that order **does**: each sheet carries its own changes, a change that lost is
/// struck through **inside the sheet that lost it**, and the panel underneath names every
/// contested item with the value the forecast actually uses.
///
/// Striking the loser in its own sheet is the part that earns the layout. A list of
/// winners tells you the outcome; this tells you where *your* scenario was overruled,
/// which is the question someone stacking two scenarios is actually asking.
///
/// Direction is delegated to [`topFirst`] and tested there — the pile is the precedence
/// list reversed, and getting that backwards would name the wrong winner.
export function ScenarioLayers({
  primaryId,
  stacked,
  onReorder,
}: {
  primaryId: string;
  /// Stacked ids in precedence order (last wins), exactly as the forecast receives them.
  stacked: string[];
  onReorder: (next: string[]) => void;
}) {
  const { scenarios } = useScenarios();
  const { baseCurrency } = useBaseCurrency();
  // Base sits under everything, so it is the first (weakest) entry.
  const ranked = useStackedEvents([null, primaryId, ...stacked]);
  const nameOf = (id: string | null) =>
    id === null
      ? "Your forecast"
      : (scenarios?.find((s) => s.id === id)?.name ?? "a deleted scenario");

  if (stacked.length === 0) return null;
  // Nothing rather than a half-answer: see `useStackedEvents`.
  if (ranked.ranked === null) return null;

  const layers = topFirst(ranked.ranked);
  const contested = collisions(ranked.ranked);

  /// Move a stacked scenario one step. `delta` is in PILE terms (up = stronger), so it is
  /// inverted against the precedence array — the same flip as the render, and the reason
  /// both go through named helpers rather than inline arithmetic.
  function move(id: string, towardTop: boolean) {
    const at = stacked.indexOf(id);
    if (at < 0) return;
    const to = towardTop ? at + 1 : at - 1;
    if (to < 0 || to >= stacked.length) return;
    const next = [...stacked];
    const [moved] = next.splice(at, 1);
    next.splice(to, 0, moved!);
    onReorder(next);
  }

  return (
    <div className="mt-3 rounded-lg border bg-card p-4">
      <div className="flex items-baseline justify-between gap-3">
        <h3 className="text-sm font-semibold">Stacked on your forecast</h3>
        <span className="text-xs text-muted-foreground">Top sheet wins</span>
      </div>
      <p className="mt-1 max-w-lg text-xs text-muted-foreground">
        Each sheet sits over the one below it. Where two change the same thing, the sheet
        nearer the top is what the forecast uses.
      </p>

      {/* Named, because "list of 3 items" says nothing about what the order means — and
          here the order IS the meaning. */}
      <ul aria-label="Scenario stack, strongest first" className="mt-3 flex flex-col">
        {layers.map((layer) => {
          const movable = layer.scenarioId !== null && stacked.includes(layer.scenarioId);
          return (
            <li
              key={layer.scenarioId ?? "base"}
              className={cn(
                "rounded-md border px-3 py-2.5",
                // Overlapped, so the pile reads as sheets rather than as a list.
                "-mt-px first:mt-0",
                layer.isTop ? "border-primary/40 bg-primary/5" : "bg-background",
              )}
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <span className="text-sm font-medium">{nameOf(layer.scenarioId)}</span>
                  {layer.isTop && (
                    <span className="ml-2 text-xs text-primary">wins ties</span>
                  )}
                </div>
                {movable && (
                  <div className="flex shrink-0 gap-1">
                    <button
                      type="button"
                      aria-label={`Move ${nameOf(layer.scenarioId)} up`}
                      onClick={() => move(layer.scenarioId!, true)}
                      className="rounded border p-0.5 text-muted-foreground hover:text-foreground"
                    >
                      <ChevronUp className="size-3.5" aria-hidden />
                    </button>
                    <button
                      type="button"
                      aria-label={`Move ${nameOf(layer.scenarioId)} down`}
                      onClick={() => move(layer.scenarioId!, false)}
                      className="rounded border p-0.5 text-muted-foreground hover:text-foreground"
                    >
                      <ChevronDown className="size-3.5" aria-hidden />
                    </button>
                  </div>
                )}
              </div>

              {layer.changes.length === 0 ? (
                <p className="mt-1 text-xs text-muted-foreground">No changes yet.</p>
              ) : (
                // Plain rows, not a nested list: a sheet's changes are its contents, and
                // announcing "list within list" adds a level of structure the reader has
                // to unpack for no gain. The sheet is the list item.
                <div className="mt-1 flex flex-col gap-0.5">
                  {layer.changes.map((change) => (
                    <div
                      key={change.event.id}
                      className={cn(
                        "flex items-baseline justify-between gap-3 text-xs",
                        change.overruled
                          ? "text-muted-foreground line-through"
                          : "text-foreground",
                      )}
                    >
                      <span className="truncate">{change.event.kind}</span>
                      {change.overruled && (
                        // Not struck silently — the sheet that took it is named, so the
                        // reader can act on it rather than just notice it.
                        <span className="shrink-0 no-underline">
                          overruled by {nameOf(change.overruledBy)}
                        </span>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </li>
          );
        })}
      </ul>

      {contested.length > 0 && (
        <div className="mt-4 rounded-md border bg-background p-3">
          <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Where they collide
          </p>
          <ul className="mt-1.5 flex flex-col gap-1">
            {contested.map((c) => (
              <li
                key={`${c.loser.eventId}:${c.winner.eventId}`}
                className="text-xs text-muted-foreground"
              >
                {describeConflict(c, baseCurrency)}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
