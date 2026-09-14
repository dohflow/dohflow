import { useQueries } from "@tanstack/react-query";

import { commands, type AssumptionEventDto } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Every sheet's events at once, in precedence order (personal-cfo-88o4).
///
/// The layers view needs ALL the stack's events together — conflicts are cross-sheet, so
/// no per-sheet component can decide on its own whether its change survived.
/// `useScenarioEvents` loads one scenario, and calling it in a loop would break the rules
/// of hooks, so this uses `useQueries` (its first use in the repo) to fan out over a list
/// whose length changes as the user stacks and unstacks.
///
/// Keys and query fn match `useScenarioEvents` exactly, so the two share a cache entry
/// rather than double-fetching the same scenario.
export function useStackedEvents(scenarioIds: (string | null)[]): {
  /// Groups in the SAME order as `scenarioIds` — weakest to strongest, last wins. The
  /// pile flips this exactly once, in `topFirst`.
  ranked: { scenarioId: string | null; events: AssumptionEventDto[] }[] | null;
} {
  const results = useQueries({
    queries: scenarioIds.map((scenarioId) => ({
      queryKey: queryKeys.scenarioEvents(scenarioId),
      queryFn: () =>
        ipcQuery(
          commands.forecastAssumptionList(scenarioId),
          "Could not load this scenario's changes.",
        ),
    })),
  });

  // All or nothing. A partly-loaded stack would compute conflicts against events that have
  // not arrived, and report a change as surviving that a still-loading sheet overrules —
  // a wrong answer that looks exactly like a right one.
  if (results.some((r) => r.data === undefined)) return { ranked: null };

  return {
    ranked: scenarioIds.map((scenarioId, i) => ({
      scenarioId,
      events: (results[i]?.data ?? []) as AssumptionEventDto[],
    })),
  };
}
