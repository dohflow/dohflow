import { Card, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";
import { THEME_PREFERENCES, type ThemePreference } from "@/theme/themePreference";
import { useTheme } from "@/theme/ThemeProvider";

const OPTION_LABELS: Record<ThemePreference, string> = {
  system: "System",
  light: "Light",
  dark: "Dark",
};

/// Settings' Appearance control (personal-cfo-17u1): System / Light / Dark, applying
/// immediately and persisting across relaunch via `useTheme`. Placed right after the
/// Household card — no IPC round trip here (the preference is a local, per-viewer
/// choice), so unlike most of this file's cards there is no `describeIpcError`, no
/// saving/error state to show; the click itself IS the save.
export function AppearanceCard() {
  const { preference, setPreference } = useTheme();

  return (
    <Card>
      <CardContent className="pt-6">
        <div className="flex flex-col gap-1.5">
          <Label id="appearance-label">Appearance</Label>
          <p className="text-xs text-muted-foreground">
            Follow the system appearance, or pick a fixed theme.
          </p>
          <div
            role="radiogroup"
            aria-labelledby="appearance-label"
            className="mt-1 inline-flex w-fit rounded-md border border-input"
          >
            {THEME_PREFERENCES.map((option, index) => (
              <button
                key={option}
                type="button"
                role="radio"
                aria-checked={preference === option}
                onClick={() => setPreference(option)}
                className={cn(
                  "px-3 py-1.5 text-sm font-medium transition-colors focus-visible:z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
                  index === 0 && "rounded-l-md",
                  index === THEME_PREFERENCES.length - 1 && "rounded-r-md",
                  index > 0 && "border-l border-input",
                  preference === option
                    ? "bg-primary text-primary-foreground"
                    : "bg-background text-foreground hover:bg-accent hover:text-accent-foreground",
                )}
              >
                {OPTION_LABELS[option]}
              </button>
            ))}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
