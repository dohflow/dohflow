import { QueryClient } from "@tanstack/react-query";

/// Build the app-wide query cache (ADR 0020). A factory (not a singleton) so each
/// `App` mount — including every test render — gets an isolated cache. Defaults
/// are tuned for a local IPC backend: no auto-retry (kernel errors are
/// deterministic, not transient), no refetch on window focus (a desktop app, not
/// a browser tab), and a short stale window. The cache holds read-model data and
/// is **cleared on vault lock** (ADR 0003 — no financial data lingers after
/// lock); see `VaultProvider`.
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchOnWindowFocus: false,
        staleTime: 5_000,
      },
      mutations: {
        retry: false,
      },
    },
  });
}
