import { useRef } from "react";
import { X } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { describeIpcError } from "@/vault/useVault";

import { AddCategoryForm } from "./AddCategoryForm";
import { useCategories } from "./useCategories";

/// Create a category without leaving the current context (personal-cfo-4d8.25.19):
/// the picker's "Create new category…" footer opens this dialog (the NewLoanDialog
/// idiom from 4d8.23.5), and on success the new id flows back so the picker
/// autofills. Fetches its own taxonomy via useCategories so any surface can host
/// it without threading parents through.
export function CreateCategoryDialog({
  initialName,
  onCreated,
  onClose,
}: {
  initialName?: string;
  onCreated: (categoryId: string) => void;
  onClose: () => void;
}) {
  const { categories, addCategory } = useCategories();
  // The control that had focus when this dialog was first rendered: the real opener. It is read
  // here, in the first render, because the name field's own `autoFocus` runs before the shared
  // Dialog records "what had focus" and would make the field itself look like the opener.
  const opener = useRef<HTMLElement | null>(
    typeof document !== "undefined" && document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null,
  );

  return (
    // Shared Dialog shell (personal-cfo-4d8.6.2). An open emoji panel handles its own Escape first
    // (it stops the key before the dialog sees it), so Escape dismisses just that layer; otherwise
    // Escape closes this dialog and only this dialog (the drawer behind never sees the key).
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        // The form is short and its emoji panel and parent list open downward past the dialog edge, so this
        // dialog lets them overflow instead of clipping them inside a scrolling body.
        className="max-w-md gap-0 overflow-visible"
        onEscapeKeyDown={(event) => event.stopPropagation()}
        // The name field starts focused through its own `autoFocus`; on close, whichever way, focus
        // goes back to the control that opened the dialog.
        restoreFocusTo={opener}
      >
        <DialogHeader className="flex-row items-center justify-between border-b px-4 pb-3 pt-3">
          <DialogTitle className="text-sm">New category</DialogTitle>
          <Button
            size="sm"
            variant="ghost"
            aria-label="Close"
            onClick={onClose}
            className="h-7 w-7 p-0"
          >
            <X className="size-4" aria-hidden />
          </Button>
        </DialogHeader>
        <DialogBody className="overflow-visible p-1">
          <AddCategoryForm
            parents={categories?.filter((c) => !c.archived) ?? []}
            initialName={initialName}
            submitLabel="Create category"
            onCancel={onClose}
            onCreate={async (input) => {
              const { error, categoryId } = await addCategory(input);
              if (error) return describeIpcError(error);
              if (categoryId !== null) onCreated(categoryId);
              onClose();
              return null;
            }}
          />
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
