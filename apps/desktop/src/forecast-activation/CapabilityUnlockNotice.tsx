import { Sparkles, X } from "lucide-react";

import { Button } from "@/components/ui/button";

import { useCapabilityUnlocks } from "./useCapabilityUnlocks";

/// A dismissible "your forecast grew" notice (ADR 0026 §10, personal-cfo-egon). Shows one
/// card per capability that has self-activated; dismissing acknowledges it so it never
/// shows again. Rendered in the app shell, so it appears above whatever tab is open and
/// communicates the "it grows with you" moment the data-progressive design promises.
export function CapabilityUnlockNotice() {
  const { unlocks, acknowledge } = useCapabilityUnlocks();
  if (unlocks.length === 0) return null;

  return (
    <div className="mb-6 space-y-3">
      {unlocks.map((unlock) => (
        <div
          key={unlock.key}
          role="status"
          className="flex items-start gap-3 rounded-lg border border-primary/30 bg-primary/5 px-4 py-3"
        >
          <Sparkles className="mt-0.5 size-5 shrink-0 text-primary" aria-hidden />
          <div className="flex-1">
            <p className="font-medium text-foreground">{unlock.title}</p>
            <p className="mt-0.5 text-sm text-muted-foreground">{unlock.body}</p>
          </div>
          <Button
            variant="ghost"
            size="icon"
            className="size-8 shrink-0"
            aria-label={`Dismiss: ${unlock.title}`}
            onClick={() => void acknowledge(unlock.key)}
          >
            <X className="size-4" aria-hidden />
          </Button>
        </div>
      ))}
    </div>
  );
}
