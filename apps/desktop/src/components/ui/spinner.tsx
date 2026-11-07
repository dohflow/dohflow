import { Loader2 } from "lucide-react";

import { cn } from "@/lib/utils";

/// The busy indicator (personal-cfo-i93n3.26). It spins, except where the person has asked for reduced
/// motion: there it is hidden, and the static text label every busy state already carries ("Unlocking…",
/// "Restoring…") is the whole indicator, as the vault access design requires. Decorative: the label, not
/// the icon, is what assistive technology reads.
export function Spinner({ className }: { className?: string }) {
  return <Loader2 aria-hidden className={cn("animate-spin motion-reduce:hidden", className)} />;
}
