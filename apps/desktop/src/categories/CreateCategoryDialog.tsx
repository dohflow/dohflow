import { useEffect } from "react";
import { X } from "lucide-react";

import { Button } from "@/components/ui/button";
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

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      // Yield to an open inner popover (emoji picker / combobox) so its own
      // Escape dismisses just that layer, not the whole form the user is
      // filling in (adversarial review of 4d8.25.19). The capture phase +
      // stopPropagation still beats the outer drawer/modal for the top layer.
      const target = event.target as Element | null;
      if (target?.closest?.('[data-escape-layer="popover"]')) return;
      event.stopPropagation();
      onClose();
    }
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center p-4">
      <div
        className="absolute inset-0 bg-foreground/40"
        onClick={onClose}
        role="presentation"
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label="New category"
        className="relative w-full max-w-md rounded-lg border bg-background shadow-xl"
      >
        <div className="flex items-center justify-between border-b px-4 py-3">
          <h2 className="text-sm font-semibold">New category</h2>
          <Button
            size="sm"
            variant="ghost"
            aria-label="Close"
            onClick={onClose}
            className="h-7 w-7 p-0"
          >
            <X className="size-4" aria-hidden />
          </Button>
        </div>
        <div className="p-1">
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
        </div>
      </div>
    </div>
  );
}
