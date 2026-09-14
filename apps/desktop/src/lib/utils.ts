import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/// Merge conditional class names, de-duplicating conflicting Tailwind utilities
/// (the shadcn/ui convention). Last write wins, e.g. `cn("p-2", "p-4")` → `p-4`.
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
