import { AlertTriangle } from "lucide-react";

import { Button } from "@/components/ui/button";

/// The debt terms behind the stat row and the balances-and-terms table could not be read. The
/// message is the read's own; until a read works those two stay absent rather than showing totals,
/// zeroes or "no terms" that nobody has checked. Retry reads the terms again and writes nothing.
export const TermsReadNotice = ({
  message,
  retry,
  retrying,
}: {
  message: string;
  retry: () => void;
  retrying: boolean;
}) => (
  <div
    role="alert"
    className="flex flex-wrap items-center gap-3 rounded-lg border border-loss/30 bg-loss/[0.08] p-4"
  >
    <AlertTriangle className="size-4 shrink-0 text-loss" aria-hidden />
    <p className="flex-1 text-sm font-semibold text-loss">{message}</p>
    <Button variant="outline" size="sm" onClick={retry} disabled={retrying}>
      {retrying ? "Retrying…" : "Retry"}
    </Button>
  </div>
);
