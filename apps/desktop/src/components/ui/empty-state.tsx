import type { LucideIcon } from "lucide-react";

import { cn } from "@/lib/utils";

/// The shared empty-state block (2pcx): a soft icon medallion, a one-line title, and
/// a short "what to do next" description. Used by lists and charts so an empty vault
/// reads as an invitation, not a dead end.
export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
  className,
}: {
  icon: LucideIcon;
  title: string;
  description?: string;
  action?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-2 py-10 text-center",
        className,
      )}
    >
      <span className="flex size-10 items-center justify-center rounded-full bg-muted">
        <Icon aria-hidden className="size-5 text-muted-foreground" />
      </span>
      <p className="text-sm font-medium">{title}</p>
      {description && (
        <p className="max-w-sm text-xs text-muted-foreground">{description}</p>
      )}
      {action}
    </div>
  );
}
