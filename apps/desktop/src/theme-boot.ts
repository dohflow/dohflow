/// Applies the stored theme before first paint (personal-cfo-17u1). Loaded as its own
/// `<script type="module">` in index.html, BEFORE main.tsx's — the CSP is
/// `script-src 'self'` (no inline scripts allowed), so a separate module file is how
/// this runs ahead of React rather than an inline `<script>` in the HTML itself.
/// Without this, the app would render light-then-flip-to-dark on every launch with a
/// stored dark preference, since `useTheme` cannot apply the class until React mounts.
import {
  applyResolvedTheme,
  readStoredThemePreference,
  resolveTheme,
  systemPrefersDark,
} from "./theme/themePreference";

applyResolvedTheme(resolveTheme(readStoredThemePreference(), systemPrefersDark()));
