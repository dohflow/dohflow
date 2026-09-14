import { useState } from "react";
import { Loader2 } from "lucide-react";

import { Card, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { describeIpcError } from "@/vault/useVault";
import { useAutoCategorizeOnImport } from "./useAutoCategorizeOnImport";

/// Toggles whether merchant memory auto-applies after an import (ADR 0030 addendum,
/// personal-cfo-5n4.2). On (the default) means freshly imported transactions are
/// categorized from what you've taught the app, leaving conflicted/unknown merchants
/// for review. A native checkbox keeps it dep-free + accessible; IPC flows through the
/// generated `commands` only (ADR 0003).
export function AutoCategorizeOnImportCard() {
  const { enabled, error: loadError, setEnabled } = useAutoCategorizeOnImport();
  // Optimistic value so the toggle doesn't flicker back while the write + refetch
  // are in flight; cleared (falls back to the query value) on failure.
  const [pending, setPending] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const value = pending ?? enabled;

  async function onToggle(next: boolean) {
    setPending(next);
    setSaving(true);
    setError(null);
    const failure = await setEnabled(next);
    setSaving(false);
    if (failure) {
      setPending(null);
      setError(describeIpcError(failure));
    } else {
      setPending(null); // fall back to the freshly-refetched stored value
    }
  }

  return (
    <Card>
      <CardContent className="pt-6">
        <div className="flex items-start justify-between gap-4">
          <div className="flex flex-col gap-1">
            <Label htmlFor="auto-categorize-on-import">
              Auto-categorize imported transactions
            </Label>
            <p className="text-xs text-muted-foreground">
              When you import a file, fill in categories the app has learned from
              your past choices. Only uncategorized transactions are touched, never
              your own — and you can change any of them.
            </p>
            {saving && (
              <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Loader2 className="size-3.5 animate-spin" aria-hidden />
                Saving…
              </p>
            )}
            {(error ?? loadError) && (
              <p role="alert" className="text-xs text-loss">
                {error ?? loadError}
              </p>
            )}
          </div>
          <input
            id="auto-categorize-on-import"
            type="checkbox"
            role="switch"
            checked={value}
            disabled={saving}
            onChange={(event) => void onToggle(event.target.checked)}
            className="mt-1 size-5 shrink-0 cursor-pointer accent-primary"
          />
        </div>
      </CardContent>
    </Card>
  );
}
