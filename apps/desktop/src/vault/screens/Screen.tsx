import type { ReactNode } from "react";

import { SAFE_AREA_PADDING } from "@/lib/safeArea";

/// Centered single-card layout shared by the pre-vault screens.
///
/// `main` is a size container so the gutters inside it can give the column its room back: at the
/// largest system text size the screen, card and callout padding nest (each grows a little with the
/// type) and leave a sliver of a column, so a narrow screen at that size drops the page gutter to a
/// hairline and its descendants (the card header and content, the warning callout) trim theirs with
/// `compact:@max-[15rem]:px-*` (personal-cfo-i93n3.25, Studio F3). The padding sits on the inner element, not on `main`: a
/// container's own padding is outside what a container query measures. Desktop is unchanged.
export function Screen({ children }: { children: ReactNode }) {
  return (
    <div className={`flex min-h-dvh flex-col bg-background ${SAFE_AREA_PADDING}`}>
      <main className="@container flex flex-1 items-center justify-center">
        <div className="w-full max-w-md p-6 compact:@max-[15rem]:p-1">{children}</div>
      </main>
    </div>
  );
}
