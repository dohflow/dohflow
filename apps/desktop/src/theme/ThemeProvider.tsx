import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  applyResolvedTheme,
  readStoredThemePreference,
  resolveTheme,
  systemPrefersDark,
  writeStoredThemePreference,
  type ResolvedTheme,
  type ThemePreference,
} from "./themePreference";

/// Best-effort native title-bar sync (personal-cfo-17u1, `core:window:allow-set-theme`).
/// Never throws: `setTheme` can reject if the platform doesn't support it, and a plain
/// `pnpm dev` in a browser (not a real Tauri window) has no native title bar to sync at
/// all — either way the web-rendered `.dark` class already applied, which is the part
/// that actually matters, so a failure here is silently swallowed rather than surfaced.
async function syncNativeTitleBar(preference: ThemePreference): Promise<void> {
  try {
    await getCurrentWindow().setTheme(preference === "system" ? null : preference);
  } catch {
    // See the doc comment above — the web theme is unaffected either way.
  }
}

export interface ThemeContextValue {
  /** The stored choice: "system" (the default), "light", or "dark". */
  preference: ThemePreference;
  /** What's actually rendered right now — "system" resolves against the live OS query. */
  resolved: ResolvedTheme;
  /** Change the preference. Applies immediately and persists across relaunch. */
  setPreference: (next: ThemePreference) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

/// Provides theme state for the app's ENTIRE lifetime — mounted once at the true root
/// (`App.tsx`), covering the locked/picker screens exactly as much as the unlocked
/// shell (personal-cfo-17u1 review, Finding 1: the original design had `useTheme` hold
/// its OWN local state, with the only instance created inside `AppearanceCard` — which
/// `SettingsView` unmounts on every other tab, and never mounts at all on the lock
/// screen. "Follows the macOS appearance… while running" held only while Settings
/// happened to be open). There must be exactly ONE instance of this state in the whole
/// app: two independent resolvers could disagree about "the current preference" the
/// moment either one's local render lagged the other, and — the specific failure mode
/// the review named — a second, still-subscribed "system" listener from an earlier
/// mount could silently revert an explicit override the user just picked elsewhere.
/// Every consumer (`useTheme()`, `AppearanceCard`, `ThemeToggle`) reads this ONE source.
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [preference, setPreferenceState] = useState<ThemePreference>(
    readStoredThemePreference,
  );
  const [resolved, setResolved] = useState<ResolvedTheme>(() =>
    resolveTheme(preference, systemPrefersDark()),
  );

  // Re-resolve and re-apply whenever the preference itself changes (an explicit
  // override, or falling back to "system"). theme-boot.ts already handled first paint;
  // re-applying here is what keeps `resolved`'s React state in sync with the DOM class
  // after a `setPreference` call.
  useEffect(() => {
    const next = resolveTheme(preference, systemPrefersDark());
    setResolved(next);
    applyResolvedTheme(next);
    void syncNativeTitleBar(preference);
  }, [preference]);

  // While following the system, listen for macOS's appearance actually changing. This
  // effect lives HERE — in the provider, mounted for the app's whole lifetime — and
  // nowhere else; a per-screen or per-component subscription is exactly what Finding 1
  // found broken. Deliberately re-subscribes whenever `preference` changes (not just on
  // mount) so an explicit override correctly stops listening (proving an override can
  // never be silently reverted by a stale subscription), and switching back to "system"
  // starts listening again with a fresh reading rather than a stale one from before the
  // switch.
  useEffect(() => {
    if (preference !== "system") return;
    let media: MediaQueryList;
    try {
      media = window.matchMedia("(prefers-color-scheme: dark)");
    } catch {
      return;
    }
    const onChange = () => {
      const next = resolveTheme("system", media.matches);
      setResolved(next);
      applyResolvedTheme(next);
    };
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, [preference]);

  const setPreference = useCallback((next: ThemePreference) => {
    writeStoredThemePreference(next);
    setPreferenceState(next);
  }, []);

  return (
    <ThemeContext.Provider value={{ preference, resolved, setPreference }}>
      {children}
    </ThemeContext.Provider>
  );
}

/// Reads the shared theme state. Must be called under `<ThemeProvider>`, which wraps
/// the whole app in `App.tsx` — outside `<VaultProvider>`, so it covers the locked
/// screens too, not just the unlocked shell.
export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) {
    throw new Error(
      "useTheme() called outside <ThemeProvider> — see App.tsx, which must wrap the whole app.",
    );
  }
  return ctx;
}
