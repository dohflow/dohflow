import { useEffect, useMemo, useRef, useState } from "react";
import { Loader2 } from "lucide-react";

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Combobox, type ComboboxItem } from "@/components/ui/combobox";
import { describeIpcError } from "@/vault/useVault";
import { PersistentNotice } from "./PersistentSlot";
import { useHouseholdTimezone } from "./useHouseholdTimezone";

/// A short, curated set of IANA zones for the rare WebView build that lacks
/// `Intl.supportedValuesOf` (personal-cfo-q329) — WKWebView on macOS 14 supports it, so
/// this is a defensive fallback, not the expected path.
const CURATED_TIMEZONES = [
  "UTC",
  "America/New_York",
  "America/Chicago",
  "America/Denver",
  "America/Los_Angeles",
  "America/Anchorage",
  "Pacific/Honolulu",
  "America/Sao_Paulo",
  "Europe/London",
  "Europe/Paris",
  "Europe/Berlin",
  "Europe/Moscow",
  "Africa/Cairo",
  "Asia/Dubai",
  "Asia/Kolkata",
  "Asia/Shanghai",
  "Asia/Tokyo",
  "Australia/Sydney",
  "Pacific/Auckland",
];

function supportedTimezones(): string[] {
  try {
    if (typeof Intl.supportedValuesOf === "function") {
      const zones = Intl.supportedValuesOf("timeZone");
      // "UTC" is the vault's default stored value (ADR 0021) but is NOT itself in this
      // list (confirmed: Node/V8's IANA zone set omits the "UTC" alias, only carrying
      // named zones like "Etc/UTC") — without adding it back, a never-configured
      // household's combobox would render blank instead of showing its actual value.
      if (zones.length > 0) return zones.includes("UTC") ? zones : ["UTC", ...zones];
    }
  } catch {
    // fall through to the curated list
  }
  return CURATED_TIMEZONES;
}

/// The browser's own IANA zone, offered as a suggestion when the vault has never had one
/// set (still the "UTC" default) — an initial default a person can adopt with one click,
/// never the authoritative value once set (ADR 0021 §1's distinction; the same one the
/// machine-zone capture at vault creation already relies on).
function machineTimezone(): string | null {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || null;
  } catch {
    return null;
  }
}

function timezoneItem(tz: string): ComboboxItem {
  const parts = tz.split("/");
  const city = (parts.at(-1) ?? tz).replace(/_/g, " ");
  const region =
    parts.length > 1 ? parts.slice(0, -1).join("/").replace(/_/g, " ") : undefined;
  return { id: tz, label: city, hint: region, searchText: tz.replace(/_/g, " ") };
}

/// The household's IANA timezone (ADR 0021 addendum, personal-cfo-q329) — the anchor for
/// what "today" means everywhere the forecast draws a calendar boundary: due dates, the
/// past-due queue, Cash Flow. The first card in Settings; later household work (reporting
/// currency + locale, personal-cfo-7oj4; household members, personal-cfo-339) extends this
/// card rather than building a second Settings surface. IPC flows through the generated
/// `commands` only (ADR 0003).
export function HouseholdCard() {
  const {
    timezone,
    isLoading,
    failure: readFailure,
    retry,
    retrying,
    isSaving,
    setTimezone,
  } = useHouseholdTimezone();
  const [draft, setDraft] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const items = useMemo(() => supportedTimezones().map(timezoneItem), []);
  const value = draft ?? timezone;
  const suggestion = timezone === "UTC" ? machineTimezone() : null;

  // While a failed write is on screen its Save lives in the section's persistent slot with the error,
  // and is not rendered in the body; the Save comes back to the body when the error clears. One ref
  // follows whichever one is rendered, so focus can be handed back after the move.
  const saveRef = useRef<HTMLButtonElement>(null);
  const refocusSave = useRef(false);
  const saveInSlot = saveError !== null;
  // Runs after every render: the Save is disabled while the write settles, and a disabled button
  // cannot take focus, so the hand-over waits for it to be enabled. If the person has moved focus to
  // another control meanwhile, nothing is taken from them.
  useEffect(() => {
    if (!refocusSave.current) return;
    const save = saveRef.current;
    const active = document.activeElement;
    if (active && active !== document.body && active !== save) {
      refocusSave.current = false;
    } else if (save && !save.disabled) {
      refocusSave.current = false;
      save.focus();
    } else if (!isSaving) {
      // Saved: there is nothing left to save, so the Save stays disabled and cannot take focus.
      // The timezone picker is where the person was working.
      refocusSave.current = false;
      document.getElementById("household-timezone")?.focus();
    }
  });

  async function onSave() {
    if (value === timezone) return;
    refocusSave.current = document.activeElement === saveRef.current;
    setSaveError(null);
    setSaved(false);
    const failure = await setTimezone(value);
    if (failure) {
      setSaveError(describeIpcError(failure));
    } else {
      setDraft(null);
      setSaved(true);
    }
  }

  const saveButton = (
    <Button
      ref={saveRef}
      type="button"
      aria-label="Save"
      onClick={onSave}
      disabled={isSaving || isLoading || value === timezone}
    >
      {isSaving ? <Loader2 className="size-4 animate-spin" aria-hidden /> : "Save"}
    </Button>
  );

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base">Household</CardTitle>
        <CardDescription>
          Sets what &ldquo;today&rdquo; means for due dates, the past-due queue, and the
          forecast.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="household-timezone">Timezone</Label>
            <Combobox
              id="household-timezone"
              aria-label="Household timezone"
              items={items}
              value={isLoading ? null : value}
              onSelect={(id) => {
                if (id === null) return;
                setDraft(id);
                setSaved(false);
                setSaveError(null);
              }}
              placeholder="Search for a timezone…"
              disabled={isLoading || isSaving}
            />
            {suggestion && suggestion !== value && (
              <p className="text-xs text-muted-foreground">
                This Mac is set to {suggestion.replace(/_/g, " ")}.{" "}
                <button
                  type="button"
                  className="underline underline-offset-2"
                  disabled={isSaving}
                  onClick={() => {
                    setDraft(suggestion);
                    setSaved(false);
                    setSaveError(null);
                  }}
                >
                  Use it
                </button>
              </p>
            )}
          </div>
          <div className="flex items-center gap-2">
            {!saveInSlot && saveButton}
            {saved && !isSaving && <span className="text-xs text-gain">Saved.</span>}
          </div>
          <PersistentNotice>
            {saveError !== null && (
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p role="alert" className="text-xs text-loss">
                  {saveError}
                </p>
                {saveButton}
              </div>
            )}
            {readFailure !== null && (
              // A failed read is read again: this never writes, and never saves the UTC fallback.
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p role="alert" className="text-xs text-loss">
                  {readFailure}
                </p>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={retrying}
                  onClick={retry}
                >
                  {retrying ? "Retrying…" : "Retry"}
                </Button>
              </div>
            )}
          </PersistentNotice>
        </div>
      </CardContent>
    </Card>
  );
}
