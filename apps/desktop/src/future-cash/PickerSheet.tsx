import type { ReactNode, RefObject } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

/// The phone presentation of a Future Cash picker (personal-cfo-i93n3.27 slice B; Mobile design 2,
/// "Opened controls"): the choice opens in the shared dialog, a bottom sheet on a phone, instead of a
/// popover that hangs off its trigger. The values keep their existing meaning and apply the moment they
/// change: there is no Apply step and nothing to roll back, so Close, Escape and an outside tap all keep
/// whatever was chosen, and focus returns to the trigger that opened it. The dialog is named by `title`.
export function PickerSheet({
  open,
  onOpenChange,
  title,
  description,
  restoreFocusTo,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  /// The trigger the sheet was opened from: focus returns to it when the sheet closes.
  restoreFocusTo: RefObject<HTMLElement | null>;
  children: ReactNode;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        // Without a description the dialog names itself by its title only (no copy is invented for one).
        {...(description ? {} : { "aria-describedby": undefined })}
        restoreFocusTo={restoreFocusTo}
        fallbackFocusTo={restoreFocusTo}
      >
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description && <DialogDescription>{description}</DialogDescription>}
        </DialogHeader>
        <DialogBody className="flex flex-col gap-1">{children}</DialogBody>
        <DialogFooter>
          <DialogClose asChild>
            <Button type="button" variant="outline">
              Close
            </Button>
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
