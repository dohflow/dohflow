import { Moon, Sun, SunMoon, type LucideIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import { THEME_PREFERENCES, type ThemePreference } from "./themePreference";
import { useTheme } from "./ThemeProvider";

const ICONS: Record<ThemePreference, LucideIcon> = {
  system: SunMoon,
  light: Sun,
  dark: Moon,
};

const LABELS: Record<ThemePreference, string> = {
  system: "System",
  light: "Light",
  dark: "Dark",
};

function nextPreference(current: ThemePreference): ThemePreference {
  const index = THEME_PREFERENCES.indexOf(current);
  return THEME_PREFERENCES[(index + 1) % THEME_PREFERENCES.length]!;
}

/// A compact, always-available appearance control (personal-cfo-17u1 review, Finding 1:
/// the only way to change the theme was buried in Settings, which doesn't exist on the
/// lock screen at all — the fix isn't just moving the *subscription* to the app root,
/// it's making the *control* reachable everywhere too). Rendered once at the app root
/// (`App.tsx`'s `VaultRouter`, unconditionally — locked, picker, or unlocked all show
/// it), so switching appearance never requires being inside the vault.
///
/// One click cycles System -> Light -> Dark -> System. The icon names the CURRENT
/// preference, not just what's rendered right now — a half-sun-half-moon glyph
/// specifically for "System" (sitting on the same sun-to-moon spectrum as the other two
/// states), so "System" stays visually distinct even on a system that currently happens
/// to be in light or dark already.
export function ThemeToggle({
  floating = false,
}: {
  /// Pin to a screen corner — used on the vault screens, which have no sidebar chrome
  /// of their own (mirrors `BuildBadge`'s `floating` prop, opposite corner so the two
  /// never collide).
  floating?: boolean;
}) {
  const { preference, setPreference } = useTheme();
  const Icon = ICONS[preference];

  return (
    <button
      type="button"
      onClick={() => setPreference(nextPreference(preference))}
      aria-label={`Appearance: ${LABELS[preference]}. Click to change.`}
      title={`Appearance: ${LABELS[preference]}`}
      className={cn(
        "inline-flex size-8 items-center justify-center rounded-full border border-input bg-background text-foreground transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
        floating && "fixed top-3 right-3 z-30",
      )}
    >
      <Icon className="size-4" aria-hidden />
    </button>
  );
}
