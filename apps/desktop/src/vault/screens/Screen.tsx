import type { ReactNode } from "react";

import { SAFE_AREA_PADDING } from "@/lib/safeArea";

/// Centered single-card layout shared by the pre-vault screens.
export function Screen({ children }: { children: ReactNode }) {
  return (
    <div className={`flex min-h-dvh flex-col bg-background ${SAFE_AREA_PADDING}`}>
      <main className="flex flex-1 items-center justify-center p-6">
        <div className="w-full max-w-md">{children}</div>
      </main>
    </div>
  );
}
