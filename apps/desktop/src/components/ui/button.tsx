import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

// A disabled button keeps its native `disabled` semantics (no activation, out of the tab order, no
// pointer events) and stays READABLE: its label is `muted-foreground` (5.0:1 or better in both themes)
// on a neutral fill instead of the whole control fading to half opacity, which left filled labels at
// about 2.2 to 2.7:1. Filled variants swap their brand fill for the neutral one so enabled and disabled
// still read as different controls (personal-cfo-4d8.52).
const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default:
          "bg-primary text-primary-foreground hover:bg-primary/90 disabled:bg-muted disabled:text-muted-foreground",
        secondary:
          "bg-secondary text-secondary-foreground hover:bg-secondary/80 disabled:text-muted-foreground",
        outline:
          "border border-input bg-background hover:bg-accent hover:text-accent-foreground disabled:text-muted-foreground",
        ghost:
          "hover:bg-accent hover:text-accent-foreground disabled:text-muted-foreground",
        destructive:
          "bg-destructive text-destructive-foreground hover:bg-destructive/90 disabled:bg-muted disabled:text-muted-foreground",
        link:
          "text-primary underline-offset-4 hover:underline disabled:text-muted-foreground",
      },
      size: {
        default: "h-10 px-4 py-2",
        sm: "h-9 rounded-md px-3",
        lg: "h-11 rounded-md px-6 text-base",
        icon: "h-10 w-10",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        ref={ref}
        className={cn(buttonVariants({ variant, size, className }))}
        {...props}
      />
    );
  },
);
Button.displayName = "Button";

export { Button, buttonVariants };
