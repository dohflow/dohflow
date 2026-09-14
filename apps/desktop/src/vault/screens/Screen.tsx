import type { ReactNode } from "react";

/// Centered single-card layout shared by the pre-vault screens.
export function Screen({ children }: { children: ReactNode }) {
  return (
    <main className="flex min-h-screen items-center justify-center bg-background p-6">
      <div className="w-full max-w-md">{children}</div>
    </main>
  );
}
