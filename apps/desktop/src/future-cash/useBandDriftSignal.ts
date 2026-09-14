import { useQuery } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Reads the descriptive comfort-band drift signal (personal-cfo-5ie.8, ADR 0018 §915.1): why the
/// Future Cash projection is set to cross below the band, with the rising-spend categories that
/// attribute it. `null` when there's no far-horizon spending-driven crossing (or while loading).
export function useBandDriftSignal() {
  const query = useQuery({
    queryKey: queryKeys.bandDrift,
    queryFn: () =>
      ipcQuery(commands.bandDriftSignal(), "Could not load the drift signal."),
  });
  return { drift: query.data ?? null };
}
