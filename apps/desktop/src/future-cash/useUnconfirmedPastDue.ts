import { useQuery } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { ipcQuery } from "@/lib/query";

/// Obligations whose scheduled date has passed with nothing recorded against them
/// (personal-cfo-4d8.27.7.6, ADR 0058).
///
/// Keyed under the `["forecast"]` prefix so confirming one refreshes this list along with
/// the projection it corrects — the confirm mutation already invalidates that prefix.
export function useUnconfirmedPastDue() {
  const query = useQuery({
    queryKey: ["forecast", "unconfirmedPastDue"],
    queryFn: () =>
      ipcQuery(
        commands.unconfirmedPastDue(),
        "Could not check which bills are still unconfirmed.",
      ),
  });
  return {
    // `null` until the answer arrives — never conflate "still loading" with "nothing
    // pending", or the section would flash an all-clear it has not verified.
    occurrences: query.data ?? null,
    error: query.error?.message ?? null,
  };
}
