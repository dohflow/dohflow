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
    <div className="@container flex flex-col gap-2 rounded-md border border-warning/40 bg-warning/10 p-3 compact:@max-[15rem]:px-2">
      <div className="flex gap-2 compact:@max-[15rem]:flex-col">
        <TriangleAlert className="size-4 shrink-0 text-warning" aria-hidden />
        {warning && (
          <p
            data-testid="no-reset-warning"
            className="min-w-0 text-xs text-foreground [overflow-wrap:anywhere] compact:@max-[15rem]:hyphens-auto compact:@max-[15rem]:[-webkit-hyphenate-limit-before:8] compact:@max-[15rem]:[-webkit-hyphenate-limit-after:8]"
          >
            {warning}
          </p>
        )}
      </div>
      <label className="flex min-w-0 items-start gap-2 text-xs text-foreground compact:@max-[15rem]:flex-col">
        <input
          type="checkbox"
          checked={acknowledged}
          onChange={(event) => onAcknowledgedChange(event.target.checked)}
          className="mt-0.5 size-3.5 shrink-0"
        />
        <span className="min-w-0 [overflow-wrap:anywhere] compact:@max-[15rem]:hyphens-auto compact:@max-[15rem]:[-webkit-hyphenate-limit-before:8] compact:@max-[15rem]:[-webkit-hyphenate-limit-after:8]">
          I understand and have saved my password somewhere safe.
        </span>
      </label>
    </div>
  );
}
