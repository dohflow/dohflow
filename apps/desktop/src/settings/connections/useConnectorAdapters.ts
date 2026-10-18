// The connector registry the provider picker renders from (personal-cfo-dto2j,
// ADR 0015 / ADR 0076 §4). Static, compiled-in configuration: the list arrives
// already in the ADR 0076 order (the launch provider first, then alphabetical),
// disabled providers included and flagged. The picker shows enabled ones only.

import { useQuery } from "@tanstack/react-query";

import { commands, type ConnectorAdapterDto } from "@/bindings";
import { queryKeys } from "@/lib/query";

export interface UseConnectorAdapters {
  /// Every registered provider, in the registry's order; null while loading.
  adapters: ConnectorAdapterDto[] | null;
  /// The providers a user can connect today (ADR 0015 §6: `enabled`).
  enabled: ConnectorAdapterDto[] | null;
  error: string | null;
  /// Display name for a stored connection's adapter id — falls back to the
  /// id itself for a provider this build no longer registers.
  displayName: (adapterId: string) => string;
  /// The registry entry for an adapter id, if this build registers it.
  adapter: (adapterId: string) => ConnectorAdapterDto | null;
}

export function useConnectorAdapters(): UseConnectorAdapters {
  const query = useQuery({
    queryKey: queryKeys.connectorAdapters,
    queryFn: () => commands.connectorAdapters(),
    staleTime: Infinity,
  });
  const adapters = query.data ?? null;
  return {
    adapters,
    enabled: adapters ? adapters.filter((adapter) => adapter.enabled) : null,
    error: query.error ? "Could not load connection providers." : null,
    displayName: (adapterId) =>
      adapters?.find((adapter) => adapter.adapter_id === adapterId)?.display_name ??
      adapterId,
    adapter: (adapterId) =>
      adapters?.find((adapter) => adapter.adapter_id === adapterId) ?? null,
  };
}
