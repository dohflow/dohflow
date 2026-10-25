import type { ReactElement } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";

import { NoticeProvider, NotificationsRegion } from "@/components/ui/notice";
import { ThemeProvider } from "@/theme/ThemeProvider";

/// Render a component that uses TanStack Query, wrapped in a **fresh**
/// `QueryClientProvider` per call (so tests never share cache). Retries are off
/// so an error path resolves immediately. Also wraps in `ThemeProvider` — outside
/// `QueryClientProvider`, matching `App.tsx`'s real nesting order — since `useTheme()`
/// throws outside one (personal-cfo-17u1 review, Finding 1's fix) and any view under
/// test may render something that reads it (e.g. `SettingsView`'s `AppearanceCard`). It also
/// supplies the shell's notice provider + region, which any view that shows an Undo needs.
export function renderWithClient(ui: ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  return render(
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        {/* The shell's notice provider and region (docs/agent/FRONTEND.md section 6.2), so a view
            that shows a notice works under test. */}
        <NoticeProvider>
          <NotificationsRegion />
          {ui}
        </NoticeProvider>
      </QueryClientProvider>
    </ThemeProvider>,
  );
}
