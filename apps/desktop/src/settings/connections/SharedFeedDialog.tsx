// One account, one feed by default (personal-cfo-6evt, ADR 0014 §3 addendum
// 2026-10-04). Mapping a connector account onto an account that another
// connector link already updates asks first. A household can connect the same
// bank twice (one connection per person); when both can see a joint account,
// importing it from only one of them is the safe default.
import { useEffect } from "react";
import { TriangleAlert, X } from "lucide-react";

import { Button } from "@/components/ui/button";

export type ExistingFeed = {
  /// The connection's label, e.g. "Alex's bank" or "SimpleFIN connection".
  connection: string;
  /// The provider's name for the account that feeds it, if known.
  account: string | null;
};

export function SharedFeedDialog({
  accountName,
  externalName,
  existingFeeds,
  busy,
  onDontImport,
  onChooseOther,
  onImportBoth,
}: {
  /// The ledger account the user picked.
  accountName: string;
  /// The provider's name for the account being mapped.
  externalName: string;
  existingFeeds: ExistingFeed[];
  busy: boolean;
  /// Leave this provider account unmapped: it stays listed and feeds nothing.
  onDontImport: () => void;
  /// Close without changing anything, back to the account picker.
  onChooseOther: () => void;
  /// Save the mapping anyway; overlaps are flagged for review.
  onImportBoth: () => void;
}) {
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      // Escape is "change nothing", never a choice the user didn't make.
      if (event.key === "Escape" && !busy) onChooseOther();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onChooseOther, busy]);

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center p-4">
      <div
        className="absolute inset-0 bg-foreground/40"
        aria-hidden
        onClick={busy ? undefined : onChooseOther}
      />
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="shared-feed-title"
        aria-describedby="shared-feed-body"
        className="relative flex w-full max-w-md flex-col gap-4 rounded-lg border bg-background p-6 shadow-xl"
      >
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-start gap-2">
            <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden />
            <h2 id="shared-feed-title" className="font-semibold">
              {accountName} is already updated by another connection
            </h2>
          </div>
          <button
            type="button"
            onClick={onChooseOther}
            disabled={busy}
            aria-label="Close"
            className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="size-4" aria-hidden />
          </button>
        </div>
        <div id="shared-feed-body" className="flex flex-col gap-2 text-sm">
          <ul aria-label="Already updating this account" className="list-disc pl-5">
            {existingFeeds.map((feed, index) => (
              <li key={index}>
                {feed.connection}
                {feed.account ? ` — ${feed.account}` : ""}
              </li>
            ))}
          </ul>
          <p className="text-muted-foreground">
            If &ldquo;{externalName}&rdquo; is the same bank account (a joint
            account you can both see, say), import it from one connection only.
            Importing from both is allowed: any transaction that appears in both
            waits in the Money Inbox for you to review, and is never counted
            twice.
          </p>
        </div>
        <div className="flex flex-col gap-2">
          <Button autoFocus disabled={busy} onClick={onDontImport}>
            Don&rsquo;t import this one
          </Button>
          <Button variant="outline" disabled={busy} onClick={onChooseOther}>
            Map it to a different account
          </Button>
          <Button variant="outline" disabled={busy} onClick={onImportBoth}>
            Import from both
          </Button>
        </div>
      </div>
    </div>
  );
}
