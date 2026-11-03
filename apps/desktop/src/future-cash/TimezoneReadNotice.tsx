import { Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";

/// Which scenarios are still in the forecast depends on the household's calendar date, so until
/// the household timezone has been read (just now, successfully) the scenario controls say what
/// they are waiting for instead of guessing from the UTC fallback (personal-cfo-5ie.18, AC8).
/// A failed read says so and offers to read it again; a read that is still pending, or paused
/// (offline, or a key rotation), shows the loading line. The read is the household-timezone read
/// only: Retry never writes a setting.
export function TimezoneReadNotice({
  failure,
  retry,
  retrying,
}: {
  failure: string | null;
  retry: () => void;
  retrying: boolean;
}) {
  if (failure) {
    return (
      <div className="flex items-center gap-2">
        <p role="alert" className="text-sm text-loss">
          {failure}
        </p>
        <Button variant="outline" size="sm" onClick={retry} disabled={retrying}>
          {retrying ? "Retrying…" : "Retry"}
        </Button>
      </div>
    );
  }
  return (
    <p
      role="status"
      className="flex items-center gap-2 text-sm text-muted-foreground"
    >
      <Loader2 className="size-4 animate-spin" aria-hidden />
      Loading…
    </p>
  );
}
