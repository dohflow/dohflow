import { useQuery } from "@tanstack/react-query";
import { TriangleAlert } from "lucide-react";

import { commands } from "@/bindings";

/// The canonical no-password-reset warning (ADR 0002 / personal-cfo-n7bo).
///
/// The copy is fetched **verbatim** from the backend — the single source of truth
/// is `vault-crypto::CANONICAL_NO_RESET_WARNING`, surfaced through the typed
/// `no_reset_warning` command — and is never re-typed here. Controlled: the parent
/// owns the `acknowledged` state and uses it to gate vault creation; checking the
/// box is what the backend records as the audit acknowledgement.
export function NoResetWarning({
  acknowledged,
  onAcknowledgedChange,
}: {
  acknowledged: boolean;
  onAcknowledgedChange: (value: boolean) => void;
}) {
  const { data: warning } = useQuery({
    queryKey: ["noResetWarning"],
    queryFn: () => commands.noResetWarning(),
    staleTime: Number.POSITIVE_INFINITY,
  });

  return (
    <div className="flex flex-col gap-2 rounded-md border border-warning/40 bg-warning/10 p-3">
      <div className="flex gap-2">
        <TriangleAlert className="size-4 shrink-0 text-warning" aria-hidden />
        {warning && (
          <p data-testid="no-reset-warning" className="text-xs text-foreground">
            {warning}
          </p>
        )}
      </div>
      <label className="flex items-start gap-2 text-xs text-foreground">
        <input
          type="checkbox"
          checked={acknowledged}
          onChange={(event) => onAcknowledgedChange(event.target.checked)}
          className="mt-0.5 size-3.5 shrink-0"
        />
        <span>I understand and have saved my password somewhere safe.</span>
      </label>
    </div>
  );
}
