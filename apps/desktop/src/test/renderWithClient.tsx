import type { ReactElement } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";

import { ThemeProvider } from "@/theme/ThemeProvider";

/// Render a component that uses TanStack Query, wrapped in a **fresh**
/// `QueryClientProvider` per call (so tests never share cache). Retries are off
/// so an error path resolves immediately. Also wraps in `ThemeProvider` — outside
/// `QueryClientProvider`, matching `App.tsx`'s real nesting order — since `useTheme()`
/// throws outside one (personal-cfo-17u1 review, Finding 1's fix) and any view under
/// test may render something that reads it (e.g. `SettingsView`'s `AppearanceCard`).
export function renderWithClient(ui: ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  return render(
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>
    </ThemeProvider>,
  );
}
