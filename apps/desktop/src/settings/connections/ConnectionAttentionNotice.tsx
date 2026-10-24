import { ExternalLink, Loader2, TriangleAlert } from "lucide-react";
import { useId, useState } from "react";

import { commands, type ConnectorConnectionDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { describeIpcError } from "@/vault/useVault";

import { syncOutcomeCopy } from "../connectorSync";
import { useConnections } from "../useConnections";
import { attentionMessage, providerSiteName } from "./attentionCopy";

/// How often the notice re-reads the connection list (milliseconds): a local, cheap read, and the
/// only way the shell learns what a background refresh just wrote.
const POLL_MS = 60_000;

/// "Sign in again" notice for bank connections that need the user at the provider
/// (personal-cfo-pxi.22; ADR 0060 addendum 2026-10-23). Rendered once in the app shell, so it sits
/// above whatever section is open, not only Settings. One card per affected connection; it shows
/// ONLY structured `attention` from the backend (never warning prose), names only the bank, and
/// goes away by itself when a healthy refresh clears the state. It cannot be dismissed: stale bank
/// data silently degrades the forecast, so the way out is to fix it or refresh.
///
/// States: loading and a failed load render nothing (a notice that is absent has nothing to say;
/// Settings > Connections shows the load error and the full list); populated renders the cards;
/// "Refresh now" shows pending and its outcome; a failure to open the provider's site is announced.
/// Works offline (the state is local); a refresh made offline reports its own failure.
export function ConnectionAttentionNotice() {
  const { connections, sync, syncingId } = useConnections({ pollMs: POLL_MS });
  const affected = (connections ?? []).filter(
    (connection) => linesFor(connection).length > 0,
  );
  if (affected.length === 0) return null;

  return (
    <section aria-label="Bank connections that need you" className="mb-6 space-y-3">
      {affected.map((connection) => (
        <AttentionCard
          key={connection.id}
          connection={connection}
          refreshing={syncingId === connection.id}
          onRefresh={sync}
        />
      ))}
    </section>
  );
}

function linesFor(connection: ConnectorConnectionDto): string[] {
  return connection.attention
    .map((entry) => attentionMessage(connection.adapter_id, entry))
    .filter((line): line is string => line !== null);
}

function AttentionCard({
  connection,
  refreshing,
  onRefresh,
}: {
  connection: ConnectorConnectionDto;
  refreshing: boolean;
  onRefresh: (connectionId: string) => ReturnType<
    ReturnType<typeof useConnections>["sync"]
  >;
}) {
  const messageId = useId();
  const [outcome, setOutcome] = useState<string | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const lines = linesFor(connection);
  const site = providerSiteName(connection.adapter_id);

  async function refresh() {
    setOutcome(null);
    setOpenError(null);
    const result = await onRefresh(connection.id);
    setOutcome(
      typeof result === "object" && "status" in result
        ? syncOutcomeCopy(result.status, result.message)
        : describeIpcError(result),
    );
  }

  async function openSite() {
    setOpenError(null);
    setOutcome(null);
    // Only the connection is named: the backend looks the address up (ADR 0060 addendum).
    const result = await commands.openProviderSite({ connection_id: connection.id });
    if (result.status === "error") setOpenError(describeIpcError(result.error));
  }

  return (
    <div
      role="alert"
      aria-describedby={messageId}
      className="flex items-start gap-3 rounded-lg border border-warning/40 bg-warning/10 px-4 py-3"
    >
      <TriangleAlert className="mt-0.5 size-5 shrink-0 text-warning" aria-hidden />
      <div className="min-w-0 flex-1">
        <div id={messageId} className="space-y-1">
          {lines.map((line) => (
            <p key={line} className="font-medium text-foreground">
              {line}
            </p>
          ))}
        </div>
        <div className="mt-3 flex flex-wrap items-center gap-2">
          {site ? (
            <Button size="sm" variant="secondary" onClick={() => void openSite()}>
              <ExternalLink aria-hidden />
              Open {site}
            </Button>
          ) : null}
          <Button
            size="sm"
            variant="secondary"
            disabled={refreshing}
            onClick={() => void refresh()}
          >
            {refreshing ? <Loader2 className="animate-spin" aria-hidden /> : null}
            Refresh now
          </Button>
        </div>
        {/* The card is this notice's ONE live region (role="alert"): the refresh outcome and an
            open-site failure are added inside it, so they are announced without a nested live
            region of their own (a nested status or alert would announce them twice). */}
        {outcome ? (
          <p className="mt-2 text-sm text-muted-foreground">{outcome}</p>
        ) : null}
        {openError ? <p className="mt-2 text-sm text-loss">{openError}</p> : null}
      </div>
    </div>
  );
}
