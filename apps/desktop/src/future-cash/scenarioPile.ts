import type { AssumptionEventDto } from "@/bindings";

import { detectConflicts, type ScenarioConflict } from "./scenarioConflicts";

/// The stack as a physical pile (personal-cfo-88o4, from the Scenarios mock).
///
/// Named `scenarioPile` rather than `scenarioLayers` on purpose: `ScenarioLayers.tsx` is
/// the component that renders this, and two modules differing only in leading case are the
/// SAME file on a case-insensitive filesystem — which is most of this project's machines.
///
/// # The direction is the whole risk
///
/// The shipped precedence is **last wins**: callers pass `[primary, ...stacked]` and a
/// later entry overrules an earlier one (ADR 0059, and `scenarioConflicts` documents the
/// winner as "later in precedence"). A pile reads the other way round — the **top sheet
/// wins** — so the pile is the precedence list **reversed**.
///
/// Rendering it un-reversed would confidently name the wrong scenario as winning, on the
/// one surface whose entire job is answering which change is in effect. `topFirst` exists
/// so that reversal happens exactly once, here, with a test on it — rather than as a
/// `.reverse()` buried in JSX where the next edit can silently drop it.
export type ScenarioLayer = {
  /// `null` for the base forecast, which is always the bottom sheet.
  scenarioId: string | null;
  /// This layer's own events, each marked with whether it survived the stack.
  changes: LayerChange[];
  /// Strongest sheet in the pile — the top one. Exactly one layer has this.
  isTop: boolean;
};

export type LayerChange = {
  event: AssumptionEventDto;
  /// True when a higher sheet overrules this change. Struck through in the UI: the reader
  /// needs to see not just who won, but where their OWN scenario was overruled.
  overruled: boolean;
  /// The scenario that overruled it, when it lost. `null` means base won.
  overruledBy: string | null;
};

/// Build the pile, top sheet first.
///
/// `ranked` is in the shipped weakest-to-strongest order — pass it exactly as the forecast
/// receives it, and let this function do the flip.
export function topFirst(
  ranked: { scenarioId: string | null; events: AssumptionEventDto[] }[],
): ScenarioLayer[] {
  const conflicts = detectConflicts(ranked);
  // Index the loser side by event id. `detectConflicts` returns winner/loser PAIRS, but a
  // sheet needs to know which of ITS OWN changes lost — so the pairs become a per-event
  // lookup rather than being re-derived per row.
  //
  // An event can lose MORE THAN ONCE: three sheets all changing rent produce three
  // pairwise conflicts, and the base sheet's change loses to both of the others. Only the
  // STRONGEST winner is the one the forecast actually uses, so rank decides — taking
  // whichever pair happened to come last would name a scenario that is itself overruled.
  const rankOf = new Map<string | null, number>(
    ranked.map((group, index) => [group.scenarioId, index]),
  );
  const overruledBy = new Map<string, { scenarioId: string | null; rank: number }>();
  for (const c of conflicts) {
    const rank = rankOf.get(c.winner.scenarioId) ?? -1;
    const held = overruledBy.get(c.loser.eventId);
    if (held === undefined || rank > held.rank) {
      overruledBy.set(c.loser.eventId, { scenarioId: c.winner.scenarioId, rank });
    }
  }

  const layers = ranked.map((group, index) => ({
    scenarioId: group.scenarioId,
    changes: group.events.map((event) => ({
      event,
      overruled: overruledBy.has(event.id),
      overruledBy: overruledBy.get(event.id)?.scenarioId ?? null,
    })),
    // Strongest is LAST in the ranked order — before the flip.
    isTop: index === ranked.length - 1,
  }));

  return layers.reverse();
}

/// The contested items, for the "where they collide" panel.
///
/// Same conflicts the layers are marked from, so the panel and the strike-throughs cannot
/// disagree — one computation, two readings.
export function collisions(
  ranked: { scenarioId: string | null; events: AssumptionEventDto[] }[],
): ScenarioConflict[] {
  return detectConflicts(ranked);
}
