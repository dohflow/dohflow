import type { AssumptionEventDto } from "@/bindings";

/// One side of a conflict: an event, and where it came from.
export type ConflictSide = {
  eventId: string;
  /// `null` for a base event (an applied scenario's promotion, or an override the user
  /// set directly).
  scenarioId: string | null;
  /// The effect window, `null` on either end meaning open.
  from: string | null;
  to: string | null;
  /// The value the event asserts, for the diff. `null` when the kind carries no amount
  /// (an exclusion, a date shift).
  amountMinor: number | null;
};

/// Two events that both claim the same field of the same entity over overlapping time.
export type ScenarioConflict = {
  kind: string;
  targetEntityId: string;
  /// The side that LOSES — earlier in precedence.
  loser: ConflictSide;
  /// The side that WINS — later in precedence, and what the forecast actually uses.
  winner: ConflictSide;
};

/// The kinds that can conflict, and how to read an effect window out of their params.
///
/// Deliberately a closed list rather than "anything with a target": `one_time_event` has a
/// target of its own but two one-offs on the same day are two separate cash movements, not
/// a contradiction — summing them is correct and flagging them would be noise.
const WINDOWED_KINDS = new Set([
  "bill_amount",
  "income_amount",
  "bill_date",
  "income_date",
  "exclusion",
]);

function parseSide(event: AssumptionEventDto): ConflictSide | null {
  let params: Record<string, unknown>;
  try {
    params = JSON.parse(event.params_json) as Record<string, unknown>;
  } catch {
    // A malformed event cannot be reasoned about, so it is not reported as a conflict —
    // claiming a specific contradiction we cannot substantiate would be worse than silence.
    return null;
  }
  const str = (key: string) =>
    typeof params[key] === "string" ? (params[key] as string) : null;
  const num = (key: string) =>
    typeof params[key] === "number" ? (params[key] as number) : null;
  return {
    eventId: event.id,
    scenarioId: event.scenario_id,
    from: str("effective_date") ?? str("new_anchor_date"),
    to: str("end_date"),
    amountMinor: num("new_amount_minor"),
  };
}

/// Do two effect windows overlap? `null` is an open end.
function overlaps(a: ConflictSide, b: ConflictSide): boolean {
  const aFrom = a.from ?? "0000-01-01";
  const bFrom = b.from ?? "0000-01-01";
  const aTo = a.to ?? "9999-12-31";
  const bTo = b.to ?? "9999-12-31";
  return aFrom <= bTo && bFrom <= aTo;
}

/// Find the conflicts inside an ordered composition (personal-cfo-4d8.27.6.5, ADR 0059 §3).
///
/// **A conflict is two events on the same `(kind, target_entity_id)` with overlapping
/// effect windows** — the predicate ADR 0059 §3 defined. Two selected scenarios that both
/// change the rent conflict; one that changes the rent and one that changes the phone bill
/// do not, and neither do two changes to the same bill for windows that never overlap.
///
/// `ranked` is the composition in PRECEDENCE ORDER — base first, then each selected
/// scenario. That is what makes the winner well-defined: the later entry wins, which is
/// exactly what the forecast does (ADR 0059 §1). Conflicts are *reported*, never blocked;
/// composition proceeds and this exists so the user can see which change is in effect
/// rather than discover it in a number.
///
/// Pure: no IPC, no clock, no state. Everything it needs is in the events.
export function detectConflicts(
  ranked: { scenarioId: string | null; events: AssumptionEventDto[] }[],
): ScenarioConflict[] {
  // Flatten to (rank, event), preserving the caller's order as the precedence.
  const flat = ranked.flatMap((group, rank) =>
    group.events
      .filter((e) => WINDOWED_KINDS.has(e.kind) && e.target_entity_id !== null)
      .map((event) => ({ rank, event })),
  );

  const conflicts: ScenarioConflict[] = [];
  for (let i = 0; i < flat.length; i += 1) {
    for (let j = i + 1; j < flat.length; j += 1) {
      const a = flat[i];
      const b = flat[j];
      if (a === undefined || b === undefined) continue;
      if (a.rank === b.rank) continue; // same source — creation order already settles it
      if (a.event.kind !== b.event.kind) continue;
      if (a.event.target_entity_id !== b.event.target_entity_id) continue;

      const sideA = parseSide(a.event);
      const sideB = parseSide(b.event);
      if (sideA === null || sideB === null) continue;
      if (!overlaps(sideA, sideB)) continue;

      // Higher rank wins, matching the forecast's own composition.
      const [loser, winner] = a.rank < b.rank ? [sideA, sideB] : [sideB, sideA];
      conflicts.push({
        kind: a.event.kind,
        targetEntityId: a.event.target_entity_id as string,
        loser,
        winner,
      });
    }
  }
  return conflicts;
}
