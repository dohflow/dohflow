import { CheckCircle2, Loader2, X } from "lucide-react";

import { Button } from "@/components/ui/button";

/// The dismissible "Marked … paid · Undo" bar shown in Future Cash after a mark-paid
/// (personal-cfo-5ie.9). Undo reverses the confirm (voids the payment and re-projects the
/// occurrence); an error keeps the bar so the user can retry.
export function MarkedPaidUndoBar({
  name,
  onUndo,
  onDismiss,
  pending,
  error,
}: {
  name: string;
  onUndo: () => void;
  onDismiss: () => void;
  pending: boolean;
  error: string | null;
}) {
  return (
    <div
      role="status"
      className="flex items-center gap-3 rounded-lg border border-primary/30 bg-primary/5 px-4 py-3"
    >
      <CheckCircle2 className="size-5 shrink-0 text-primary" aria-hidden />
      <p className="flex-1 text-sm">
        {error ? (
          <span className="text-loss">{error}</span>
        ) : (
          <>
            Marked <span className="font-medium">{name || "bill"}</span> paid.
          </>
        )}
      </p>
      <Button variant="outline" size="sm" onClick={onUndo} disabled={pending}>
        {pending ? <Loader2 className="animate-spin" aria-hidden /> : null}
        Undo
      </Button>
      <Button
        variant="ghost"
        size="icon"
        className="size-8 shrink-0"
        aria-label="Dismiss"
        onClick={onDismiss}
      >
        <X className="size-4" aria-hidden />
      </Button>
    </div>
  );
}
