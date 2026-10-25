import { useId } from "react";

import { Button } from "@/components/ui/button";

/// The inline confirm for a REVERSIBLE remove (docs/agent/FRONTEND.md section 6.2): "Remove
/// <name>?" with "You can bring it back." The prompt names the thing, so a screen reader hears
/// what it is confirming; Cancel takes focus, so the safe choice is the default.
export function RemoveConfirm({
  name,
  busy,
  onConfirm,
  onCancel,
}: {
  /// What is being removed, as it reads in the question: "Rent", "transfer from A to B".
  name: string;
  busy: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const promptId = useId();
  const hintId = useId();
  return (
    <div
      role="group"
      aria-labelledby={promptId}
      aria-describedby={hintId}
      className="flex items-center gap-2"
    >
      <div className="text-right text-xs">
        <div id={promptId} className="font-medium text-foreground">{`Remove ${name}?`}</div>
        <div id={hintId} className="text-muted-foreground">
          You can bring it back.
        </div>
      </div>
      <Button
        size="sm"
        variant="ghost"
        disabled={busy}
        onClick={onConfirm}
        aria-label={`Confirm remove ${name}`}
      >
        Remove
      </Button>
      <Button size="sm" variant="ghost" type="button" autoFocus onClick={onCancel}>
        Cancel
      </Button>
    </div>
  );
}
