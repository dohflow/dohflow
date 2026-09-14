/// Pure theme-resolution logic (personal-cfo-17u1): the stored preference, the live
/// system query, and the single function that reconciles them into what actually
/// renders. Shared by `theme-boot.ts` (applies the theme before first paint, so there is
/// no flash of the wrong one while React boots) and `useTheme.ts` (keeps it correct
/// afterward) — both must agree on the exact same resolution, so it lives here once.

/// The three choices exposed in Settings. "system" (the default) follows macOS; "light"
/// and "dark" pin an explicit override that wins regardless of the OS setting.
export const THEME_PREFERENCES = ["system", "light", "dark"] as const;
export type ThemePreference = (typeof THEME_PREFERENCES)[number];

/// What actually renders — always one of these two, even when the preference is "system".
export type ResolvedTheme = "light" | "dark";

const DEFAULT_PREFERENCE: ThemePreference = "system";

// Every other sticky per-viewer UI preference in this codebase uses a `pcfo.`-prefixed
// localStorage key (NeedsConfirmation.tsx's `pcfo.needsConfirmCollapsed`,
// TransactionsView.tsx's `pcfo.txnCategoryIcons`, usePagination's page-size keys) — kept
// consistent with that existing convention rather than introducing a new `dohflow.`
// prefix, since the choice of literal has no user-facing effect and consistency does.
const STORAGE_KEY = "pcfo.theme";

function isThemePreference(value: string): value is ThemePreference {
  return (THEME_PREFERENCES as readonly string[]).includes(value);
}

/// Read the stored theme preference, falling back to "system". Storage can be
/// unavailable (a restricted webview, private mode) — never throw; the theme still
/// resolves against the live system query either way.
export function readStoredThemePreference(): ThemePreference {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    return raw !== null && isThemePreference(raw) ? raw : DEFAULT_PREFERENCE;
  } catch {
    return DEFAULT_PREFERENCE;
  }
}

/// Persist a theme preference. Storage can be unavailable — the choice still applies
/// for this session; only stickiness across relaunch is lost.
export function writeStoredThemePreference(preference: ThemePreference): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, preference);
  } catch {
    // Session-only fallback — see the doc comment above.
  }
}

/// Whether the system currently prefers dark, per `matchMedia`. The one place this
/// query is made, so `theme-boot.ts` (pre-paint) and `useTheme` (runtime) can never
/// disagree about what "system" currently means.
export function systemPrefersDark(): boolean {
  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  } catch {
    // matchMedia unavailable (an unusual WebView build) — default to light rather than
    // guessing dark; an explicit override is still always available in Settings.
    return false;
  }
}

/// The actual theme to render for a given preference. An explicit choice always wins;
/// "system" resolves against the live system query passed in (kept as a parameter,
/// not called internally, so both callers can supply a single fresh reading and this
/// function itself stays trivially testable with no DOM).
export function resolveTheme(
  preference: ThemePreference,
  prefersDark: boolean,
): ResolvedTheme {
  if (preference === "light") return "light";
  if (preference === "dark") return "dark";
  return prefersDark ? "dark" : "light";
}

/// Apply a resolved theme to the document. The ONLY place the `.dark` class is
/// toggled — called at boot (before paint) and on every later change, so the class and
/// the "resolved" value in React state can never drift apart.
export function applyResolvedTheme(resolved: ResolvedTheme): void {
  document.documentElement.classList.toggle("dark", resolved === "dark");
}
