import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

/// shadcn `badge` primitive (ADR 0031), themed to the brand tokens. The semantic
/// variants (gain / warning / info / loss) tint with their finance color at low opacity so
/// a badge never shouts louder than the number it annotates.
///
/// personal-cfo-4d8.35: the semantic variants read as NEUTRAL foreground text with a small
/// semantic MARK (the dot before the label), not as colored text. Colored text on its own 10%
/// tint measured below 4.5:1 for 12px text as rendered (light: gain 4.36, warning 3.14, info
/// 4.32, loss 4.30 on --background), and amber text cannot reach 4.5:1 without turning brown.
/// The mark carries the meaning and clears the 3:1 non-text floor on its tint in both themes;
/// the label is `--foreground` (12.9 to 15.8:1). A caller with its own icon passes
/// `before:hidden` and colors the icon with the same token (see CardReviewModal).
const badgeVariants = cva(
  "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium transition-colors",
  {
    variants: {
      variant: {
        default: "border-transparent bg-primary text-primary-foreground",
        secondary: "border-transparent bg-secondary text-secondary-foreground",
        outline: "border-border text-muted-foreground",
        gain: "border-gain/25 bg-gain/10 text-foreground before:size-1.5 before:shrink-0 before:rounded-full before:bg-gain",
        warning:
          "border-warning/30 bg-warning/10 text-foreground before:size-1.5 before:shrink-0 before:rounded-full before:bg-warning",
        info: "border-info/25 bg-info/10 text-foreground before:size-1.5 before:shrink-0 before:rounded-full before:bg-info",
        loss: "border-loss/25 bg-loss/10 text-foreground before:size-1.5 before:shrink-0 before:rounded-full before:bg-loss",
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
