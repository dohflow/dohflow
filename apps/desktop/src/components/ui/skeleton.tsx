import { cn } from "@/lib/utils";

/// shadcn `skeleton` primitive — a pulsing placeholder block for loading states.
function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      className={cn("animate-pulse rounded-md bg-muted", className)}
      {...props}
    />
  );
}

export { Skeleton };
