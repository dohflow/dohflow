import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

/// shadcn `badge` primitive (ADR 0031), themed to the brand tokens. The semantic
/// variants (gain / warning / info) tint with their finance color at low opacity so
/// a badge never shouts louder than the number it annotates.
const badgeVariants = cva(
  "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium transition-colors",
  {
    variants: {
      variant: {
        default: "border-transparent bg-primary text-primary-foreground",
        secondary: "border-transparent bg-secondary text-secondary-foreground",
        outline: "border-border text-muted-foreground",
        gain: "border-gain/25 bg-gain/10 text-gain",
        warning: "border-warning/30 bg-warning/10 text-warning",
        info: "border-info/25 bg-info/10 text-info",
        loss: "border-loss/25 bg-loss/10 text-loss",
      },
    },
    defaultVariants: { variant: "default" },
  },
);

function Badge({
  className,
  variant,
  ...props
}: React.ComponentProps<"span"> & VariantProps<typeof badgeVariants>) {
  return (
    <span className={cn(badgeVariants({ variant }), className)} {...props} />
  );
}

export { Badge, badgeVariants };
