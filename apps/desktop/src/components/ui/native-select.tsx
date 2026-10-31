import * as React from "react";
import { ChevronDown } from "lucide-react";

import { cn } from "@/lib/utils";

/// A consistently-themed **native** `<select>` (ADR 0031). The app deliberately keeps
/// the platform select (reliable keyboard/AT behavior, `fireEvent.change`-testable)
/// and standardizes the skin here instead of hand-rolling classes at every call site.
/// A Radix-based `Select` can replace this wholesale later — call sites won't change
/// shape. Sizes: `sm` for inline row controls, default for forms.
///
/// personal-cfo-4d8.35: the hover border is the FULL `--ring` color, not `border-ring/60`. At 60% over
/// the control's fill it rendered at 2.05:1 (Light) and 2.74:1 (Dark), below the 3:1 a boundary needs to
/// keep identifying the control while hovered, and LOWER than the resting `--input` boundary it replaced.
/// Full `--ring` is 3.55:1 / 5.56:1 on `--background`; `styles/theme-contrast.test.ts` pins any hover
/// border override on a shared control to 3:1 plus headroom.
///
/// The right padding keeps the text clear of the chevron (inset 10px plus the icon: 26px at the default
/// size, 22px at `sm`). It is set per size, AFTER the left padding: a base `pr-8` followed by a size's
/// `px-*` is dropped by tailwind-merge, which let long options run under the chevron.
const NativeSelect = React.forwardRef<
  HTMLSelectElement,
  Omit<React.ComponentProps<"select">, "size"> & { size?: "sm" | "default" }
>(({ className, size = "default", children, ...props }, ref) => (
  <span className={cn("relative inline-flex", className)}>
    <select
      ref={ref}
      className={cn(
        "w-full cursor-pointer appearance-none rounded-md border border-input bg-background text-foreground transition-colors",
        "hover:border-ring focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
        "disabled:cursor-not-allowed disabled:opacity-50",
        size === "sm" ? "h-8 pl-2.5 pr-7 text-xs" : "h-10 py-2 pl-3 pr-8 text-sm",
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
