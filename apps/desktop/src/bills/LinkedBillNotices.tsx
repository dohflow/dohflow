import { AlertTriangle } from "lucide-react";

import { belowScheduledMessage, cadenceChangedMessage } from "./linkMessages";
import { useBills } from "./useBills";

/// The linked bills whose link needs the user's attention, on Bills and Cash Flow (personal-cfo-6wk.22,
/// planning): a bill that no longer repeats like its loan's payment (so both now count), and a linked
/// bill below the loan's scheduled payment (the link stays; the amount is the user's truth). The
/// messages are the approved sentences; the bill's name is a label above each. Renders nothing when
/// every link is doing its job.
export function LinkedBillNotices() {
  const { bills } = useBills();
  const notices = (bills ?? []).flatMap((bill) => {
    const link = bill.loan_link;
    if (!link) return [];
    const messages: string[] = [];
    if (link.cadence_changed) messages.push(cadenceChangedMessage(link.loan_name));
    if (link.below_scheduled) messages.push(belowScheduledMessage(link.loan_name));
    return messages.length > 0 ? [{ bill, messages }] : [];
  });
  if (notices.length === 0) return null;

  return (
    <div
      role="region"
      aria-label="Linked bills"
      className="flex flex-col gap-2 rounded-md border border-warning/40 bg-warning/5 p-3 text-sm"
    >
      {notices.map(({ bill, messages }) => (
        <div key={bill.id} className="flex items-start gap-2">
          <AlertTriangle className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden />
          <div>
            <div className="font-medium">{bill.name}</div>
            {messages.map((message) => (
              <div key={message} className="text-muted-foreground">
                {message}
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
