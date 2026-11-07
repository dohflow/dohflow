import { useRef } from "react";

import type { AccountViewDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogError,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useCompactLayout } from "@/lib/compact";
import { COMPACT_FULLSCREEN } from "@/lib/sheet";
import { formatMoney } from "@/lib/format";

import { storedToShownMinor } from "./balanceSign";
import type { ArchiveFlow } from "./archiveFlow";

/// The question's words, exactly as the desktop's inline notice has always said them.
function impactSentence(account: AccountViewDto): string {
  const shown = formatMoney({
    minor_units: storedToShownMinor(account.cashflow_role, account.balance.minor_units),
    currency: account.balance.currency,
  });
  return `Archiving ${account.name} stops counting its ${shown} toward your forecast, so your projected cash will drop by that much. Restoring the account brings it back.`;
}

/// What archiving this account does to the forecast, and the choice to go ahead. Renders only while
/// the flow is confirming. On a phone it is a full-screen sheet (`PhoneArchiveConfirm`); elsewhere the inline
/// notice under the row.
export function ArchiveImpactConfirm({
  account,
  flow,
  confirmDisabled = false,
  triggerRef,
}: {
  account: AccountViewDto;
  flow: ArchiveFlow;
  /// Holds the Archive button while the account figure shown above cannot be trusted (a read of the
  /// accounts that failed after the question opened). Cancel stays available.
  confirmDisabled?: boolean;
  /// The control that opened the question, so closing returns focus to exactly it (a phone does not focus a tapped button).
  triggerRef?: React.RefObject<HTMLElement | null>;
}) {
  const compact = useCompactLayout();
  if (compact) {
    return <PhoneArchiveConfirm account={account} flow={flow} confirmDisabled={confirmDisabled} triggerRef={triggerRef} />;
  }
  if (!flow.confirming) return null;
  return (
    <div
      role="alertdialog"
      aria-label={`Archive ${account.name}`}
      className="mt-2 rounded-md border bg-muted/40 p-3 text-sm"
    >
      <p>{impactSentence(account)}</p>
      <div className="mt-2 flex gap-1">
        <Button
          size="sm"
          variant="ghost"
          onClick={flow.confirm}
          disabled={confirmDisabled}
          aria-label={`Confirm archive ${account.name}`}
        >
          Archive
        </Button>
        <Button size="sm" variant="ghost" type="button" onClick={flow.cancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

/// The phone's archive question (personal-cfo-i93n3.28; AA-1 AA04). A full-screen alert dialog over the page, which is
/// inert and hidden from assistive technology behind it. **Cancel comes first and takes focus**, so the safe choice
/// is the default; Archive is the second and only other action. While the archive runs both are held and the sheet
/// cannot be dismissed (Escape and a tap outside do nothing), because a begun write is never reversed by closing. A
/// failure stays in the sheet's own announced error (`DialogError`) until the next attempt or Cancel, and the sheet
/// stays open. Closing returns focus to the control that opened it, or, when the archive removed that row, to the
/// nearest row that survives, else the section's own toggle.
function PhoneArchiveConfirm({
  account,
  flow,
  confirmDisabled,
  triggerRef,
}: {
  account: AccountViewDto;
  flow: ArchiveFlow;
  confirmDisabled: boolean;
  triggerRef?: React.RefObject<HTMLElement | null>;
}) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const fallbackRef = useRef<HTMLElement | null>(null);
  return (
    <Dialog open={flow.confirming} onOpenChange={(open) => !open && flow.dismissSheet()}>
      <DialogContent
        role="alertdialog"
        aria-label={`Archive ${account.name}`}
        className={COMPACT_FULLSCREEN}
        initialFocusRef={cancelRef}
        restoreFocusTo={triggerRef}
        fallbackFocusTo={fallbackRef}
        onOpenAutoFocus={() => {
          // Remember where focus can go if the archive removes this row: the next row, else the previous, else the section's toggle.
          const row = triggerRef?.current?.closest("li");
          const survivor =
            row?.nextElementSibling?.querySelector<HTMLElement>("button") ??
            row?.previousElementSibling?.querySelector<HTMLElement>("button") ??
            row?.closest("ul")?.parentElement?.querySelector<HTMLElement>("button[aria-expanded]") ??
            null;
          fallbackRef.current = survivor;
        }}
        onEscapeKeyDown={(event) => flow.busy && event.preventDefault()}
        onPointerDownOutside={(event) => event.preventDefault()}
        onInteractOutside={(event) => event.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle>{`Archive ${account.name}`}</DialogTitle>
        </DialogHeader>
        <DialogBody>
          <DialogDescription>{impactSentence(account)}</DialogDescription>
        </DialogBody>
        <DialogError error={flow.error} />
        <DialogFooter className="compact:flex-col-reverse compact:[&>button]:w-full">
          <Button
            ref={cancelRef}
            type="button"
            variant="outline"
            disabled={flow.busy}
            onClick={flow.cancel}
          >
            Cancel
          </Button>
          <Button
            type="button"
            disabled={confirmDisabled || flow.busy}
            aria-busy={flow.busy}
            aria-label={`Confirm archive ${account.name}`}
            onClick={() => void flow.confirmInSheet()}
          >
            {flow.busy ? "Archiving…" : "Archive"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// A failed archive, kept on screen until the next attempt (not while the phone's sheet is open: it carries the failure itself).
export function ArchiveFailure({ flow }: { flow: ArchiveFlow }) {
  if (flow.error === null || flow.confirming) return null;
  return (
    <p role="alert" className="mt-2 text-sm text-loss">
      {flow.error}
    </p>
  );
}
