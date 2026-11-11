import { useState } from "react";
import { Loader2 } from "lucide-react";

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { useComfortBand } from "@/dashboard/useComfortBand";
import { dollarsToMinorUnits, formatMoney } from "@/lib/format";
import { describeIpcError } from "@/vault/useVault";
import { PersistentNotice } from "./PersistentSlot";
import { useAttemptCount, useReannounced } from "./attemptKey";
import { useKeepFocusAcrossMove } from "./useKeepFocusAcrossMove";

/// A wire minor-units amount as an editable major-unit string (USD/EUR are 2-dp).
function minorUnitsToInput(minorUnits: number): string {
  return minorUnits === 0 ? "" : (minorUnits / 100).toString();
}

/// The household liquid-cash comfort band (ADR 0018 addendum 915.1, personal-cfo-3v6d): a target
/// range cash should sit within. The LOWER edge is the shipped minimum-cash floor (the buffer the
/// dashboard's below-floor alert uses); the UPPER edge is optional (excess you may choose to
/// deploy). This card enforces `upper >= lower`; the backend stores them independently. IPC flows
/// through the generated `commands` only (ADR 0003).
export function ComfortBandCard() {
  const { band, error: loadError, setLower, setUpper } = useComfortBand();
  const currency = band?.currency ?? "USD";

  // `null` until the user edits, so each field tracks the stored value on load.
  const [lowerDraft, setLowerDraft] = useState<string | null>(null);
  const [upperDraft, setUpperDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const storedLower = band ? minorUnitsToInput(band.lower.minor_units) : "";
  const storedUpper = band?.upper ? minorUnitsToInput(band.upper.minor_units) : "";
  const lowerValue = lowerDraft ?? storedLower;
  const upperValue = upperDraft ?? storedUpper;

  // While an error is on screen, the Save lives in the section's persistent slot with it and is not
  // rendered in the body; it comes back when the error clears. The hand-over keeps focus across
  // that move (see useKeepFocusAcrossMove): to the Save, else (collapsed) the section's heading.
  const saveInSlot = error !== null;
  const save = useKeepFocusAcrossMove({ settled: !saving });
  // Each settled attempt that fails clears and restores its alert text, so an identical repeat is
  // announced again.
  const [attempt, nextAttempt] = useAttemptCount();
  const alertText = useReannounced(error, attempt);
  const fail = (message: string) => {
    setError(message);
    nextAttempt();
  };

  async function onSubmit(event?: React.FormEvent) {
    event?.preventDefault();
    save.remember();
    const lowerMinor = dollarsToMinorUnits(lowerValue);
    if (lowerMinor === null || lowerMinor < 0) {
      fail("Enter a lower edge of zero or more.");
      return;
    }
    // The upper edge is optional; blank clears it.
    let upperMinor: number | null = null;
    if (upperValue.trim() !== "") {
      const parsed = dollarsToMinorUnits(upperValue);
      if (parsed === null || parsed < 0) {
        fail("Enter an upper edge of zero or more, or leave it blank.");
        return;
      }
      if (parsed < lowerMinor) {
        fail("The upper edge must be at or above the lower edge.");
        return;
      }
      upperMinor = parsed;
    }

    setSaving(true);
    setSaved(false);
    // The earlier error and its Save stay on screen until this try settles (see the Household card).
    // Two independent settings. If the lower fails, nothing changed; if the upper fails after the
    // lower saved, say so specifically and keep the drafts so the user can retry the upper.
    const lowerFailure = await setLower({ minor_units: lowerMinor, currency });
    if (lowerFailure) {
      setSaving(false);
      fail(describeIpcError(lowerFailure));
      return;
    }
    const upperFailure = await setUpper(
      upperMinor === null ? null : { minor_units: upperMinor, currency },
    );
    setSaving(false);
    if (upperFailure) {
      fail(
        `Saved the lower edge, but couldn't update the upper edge: ${describeIpcError(upperFailure)}`,
      );
      return;
    }
    setError(null);
    setLowerDraft(null);
    setUpperDraft(null);
    setSaved(true);
  }

  // A button of type "button" in the slot (it sits outside the form) runs the same submit.
  const saveButton = (
    <Button
      ref={save.ref}
      type={saveInSlot ? "button" : "submit"}
      aria-label="Save"
      disabled={saving}
      onClick={saveInSlot ? () => void onSubmit() : undefined}
    >
      {saving ? <Loader2 className="size-4 animate-spin" aria-hidden /> : "Save"}
    </Button>
  );

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base">Comfort band</CardTitle>
        <CardDescription>
          The range you want your liquid cash to stay within.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={onSubmit} className="flex flex-col gap-3">
          <div className="grid grid-cols-2 gap-3 compact:grid-cols-[repeat(auto-fit,minmax(min(100%,9rem),1fr))] compact:[&>*]:min-w-0">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="comfort-band-lower">Lower edge (floor)</Label>
              <Input
                id="comfort-band-lower"
                inputMode="decimal"
                value={lowerValue}
                disabled={saving}
                placeholder="0.00"
                onChange={(event) => {
                  setLowerDraft(event.target.value);
                  setSaved(false);
                }}
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="comfort-band-upper">Upper edge (optional)</Label>
              <Input
                id="comfort-band-upper"
                inputMode="decimal"
                value={upperValue}
                disabled={saving}
                placeholder="—"
                onChange={(event) => {
                  setUpperDraft(event.target.value);
                  setSaved(false);
                }}
              />
            </div>
          </div>
          <div className="flex items-center gap-2">
            {!saveInSlot && saveButton}
            {saved && !saving && <span className="text-xs text-gain">Saved.</span>}
          </div>
          <p className="text-xs text-muted-foreground">
            The range you want your liquid cash to stay within. Your dashboard warns you
            when the cash left after upcoming bills drops below the lower edge; the Future
            Cash chart shades the band. The lower edge is your buffer (set 0 for no floor);
            the upper edge is optional — cash above it is excess you may choose to deploy.
            {band?.upper && <> Currently up to {formatMoney(band.upper)}.</>}
          </p>
          <PersistentNotice>
            {error !== null && (
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p role="alert" className="text-xs text-loss">
                  {alertText}
                </p>
                {saveButton}
              </div>
            )}
            {loadError !== null && (
              <p role="alert" className="text-xs text-loss">
                {loadError}
              </p>
            )}
          </PersistentNotice>
        </form>
      </CardContent>
    </Card>
  );
}
