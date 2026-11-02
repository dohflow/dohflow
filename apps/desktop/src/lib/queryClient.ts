import { onlineManager, QueryClient } from "@tanstack/react-query";

/// TanStack Query's `online` network mode pauses every query and mutation while
/// `onlineManager` says offline, and by default that follows the OS's offline
/// event. Every read and write here is local IPC, so a laptop in airplane mode must
/// not leave screens on a spinner. The manager is therefore cut off from the OS: it
/// means "the vault can take a command". The key rotation in `VaultProvider` is the
/// only writer, and it uses that to pause polling while it holds the vault lock
/// (ADR 0083 §5). A future network-backed query (Sync, connectors) sets its own
/// `networkMode` and checks connectivity itself.
///
/// `onlineManager` is a process-wide singleton, so this installs once however many
/// clients are built.
let osConnectivityIgnored = false;
function ignoreOsConnectivity(): void {
  if (osConnectivityIgnored) return;
  osConnectivityIgnored = true;
  onlineManager.setEventListener(() => () => {});
}

/// Build the app-wide query cache (ADR 0020). A factory (not a singleton) so each
/// `App` mount — including every test render — gets an isolated cache. Defaults
/// are tuned for a local IPC backend: no auto-retry (kernel errors are
/// deterministic, not transient), no refetch on window focus (a desktop app, not
/// a browser tab), and a short stale window. The cache holds read-model data and
/// is **cleared on vault lock** (ADR 0003 — no financial data lingers after
/// lock); see `VaultProvider`.
export function createQueryClient(): QueryClient {
  ignoreOsConnectivity();
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
