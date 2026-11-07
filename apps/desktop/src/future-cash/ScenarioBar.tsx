import { useEffect, useState, type FormEvent } from "react";
import { Archive, FolderPlus, Pencil } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { todayInTimezone } from "@/lib/format";
import { useHouseholdTimezone } from "@/settings/useHouseholdTimezone";
import { describeIpcError } from "@/vault/useVault";
import { isExpired } from "./scenarioEligibility";
import { TimezoneReadNotice } from "./TimezoneReadNotice";
import { SCENARIO_NAME_MAX, useScenarios } from "./useScenarios";

/// What a held selection is called when its row is not in the list and the date could not be read.
const SELECTED_SCENARIO = "Selected scenario";

const STATUS_CLASS = "h-9 rounded-md border bg-background px-3 text-sm";

/// The scenario selector (personal-cfo-6zep): pick the base forecast or one of the
/// user's scenarios, create a new scenario, or archive the selected one. The parent
/// owns the selection (the forecast queries key on it); this bar drives it via
/// `onSelect`. Archiving keeps every change and falls the selection back to base
/// (ADR 0051 §1); deleting lives in the Scenarios tab.
export function ScenarioBar({
  selectedId,
  onSelect,
}: {
  selectedId: string | null;
  onSelect: (id: string | null) => void;
}) {
  const { scenarios, error, addScenario, archiveScenario, updateScenario } = useScenarios();
  // The backend gate compares `expires_on` against the HOUSEHOLD-local date (ADR 0051 §3),
  // so the picker does too, not the device's calendar.
  // Until that date has been read (not the UTC fallback), the picker offers no choices and the
  // selection is left alone.
  const tz = useHouseholdTimezone();
  const today = todayInTimezone(tz.timezone);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  // Edit (rename + status) of the selected scenario (personal-cfo-vru6).
  const [editing, setEditing] = useState(false);
  const [editName, setEditName] = useState("");
  const [editStatus, setEditStatus] = useState("draft");

  // Only SELECTABLE scenarios belong in the picker: archived and expired ones are
  // gated by the backend (ADR 0051 §1/§3), so offering them would let the user pick a
  // scenario whose changes are silently not applied — the compare banner would read
  // "+$0.00" and look like the plan had no effect.
  const notArchived = (scenarios ?? []).filter((s) => s.status !== "archived");
  const visible = tz.verified ? notArchived.filter((s) => !isExpired(s, today)) : [];
  // While the date is unverified the selection stays as it was, and is shown, not judged.
  const selected =
    (tz.verified ? visible : notArchived).find((s) => s.id === selectedId) ?? null;
  // Until the reads that can judge the selection are in (the date, and the list), the select still
  // holds that exact scenario, so it never reads as "Base (no scenario)". Its text, in order: the
  // row's name when it is known (archived or not); the list's own failure; "Selected scenario" when
  // the date could not be verified but the list is here; "Loading…" only while a read is pending.
  const listed = tz.verified ? visible : [];
  const heldSelection =
    selectedId !== null &&
    (!tz.verified || scenarios === null) &&
    !listed.some((s) => s.id === selectedId)
      ? {
          id: selectedId,
          label:
            (scenarios ?? []).find((s) => s.id === selectedId)?.name ??
            error ??
            (tz.failure !== null && scenarios !== null ? SELECTED_SCENARIO : "Loading…"),
        }
      : null;
  // A selection that is no longer selectable (archived/expired/deleted elsewhere) falls
  // back to base, rather than leaving a blank picker over base numbers. Only a verified date
  // can say a scenario has expired, so nothing falls back before that.
  useEffect(() => {
    if (tz.verified && selectedId !== null && scenarios !== null && selected === null) {
      onSelect(null);
    }
  }, [tz.verified, selectedId, scenarios, selected, onSelect]);

  function startEditing() {
    if (!selected) return;
    setEditName(selected.name);
    setEditStatus(selected.status);
    setFormError(null);
    setEditing(true);
  }

  async function saveEdit(event: FormEvent) {
    event.preventDefault();
    if (!selected) return;
    if (!editName.trim()) {
      setFormError("Name your scenario.");
      return;
    }
    setBusy(true);
    setFormError(null);
    const failure = await updateScenario({
      id: selected.id,
      name: editName.trim(),
      status: editStatus,
    });
    setBusy(false);
    if (failure) {
      setFormError(describeIpcError(failure));
    } else {
      setEditing(false);
    }
  }

  async function create(event: FormEvent) {
    event.preventDefault();
    if (!name.trim()) {
      setFormError("Name your scenario.");
      return;
    }
    setBusy(true);
    setFormError(null);
    const result = await addScenario({ name: name.trim(), description: null });
    setBusy(false);
    if (result.status === "ok") {
      onSelect(result.data.id);
      setName("");
      setCreating(false);
    } else {
      setFormError(describeIpcError(result.error));
    }
  }

  return (
    <div className="flex flex-col gap-2">
      {/* On a phone the label sits over a full-width select and the action buttons take the full width
          (personal-cfo-i93n3.27). */}
      <div className="flex flex-wrap items-center gap-2 compact:flex-col compact:items-stretch">
        <Label
          htmlFor="scenario-select"
          className="text-sm text-muted-foreground"
        >
          Scenario
        </Label>
        <select
          id="scenario-select"
          value={selectedId ?? ""}
          onChange={(e) => onSelect(e.target.value || null)}
          disabled={!tz.verified}
          className="h-9 rounded-md border bg-background px-3 text-sm compact:w-full"
        >
          <option value="">Base (no scenario)</option>
          {listed.map((scenario) => (
            <option key={scenario.id} value={scenario.id}>
              {scenario.name}
            </option>
          ))}
          {heldSelection && (
            <option key={heldSelection.id} value={heldSelection.id}>
              {heldSelection.label}
            </option>
          )}
        </select>
        {!tz.verified && (
          <TimezoneReadNotice failure={tz.failure} retry={tz.retry} retrying={tz.retrying} />
        )}
        {!creating && (
          <Button variant="outline" size="sm" className="compact:w-full" onClick={() => setCreating(true)}>
            <FolderPlus aria-hidden />
            New scenario
          </Button>
        )}
        {selected && !editing && (
          <Button
            variant="ghost"
            size="icon"
            aria-label={`Edit ${selected.name}`}
            onClick={startEditing}
          >
            <Pencil aria-hidden />
          </Button>
        )}
        {selected && (
          // ARCHIVE, not delete (ADR 0051 §1/§5). This bar has no confirmation step,
          // and since ADR 0051 `delete` removes the scenario and its changes from every
          // view with no way back in the app — an unconfirmed one-click delete would be a
          // trap, and it used to be recoverable. Deleting lives in the Scenarios tab,
          // behind a confirmation that says what is lost.
          <Button
            variant="ghost"
            size="icon"
            aria-label={`Archive ${selected.name}`}
            title="Archive — keeps your changes; manage scenarios in the Scenarios tab"
            onClick={async () => {
              await archiveScenario(selected.id);
              onSelect(null);
            }}
          >
            <Archive aria-hidden />
          </Button>
        )}
      </div>

      {editing && selected && (
        <form onSubmit={saveEdit} className="flex flex-wrap items-end gap-2">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="scenario-edit-name">Name</Label>
            <Input
              id="scenario-edit-name"
              value={editName}
              onChange={(e) => setEditName(e.target.value)}
              maxLength={SCENARIO_NAME_MAX}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="scenario-edit-status">Status</Label>
            <select
              id="scenario-edit-status"
              value={editStatus}
              onChange={(e) => setEditStatus(e.target.value)}
              className={STATUS_CLASS}
            >
              <option value="draft">Draft</option>
              <option value="active">Active</option>
            </select>
          </div>
          <Button type="submit" disabled={busy}>
            Save
          </Button>
          <Button
            type="button"
            variant="ghost"
            onClick={() => {
              setEditing(false);
              setFormError(null);
            }}
          >
            Cancel
          </Button>
        </form>
      )}

      {creating && (
        <form onSubmit={create} className="flex flex-wrap items-end gap-2">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="scenario-name">New scenario name</Label>
            <Input
              id="scenario-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              maxLength={SCENARIO_NAME_MAX}
              placeholder="e.g. Raise + rent hike"
            />
          </div>
          <Button type="submit" disabled={busy}>
            Create
          </Button>
          <Button
            type="button"
            variant="ghost"
            onClick={() => {
              setCreating(false);
              setName("");
              setFormError(null);
            }}
          >
            Cancel
          </Button>
        </form>
      )}

      {(formError ?? error) && (
        <p role="alert" className="text-sm text-loss">
          {formError ?? error}
        </p>
      )}
    </div>
  );
}
