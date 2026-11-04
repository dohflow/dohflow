import { useRef, useState, type FormEvent } from "react";
import { Archive, Check, Copy, FolderPlus, LineChart, RotateCcw, Trash2, Undo2 } from "lucide-react";

import type { IpcError, ScenarioDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogError,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ApplyConflictNotice } from "@/future-cash/ApplyConflictNotice";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  DataTable,
  type DataTableColumn,
} from "@/components/ui/data-table";
import { formatIsoDate, todayInTimezone } from "@/lib/format";
import { appliedRowProps } from "./appliedRow";
import { RevertScenarioDialog, type RevertPlan } from "./RevertScenarioDialog";
import { cn } from "@/lib/utils";
import { SCENARIO_NAME_MAX, useScenarios } from "@/future-cash/useScenarios";
import { useHouseholdTimezone } from "@/settings/useHouseholdTimezone";
import { describeIpcError, isIpcError } from "@/vault/useVault";

/// What a failed undo says. A typed refusal means the command ran and wrote nothing, so the dialog
/// says so. That is proven in the db-worker, not assumed: `apply_command` runs the whole command, the
/// operation log and the idempotency memo in one transaction that only the final commit publishes, so
/// any returned error was rolled back (`scenario_revert.rs`, and `a_refused_undo_leaves_the_vault_
/// exactly_as_it_was` in `tests/scenario_revert.rs`). Nothing in the kernel or the IPC command can
/// fail after that worker call; a thrown error (the call itself failed) leaves the outcome unknown, so it does not claim
/// either way and points at the forecast (personal-cfo-5ie.12, Studio's F2).
const UNDO_OUTCOME_UNKNOWN =
  "Couldn’t confirm whether the scenario was undone. Check the forecast before trying again.";

/// Shown in the Apply or Delete dialog after the write threw: nobody knows whether it was carried out.
/// The scenarios are read again behind the dialog, so what the screen shows afterwards is what is
/// stored. A typed refusal is not this: it says what was refused, in its own words.
const APPLY_OUTCOME_UNKNOWN =
  "Couldn’t confirm whether the scenario was applied. Close this dialog and check its status in Scenarios before applying it again.";
const DELETE_OUTCOME_UNKNOWN =
  "Couldn’t confirm whether the scenario was deleted. Close this dialog and check Scenarios before trying to delete it again.";

function refusedUndo(error: IpcError): string {
  const detail = describeIpcError(error).trim();
  const sentence = detail === "" ? "" : `${detail.charAt(0).toUpperCase()}${detail.slice(1)}${/[.!?]$/.test(detail) ? "" : "."} `;
  return `That didn’t work, and nothing was undone. ${sentence}Please try again.`;
}

const COPY_SUFFIX = " (copy)";

/// The name of a duplicate: the original plus " (copy)", cut to fit the name limit. A name
/// written before the limit existed can be longer than it, so the base is shortened and the
/// suffix always survives.
function duplicateName(name: string): string {
  const room = SCENARIO_NAME_MAX - COPY_SUFFIX.length;
  return `${Array.from(name.trim()).slice(0, room).join("").trimEnd()}${COPY_SUFFIX}`;
}

function isExpired(scenario: ScenarioDto, today: string): boolean {
  return scenario.expires_on !== null && scenario.expires_on < today;
}

/// The lifecycle state a user actually sees — status plus the derived expiry
/// (ADR 0051 §3: expiry is computed, never a stored status). `today` is the
/// household-local calendar date (ADR 0021 §1, personal-cfo-q329) — the same
/// boundary the backend applying-scenario gate resolves against, not the browser's.
function stateOf(
  scenario: ScenarioDto,
  today: string,
): { label: string; tone: string } {
  if (scenario.status === "archived") {
    return { label: "Archived", tone: "bg-muted text-muted-foreground" };
  }
  if (isExpired(scenario, today)) {
    return { label: "Expired", tone: "bg-muted text-muted-foreground" };
  }
  if (scenario.status === "active") {
    return { label: "Active", tone: "bg-primary/15 text-primary" };
  }
  return { label: "Draft", tone: "bg-muted text-muted-foreground" };
}

/// The Scenarios manager (personal-cfo-4d8.27.6.1, ADR 0049 §1/§6 — the reserved
/// Planning slot).
///
/// Scenario planning used to be a bar wedged into the Cash Flow screen, which made
/// "what plans do I have?" unanswerable without first opening the forecast. This tab
/// owns the LIFECYCLE — create, clone, archive, restore, delete, expire — while the
/// Cash Flow chart keeps the selector that compares one against base (ADR 0051 §5).
/// Opening a scenario routes to Cash Flow with it selected, so the two stay coherent.
/// First run (personal-cfo-m1am, Scenarios mock 1f).
///
/// Every other empty state in this app can say "add an account" or "add a bill" and the
/// reader already knows what those are. A scenario is the one object with no equivalent
/// outside the app — so a bare "New scenario" button asks the user to **invent the
/// concept** before they can use it.
///
/// So: one sentence saying what a scenario IS, the line that makes it safe to try
/// (nothing real changes), then named starting points that PREFILL the form. The first
/// scenario becomes a choice rather than an invention.
///
/// The starting points are examples of the MECHANISM, not suggestions about the reader's
/// finances — worded so they cannot be read as advice to take a leave or expect a raise
/// (ADR 0018).
const STARTING_POINTS = [
  { name: "A raise", hint: "change what an income pays" },
  { name: "A rent increase", hint: "change what a bill costs" },
  { name: "Time off work", hint: "pause an income for a while" },
];

function NoScenariosYet({ onStart }: { onStart: (name: string) => void }) {
  return (
    <div className="flex flex-col gap-3 py-2">
      <div>
        <p className="text-sm font-medium">No scenarios yet.</p>
        <p className="mt-1 max-w-lg text-sm text-muted-foreground">
          A scenario is a set of changes laid over your forecast — a different rent, a
          paused paycheck — so you can see where your cash lands under them.{" "}
          <span className="text-foreground">
            Nothing real changes: your accounts and transactions stay exactly as they are.
          </span>
        </p>
      </div>
      <div className="flex flex-col gap-1.5">
        <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Start from
        </p>
        <div className="flex flex-wrap gap-2">
          {STARTING_POINTS.map((point) => (
            <button
              key={point.name}
              type="button"
              onClick={() => onStart(point.name)}
              className="rounded-md border px-3 py-1.5 text-left text-sm hover:bg-muted"
            >
              <span className="font-medium">{point.name}</span>
              <span className="ml-1.5 text-xs text-muted-foreground">{point.hint}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

export function ScenariosView({
  onOpenInCashFlow,
}: {
  /// Show this scenario against base on the Cash Flow screen.
  onOpenInCashFlow: (id: string) => void;
}) {
  const {
    scenarios,
    error,
    addScenario,
    deleteScenario,
    archiveScenario,
    cloneScenario,
    setScenarioExpiry,
    updateScenario,
    deletedApplied,
    applyScenario,
    previewRevert,
    revertScenarioApply,
    refreshScenarios,
  } = useScenarios();
  const { timezone } = useHouseholdTimezone();
  const today = todayInTimezone(timezone);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [showArchived, setShowArchived] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<ScenarioDto | null>(null);
  // The Apply or Delete write that is running behind its open dialog, and what it reported. The
  // dialog stays open until the write has settled: Escape, Cancel and a click outside do nothing
  // meanwhile, and the outcome shows in the dialog it belongs to.
  const [pendingWrite, setPendingWrite] = useState<"apply" | "delete" | null>(null);
  const [dialogError, setDialogError] = useState<string | null>(null);
  // Applying changes the household's real forecast, so it is confirmed and the dialog
  // says exactly what will and will not change (ADR 0055).
  const [confirmApply, setConfirmApply] = useState<ScenarioDto | null>(null);
  // Undoing an apply is confirmed when the person changed the scenario's items after applying
  // it (undoing removes those changes too), and explained when a later apply has to go first.
  const [revertPlan, setRevertPlan] = useState<RevertPlan | null>(null);
  // Where focus goes when that dialog closes: the Undo button that opened it, or the section
  // heading when that button is gone (an undone scenario stops showing Undo).
  const revertOpener = useRef<HTMLElement | null>(null);
  const headingRef = useRef<HTMLDivElement>(null);

  const all = scenarios ?? [];
  const visible = showArchived ? all : all.filter((s) => s.status !== "archived");
  const archivedCount = all.filter((s) => s.status === "archived").length;

  // Deleting hides a scenario but leaves its apply in the forecast, so the way back stays here
  // rather than needing a reinstate first. It is shown with the empty state too: deleting the last
  // applied scenario leaves nothing to list, but its changes are still in the forecast.
  const deletedAppliedSection =
    deletedApplied.length > 0 ? (
      <div className="flex flex-col gap-2 border-t pt-3">
        <h3 className="text-sm font-medium">Deleted, but still applied</h3>
        <p className="text-xs text-muted-foreground">
          You deleted these after applying them, so their changes are still in your
          forecast. Undo to take them out.
        </p>
        <ul className="flex flex-col gap-1">
          {deletedApplied.map((deleted) => (
            <li
              key={deleted.id}
              className="flex items-center justify-between gap-2 text-sm"
            >
              <span>{deleted.name}</span>
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                aria-label={`Undo applying deleted scenario ${deleted.name}`}
                onClick={(event) => void startRevert(deleted, event.currentTarget)}
              >
                <Undo2 aria-hidden className="size-4" />
              </Button>
            </li>
          ))}
        </ul>
      </div>
    ) : null;

  async function onCreate(event: FormEvent) {
    event.preventDefault();
    if (!name.trim()) {
      setFormError("Give the scenario a name.");
      return;
    }
    setBusy(true);
    const result = await addScenario({ name: name.trim(), description: null });
    setBusy(false);
    if (result.status === "error") {
      setFormError(describeIpcError(result.error));
      return;
    }
    setName("");
    setCreating(false);
    setFormError(null);
  }

  /// Run a row action, surfacing its failure and ALWAYS releasing `busy`.
  ///
  /// The mutations resolve to `IpcError | null` (they do not throw for a handled IPC
  /// failure), so the result has to be inspected or every row action fails silently.
  /// And `typedError` rethrows genuine transport errors, so without `finally` a single
  /// throw would leave every row's buttons disabled until the tab was remounted.
  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    setFormError(null);
    try {
      const result = await action();
      // The mutations resolve to `IpcError | null` (or `{ id } | IpcError` for clone); an
      // `IpcError` is recognized by the generated type's shape, and anything else is success.
      if (isIpcError(result)) setFormError(describeIpcError(result));
    } catch {
      setFormError("That didn't work. Please try again.");
    } finally {
      setBusy(false);
    }
  }

  /// Run the write behind an open Apply or Delete dialog. The dialog is released only once the write
  /// has settled and the scenarios have been read again, so on success focus lands where it can stay
  /// (the row's button is gone by then, so the dialog hands focus to the heading). A refusal shows in
  /// the dialog and leaves it open; so does a call that threw, whose outcome is not known, after the
  /// scenarios are read again so the screen shows what is stored.
  async function confirmWrite(
    kind: "apply" | "delete",
    action: () => Promise<IpcError | null>,
    close: () => void,
  ) {
    setPendingWrite(kind);
    setDialogError(null);
    setBusy(true);
    try {
      // Both writes resolve to `IpcError | null`: a value is the refusal, `null` is success.
      const failure = await action();
      if (failure) {
        setDialogError(describeIpcError(failure as IpcError));
        return;
      }
      await refreshScenarios();
      close();
    } catch {
      // The call itself failed, so the outcome is not known. The sentence says so, and the dialog
      // stays locked until the scenarios have been read again.
      setDialogError(kind === "apply" ? APPLY_OUTCOME_UNKNOWN : DELETE_OUTCOME_UNKNOWN);
      await refreshScenarios();
    } finally {
      setBusy(false);
      setPendingWrite(null);
    }
  }

  /// Undo an apply. With nothing changed since applying it goes straight through; otherwise the
  /// confirmation lists what else is undone (personal-cfo-5ie.12). The ids the preview named are
  /// sent back unchanged, so the backend can tell if they moved in the meantime.
  async function startRevert(target: ScenarioDto, opener: HTMLElement | null) {
    setFormError(null);
    revertOpener.current = opener;
    let result: Awaited<ReturnType<typeof previewRevert>>;
    try {
      result = await previewRevert(target.id);
    } catch {
      setFormError("That didn't work. Please try again.");
      return;
    }
    if (result.status === "error") {
      setFormError(describeIpcError(result.error));
      return;
    }
    const preview = result.data;
    if (preview.blocker_name !== null || preview.lines.length > 0) {
      setRevertPlan({ scenario: target, preview, moved: false, error: null });
      return;
    }
    // Nothing to confirm, so nothing opened. The undone row stops showing Undo, so on success focus
    // goes to the heading rather than to a button that is about to disappear.
    revertOpener.current = null;
    setBusy(true);
    try {
      const error = await revertScenarioApply(target.id, preview.confirmed_descendants);
      if (error === null) headingRef.current?.focus();
      else setFormError(describeIpcError(error));
    } catch {
      // The call itself failed, so the outcome is not known: read the scenarios again so the row
      // shows what is stored, as the dialog path does.
      refreshScenarios();
      setFormError(UNDO_OUTCOME_UNKNOWN);
    } finally {
      setBusy(false);
    }
  }

  /// Send the undo with the list the person confirmed. The dialog stays open until it succeeds. If
  /// the backend refuses because the changes moved meanwhile (another device edited an item, or
  /// a later apply appeared), read the list again and show the fresh one to confirm again; nothing
  /// was written. Any other failure is shown inside the dialog and stays until the next attempt.
  async function confirmUndo(plan: RevertPlan) {
    setBusy(true);
    setRevertPlan({ ...plan, error: null });
    try {
      let error: IpcError | null;
      try {
        error = await revertScenarioApply(plan.scenario.id, plan.preview.confirmed_descendants);
      } catch {
        // The call itself failed, so the outcome is not known: do not say nothing was undone. Read
        // the scenarios and the preview again, so the row and the list show what is really stored.
        refreshScenarios();
        let latest = plan.preview;
        try {
          const fresh = await previewRevert(plan.scenario.id);
          if (fresh.status === "ok") latest = fresh.data;
        } catch {
          // The list stays as it was; the message already says to check the forecast.
        }
        setRevertPlan({ ...plan, preview: latest, error: UNDO_OUTCOME_UNKNOWN });
        return;
      }
      if (error === null) {
        // The row stops being applied, so its Undo button is going away: focus goes to the heading.
        revertOpener.current = null;
        setRevertPlan(null);
        return;
      }
      // A typed refusal wrote nothing. Read the list again: if the changes moved, or a later apply
      // appeared, show the fresh preview to confirm again; otherwise show the reason.
      let fresh: Awaited<ReturnType<typeof previewRevert>> | null = null;
      try {
        fresh = await previewRevert(plan.scenario.id);
      } catch {
        fresh = null;
      }
      const moved =
        fresh !== null &&
        fresh.status === "ok" &&
        (fresh.data.blocker_name !== null ||
          fresh.data.confirmed_descendants.join(",") !==
            plan.preview.confirmed_descendants.join(","));
      if (fresh !== null && fresh.status === "ok" && moved) {
        setRevertPlan({ scenario: plan.scenario, preview: fresh.data, moved: true, error: null });
        return;
      }
      setRevertPlan({ ...plan, error: refusedUndo(error) });
    } finally {
      setBusy(false);
    }
  }

  // Column defs for the shared DataTable (ADR 0053). Sorting is not offered: the list is
  // bounded and ordered by creation, and no column here would order into something more
  // useful than the name.
  const scenarioColumns: DataTableColumn<ScenarioDto>[] = [
    {
      key: "scenario",
      header: "Scenario",
      cell: (scenario) => (
        <>
          <button
            type="button"
            onClick={() => onOpenInCashFlow(scenario.id)}
            className="text-left font-medium transition-colors hover:text-primary"
          >
            {scenario.name}
          </button>
          {scenario.description && (
            <p className="text-xs text-muted-foreground">
              {scenario.description}
            </p>
          )}
        </>
      ),
    },
    {
      key: "state",
      header: "State",
      cell: (scenario) => {
        const state = stateOf(scenario, today);
        return (
          <span className="flex flex-wrap items-center gap-1">
            <span
              className={cn(
                "rounded-full px-2 py-0.5 text-xs font-medium",
                state.tone,
              )}
            >
              {state.label}
            </span>
            {/* Applied-ness is orthogonal to the lifecycle (ADR 0055 §4), so it reads
                ALONGSIDE the state rather than replacing it — an applied scenario can
                still be a draft, or archived. */}
            {scenario.applied_at !== null && (
              <span className="rounded-full bg-gain/15 px-2 py-0.5 text-xs font-medium text-gain">
                Applied
              </span>
            )}
            {/* When it became live. An applied scenario is the only row whose effect
                started at a moment rather than on save, and without the date the user
                cannot tell a change they made this morning from one from six weeks ago
                that they have since forgotten about. */}
            {scenario.applied_at !== null && (
              <span className="text-xs text-muted-foreground">
                since {formatIsoDate(scenario.applied_at.slice(0, 10))}
              </span>
            )}
          </span>
        );
      },
    },
    {
      key: "changes",
      header: "Changes",
      align: "right",
      cell: (scenario) => (
        <>
          {scenario.event_count}
        </>
      ),
    },
    {
      key: "expires",
      header: "Expires",
      cell: (scenario) => (
        <>
          <Input
            type="date"
            aria-label={`Expiry for ${scenario.name}`}
            disabled={busy}
            className="h-8 w-36"
            defaultValue={scenario.expires_on ?? ""}
            key={`${scenario.id}:${scenario.expires_on ?? ""}`}
            onBlur={(e) => {
              const next = e.target.value || null;
              if (next === scenario.expires_on) return;
              void run(() => setScenarioExpiry(scenario.id, next));
            }}
          />
        </>
      ),
    },
    {
      key: "actions",
      header: "Actions",
      align: "right",
      cell: (scenario) => {
        const archived = scenario.status === "archived";
        return (
          <>
          <div className="flex items-center justify-end gap-1">
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Open ${scenario.name} in Cash Flow`}
              onClick={() => onOpenInCashFlow(scenario.id)}
            >
              <LineChart aria-hidden className="size-4" />
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={busy}
              aria-label={`Duplicate ${scenario.name}`}
              onClick={() =>
                void run(() =>
                  cloneScenario(scenario.id, duplicateName(scenario.name)),
                )
              }
            >
              <Copy aria-hidden className="size-4" />
            </Button>
            {scenario.applied_at === null ? (
              <Button
                variant="ghost"
                size="sm"
                // Not offered for a scenario that is not selectable. Archiving means
                // "stop offering this" and expiry means "this has passed" (ADR 0051 §1,
                // §3) — applying either would contradict the state the row is showing.
                // Note the asymmetry with Revert below, which stays available on an
                // archived scenario: archiving must not un-apply anything (ADR 0055 §4),
                // so an applied-then-archived scenario still needs a way back.
                disabled={
                  busy ||
                  scenario.event_count === 0 ||
                  archived ||
                  isExpired(scenario, today)
                }
                aria-label={`Apply ${scenario.name} to the forecast`}
                onClick={() => setConfirmApply(scenario)}
              >
                <Check aria-hidden className="size-4" />
              </Button>
            ) : (
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                aria-label={`Undo applying ${scenario.name}`}
                onClick={(event) => void startRevert(scenario, event.currentTarget)}
              >
                <Undo2 aria-hidden className="size-4" />
              </Button>
            )}
            {archived ? (
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                aria-label={`Restore ${scenario.name}`}
                onClick={() =>
                  void run(() =>
                    updateScenario({
                      id: scenario.id,
                      status: "draft",
                      name: null,
                    }),
                  )
                }
              >
                <RotateCcw aria-hidden className="size-4" />
              </Button>
            ) : (
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                aria-label={`Archive ${scenario.name}`}
                onClick={() => void run(() => archiveScenario(scenario.id))}
              >
                <Archive aria-hidden className="size-4" />
              </Button>
            )}
            <Button
              variant="ghost"
              size="sm"
              disabled={busy}
              aria-label={`Delete ${scenario.name}`}
              onClick={() => setConfirmDelete(scenario)}
            >
              <Trash2 aria-hidden className="size-4 text-loss" />
            </Button>
          </div>
          </>
        );
      },
    },
  ];

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader className="flex flex-row items-start justify-between gap-4">
          <div>
            <CardTitle ref={headingRef} tabIndex={-1} className="focus-visible:outline-none">
              Scenarios
            </CardTitle>
            <CardDescription>
              Try changes and compare them on Cash Flow before you apply them. Comparing does
              not change your real forecast. Apply makes those changes part of your real
              forecast.
            </CardDescription>
          </div>
          <Button size="sm" onClick={() => setCreating((v) => !v)}>
            <FolderPlus aria-hidden className="size-4" />
            New scenario
          </Button>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          {creating && (
            <form onSubmit={onCreate} className="flex flex-wrap items-end gap-2">
              <div className="flex flex-col gap-1">
                <Label htmlFor="scenario-name">Name</Label>
                <Input
                  id="scenario-name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  maxLength={SCENARIO_NAME_MAX}
                  placeholder="Maternity leave"
                  className="w-64"
                />
              </div>
              <Button type="submit" disabled={busy}>
                Create
              </Button>
              <Button type="button" variant="ghost" onClick={() => setCreating(false)}>
                Cancel
              </Button>
            </form>
          )}

          {(error ?? formError) && (
            // A scenario is a set of forecast ASSUMPTIONS, so a failed read here leaves
            // every account, transaction and balance untouched (personal-cfo-m1am, the
            // same distinction drawn for the transactions list in xu32 and the forecast in
            // 4fbl). Saying so is what separates "try again" from "restore from backup".
            <p role="alert" className="text-sm">
              <span className="text-loss">
                Couldn&apos;t load your scenarios. Your accounts, transactions and balances
                are unaffected — scenarios are forecast assumptions, and only that read
                failed.
              </span>{" "}
              <span className="text-muted-foreground">({error ?? formError})</span>
            </p>
          )}

          {scenarios === null ? (
            // Loading is NOT empty. `all` falls back to [] while the read is in flight, so
            // the empty state used to render first and tell the user they had no scenarios
            // before the answer arrived — and that empty state INVITES AN ACTION, so a
            // slow load could prompt someone to create a scenario they already have.
            <p className="text-sm text-muted-foreground">Loading your scenarios…</p>
          ) : all.length === 0 ? (
            <>
              <NoScenariosYet
                onStart={(startingPoint) => {
                  setName(startingPoint);
                  setCreating(true);
                }}
              />
              {deletedAppliedSection}
            </>
          ) : (
            <>
              <DataTable
                columns={scenarioColumns}
                rows={visible}
                rowKey={(scenario) => scenario.id}
                rowProps={appliedRowProps}
                minWidth="min-w-[640px]"
              />
              {deletedAppliedSection}
              {archivedCount > 0 && (
                <button
                  type="button"
                  onClick={() => setShowArchived((v) => !v)}
                  className="self-start text-xs font-medium text-muted-foreground hover:text-foreground"
                >
                  {showArchived
                    ? "Hide archived"
                    : `Show ${archivedCount} archived`}
                </button>
              )}
            </>
          )}
        </CardContent>
      </Card>

      {confirmApply && (
        <ApplyScenarioDialog
          scenario={confirmApply}
          pending={pendingWrite === "apply"}
          error={dialogError}
          fallbackFocusTo={headingRef}
          onCancel={() => {
            setDialogError(null);
            setConfirmApply(null);
          }}
          onConfirm={() => {
            const target = confirmApply;
            void confirmWrite("apply", () => applyScenario(target.id), () => setConfirmApply(null));
          }}
        />
      )}
      <RevertScenarioDialog
        plan={revertPlan}
        busy={busy}
        onCancel={() => setRevertPlan(null)}
        onConfirm={() => {
          if (revertPlan) void confirmUndo(revertPlan);
        }}
        restoreFocusTo={revertOpener}
        fallbackFocusTo={headingRef}
      />
      {confirmDelete && (
        <DeleteScenarioDialog
          // The latest row for the selected scenario, and only while the list read is working: a
          // row from a failed read, or one that has gone, is not known to be applied or not.
          scenario={error === null ? (all.find((s) => s.id === confirmDelete.id) ?? null) : null}
          title={confirmDelete.name}
          pending={pendingWrite === "delete"}
          error={dialogError}
          fallbackFocusTo={headingRef}
          onCancel={() => {
            setDialogError(null);
            setConfirmDelete(null);
          }}
          onConfirm={() => {
            const target = confirmDelete;
            void confirmWrite("delete", () => deleteScenario(target.id), () => setConfirmDelete(null));
          }}
        />
      )}
    </div>
  );
}

/// Applying is the one scenario action that changes the household's real forecast
/// (ADR 0055), so it is confirmed — and the dialog says precisely what moves and what does
/// not, because "apply" could reasonably be read as "edit my bills", which it is not.
function ApplyScenarioDialog({
  scenario,
  pending,
  error,
  fallbackFocusTo,
  onCancel,
  onConfirm,
}: {
  scenario: ScenarioDto;
  /// The apply write is running: nothing dismisses the dialog until it has settled.
  pending: boolean;
  error: string | null;
  /// Where focus goes when the dialog closes and the button that opened it is gone.
  fallbackFocusTo: React.RefObject<HTMLElement | null>;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const safeActionRef = useRef<HTMLButtonElement>(null);
  return (
    <Dialog open onOpenChange={(open) => !open && !pending && onCancel()}>
      <DialogContent initialFocusRef={safeActionRef} fallbackFocusTo={fallbackFocusTo}>
        <DialogHeader>
          <DialogTitle>Apply “{scenario.name}”?</DialogTitle>
        </DialogHeader>
        <DialogBody>
          <DialogDescription className="mt-2">
            Your real forecast will start using this scenario&apos;s {scenario.event_count}{" "}
            {scenario.event_count === 1 ? "change" : "changes"}, instead of only showing them
            when the scenario is selected.
          </DialogDescription>
          {/* What does NOT move, immediately after what does. This is a FACT, not
            reassurance: ADR 0055 promotes assumption events into base and touches no ledger
            rows at all, so nothing here can alter a balance or a transaction. Naming it
            straight after the change is what stops a reversible action reading as a
            dangerous one. */}
          <p className="mt-2 text-sm text-muted-foreground">
            Your bills, income and transactions are not edited — the forecast&apos;s
            assumptions are.
          </p>
          {/* The diff, before the write. Composition is a read-time overlay you can undo by
            deselecting; applying PROMOTES events into base and supersedes what they collide
            with (ADR 0055), so a conflict stops being a view and becomes durable here. */}
          <ApplyConflictNotice scenarioId={scenario.id} />
          {/* Undo LAST, and unconditional. It used to sit before the supersedes block and
            only said "you can undo this" — while the sentence naming the MECHANISM lived
            inside the conflict notice, which renders only when there IS a conflict. So a
            clean apply told the user it was undoable without ever saying how. Superseding
            is the part that sounds permanent, so the answer belongs directly after it. */}
          <p className="mt-3 text-sm text-muted-foreground">
            To undo, use <span className="text-foreground">Revert</span> on this scenario —
            it removes what applying added and restores anything it superseded.
          </p>
        </DialogBody>
        <DialogError error={error} />
        <DialogFooter>
          <Button variant="ghost" ref={safeActionRef} disabled={pending} onClick={onCancel}>
            Cancel
          </Button>
          <Button disabled={pending} onClick={onConfirm}>
            Apply to my forecast
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// Deleting takes a scenario out of every view and the app offers no way back (ADR 0051 §1;
/// underneath it is a tombstone, ADR 0074 Addendum H), so it is confirmed and says exactly
/// what goes — including the change count, so "archive instead" is an informed choice
/// rather than a guess. It does not claim permanence: the rows stay in the vault.
function DeleteScenarioDialog({
  scenario,
  title,
  pending,
  error,
  fallbackFocusTo,
  onCancel,
  onConfirm,
}: {
  /// The latest known row for the scenario being deleted, or `null` when it is not known (the list
  /// read failed, or the row has gone). Nothing about the forecast is claimed for an unknown row.
  scenario: ScenarioDto | null;
  title: string;
  /// The delete write is running: nothing dismisses the dialog until it has settled.
  pending: boolean;
  error: string | null;
  /// Where focus goes when the dialog closes and the button that opened it is gone.
  fallbackFocusTo: React.RefObject<HTMLElement | null>;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const safeActionRef = useRef<HTMLButtonElement>(null);
  const known = scenario !== null && Number.isInteger(scenario.event_count) && scenario.event_count >= 0;
  return (
    <Dialog open onOpenChange={(open) => !open && !pending && onCancel()}>
      <DialogContent initialFocusRef={safeActionRef} fallbackFocusTo={fallbackFocusTo}>
        <DialogHeader>
          <DialogTitle>Delete “{title}”?</DialogTitle>
        </DialogHeader>
        <DialogBody>
          {known && (
            <>
              <DialogDescription className="mt-2">{deleteDescription(scenario)}</DialogDescription>
              <p className="mt-2 text-xs text-muted-foreground">
                Created {formatIsoDate(scenario.created_at.slice(0, 10))}
              </p>
            </>
          )}
        </DialogBody>
        <DialogError error={error} />
        <DialogFooter>
          <Button variant="ghost" ref={safeActionRef} disabled={pending} onClick={onCancel}>
            Cancel
          </Button>
          <Button variant="destructive" disabled={pending || !known} onClick={onConfirm}>
            Delete scenario
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// What deleting a scenario does, for a scenario whose row is known (Studio's lines, personal-cfo-5ie.19).
/// An unapplied scenario leaves the real forecast alone; an applied one does not undo what it already
/// changed there, and Undo apply stays available after deletion. The last sentence is only for a
/// scenario that can still be archived.
function deleteDescription(scenario: ScenarioDto): string {
  const changes = `${scenario.event_count} ${scenario.event_count === 1 ? "change" : "changes"}`;
  const applied = scenario.applied_at !== null;
  const head = `This deletes the scenario and its ${changes}.`;
  const body = applied
    ? "It does not undo the changes already applied to your real forecast. Use Undo apply to reverse those changes; it remains available after deletion."
    : "Your real forecast is not affected.";
  const archive =
    scenario.status === "archived" ? "" : " To keep the plan but stop seeing it, archive it instead.";
  return `${head} ${body}${archive}`;
}
