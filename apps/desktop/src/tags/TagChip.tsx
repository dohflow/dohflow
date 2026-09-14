import { X } from "lucide-react";

import type { TagViewDto } from "@/bindings";

/// A small tag pill (ADR 0033). With `onRemove`, shows an × to unassign it (the
/// drawer editor); without it, the chip is read-only (the transaction list).
export function TagChip({
  tag,
  onRemove,
}: {
  tag: TagViewDto;
  onRemove?: () => void;
}) {
  return (
    <span className="inline-flex items-center gap-1 rounded-full border bg-muted/40 px-2 py-0.5 text-xs text-muted-foreground">
      <span className="max-w-[10rem] truncate">{tag.name}</span>
      {onRemove && (
        <button
          type="button"
          onClick={onRemove}
          aria-label={`Remove ${tag.name}`}
          className="rounded-full hover:text-foreground"
        >
          <X className="size-3" aria-hidden />
        </button>
      )}
    </span>
  );
}
