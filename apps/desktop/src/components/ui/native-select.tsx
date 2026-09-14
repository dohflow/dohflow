import * as React from "react";
import { ChevronDown } from "lucide-react";

import { cn } from "@/lib/utils";

/// A consistently-themed **native** `<select>` (ADR 0031). The app deliberately keeps
/// the platform select (reliable keyboard/AT behavior, `fireEvent.change`-testable)
/// and standardizes the skin here instead of hand-rolling classes at every call site.
/// A Radix-based `Select` can replace this wholesale later — call sites won't change
/// shape. Sizes: `sm` for inline row controls, default for forms.
const NativeSelect = React.forwardRef<
  HTMLSelectElement,
  Omit<React.ComponentProps<"select">, "size"> & { size?: "sm" | "default" }
>(({ className, size = "default", children, ...props }, ref) => (
  <span className={cn("relative inline-flex", className)}>
    <select
      ref={ref}
      className={cn(
        "w-full cursor-pointer appearance-none rounded-md border border-input bg-background pr-8 text-foreground transition-colors",
        "hover:border-ring/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
        "disabled:cursor-not-allowed disabled:opacity-50",
        size === "sm" ? "h-8 px-2.5 text-xs" : "h-10 px-3 py-2 text-sm",
      )}
      {...props}
    >
      {children}
    </select>
    <ChevronDown
      aria-hidden
      className={cn(
        "pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground",
        size === "sm" ? "size-3" : "size-4",
      )}
    />
  </span>
));
NativeSelect.displayName = "NativeSelect";

export { NativeSelect };
