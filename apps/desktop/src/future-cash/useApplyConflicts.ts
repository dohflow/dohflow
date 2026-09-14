import { useQuery } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { ipcQuery } from "@/lib/query";

import { detectConflicts, type ScenarioConflict } from "./scenarioConflicts";

/// The conflicts applying `scenarioId` would create against what the forecast already
/// assumes (personal-cfo-4d8.27.6.5, ADR 0059 §3).
///
/// Apply is where a conflict stops being a view and becomes durable: composition is a
/// read-time overlay that the user can undo by deselecting, but applying **promotes events
/// into base** (ADR 0055) and supersedes whatever they collide with. So the diff is shown
/// here, before the write, rather than only on the compose path.
///
/// The comparison is base-then-scenario, which is exactly the precedence the forecast uses
/// (ADR 0059 §2: base ranks below every scenario), so the "winner" this reports is the
/// value the forecast will actually use afterwards.
export function useApplyConflicts(scenarioId: string | null) {
  const base = useQuery({
    queryKey: ["forecast", "assumptions", "base"],
    queryFn: () =>
      ipcQuery(commands.forecastAssumptionList(null), "Could not load your assumptions."),
    enabled: scenarioId !== null,
  });
  const scenario = useQuery({
    queryKey: ["forecast", "assumptions", scenarioId],
    queryFn: () =>
      ipcQuery(
        commands.forecastAssumptionList(scenarioId),
        "Could not load the scenario's changes.",
      ),
    enabled: scenarioId !== null,
  });

  // `null` until BOTH sides have arrived — reporting "no conflicts" from a half-loaded
  // comparison would be an all-clear the check has not actually performed.
  const ready = base.data !== undefined && scenario.data !== undefined;
  const conflicts: ScenarioConflict[] | null = ready
    ? detectConflicts([
        { scenarioId: null, events: base.data ?? [] },
        { scenarioId, events: scenario.data ?? [] },
      ])
    : null;

  return { conflicts };
}
