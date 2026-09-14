import { useState, type FormEvent } from "react";
import { Archive, Check, Copy, FolderPlus, LineChart, RotateCcw, Trash2, Undo2 } from "lucide-react";

import type { IpcError, ScenarioDto } from "@/bindings";
import { Button } from "@/components/ui/button";
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
import { cn } from "@/lib/utils";
import { useScenarios } from "@/future-cash/useScenarios";
import { useHouseholdTimezone } from "@/settings/useHouseholdTimezone";
import { describeIpcError } from "@/vault/useVault";

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
    applyScenario,
    revertScenarioApply,
  } = useScenarios();
  const { timezone } = useHouseholdTimezone();
  const today = todayInTimezone(timezone);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [showArchived, setShowArchived] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<ScenarioDto | null>(null);
  // Applying changes the household's real forecast, so it is confirmed and the dialog
  // says exactly what will and will not change (ADR 0055).
  const [confirmApply, setConfirmApply] = useState<ScenarioDto | null>(null);

  const all = scenarios ?? [];
  const visible = showArchived ? all : all.filter((s) => s.status !== "archived");
  const archivedCount = all.filter((s) => s.status === "archived").length;

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
      // The mutations resolve to `IpcError | null` (or `{ id } | IpcError` for clone);
      // an object carrying `kind` is the error shape.
      if (result && typeof result === "object" && "kind" in result) {
        setFormError(describeIpcError(result as unknown as IpcError));
      }
    } catch {
      setFormError("That didn't work. Please try again.");
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
                  cloneScenario(scenario.id, `${scenario.name} (copy)`),
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
                onClick={() => void run(() => revertScenarioApply(scenario.id))}
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
            <CardTitle>Scenarios</CardTitle>
            <CardDescription>
              Plans layered over your real forecast. A scenario never changes your
              actual numbers — you compare it on Cash Flow.
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
            <NoScenariosYet
              onStart={(startingPoint) => {
                setName(startingPoint);
                setCreating(true);
              }}
            />
          ) : (
            <>
              <DataTable
                columns={scenarioColumns}
                rows={visible}
                rowKey={(scenario) => scenario.id}
                rowProps={appliedRowProps}
                minWidth="min-w-[640px]"
              />
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
          busy={busy}
          onCancel={() => setConfirmApply(null)}
          onConfirm={async () => {
            const target = confirmApply;
            setConfirmApply(null);
            await run(() => applyScenario(target.id));
          }}
        />
      )}
      {confirmDelete && (
        <DeleteScenarioDialog
          scenario={confirmDelete}
          busy={busy}
          onCancel={() => setConfirmDelete(null)}
          onConfirm={async () => {
            const target = confirmDelete;
            setConfirmDelete(null);
            await run(() => deleteScenario(target.id));
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
  busy,
  onCancel,
  onConfirm,
}: {
  scenario: ScenarioDto;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-foreground/40 p-6"
      onClick={onCancel}
      role="presentation"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Apply scenario"
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-md rounded-lg border bg-card p-5 shadow-lg"
      >
        <h2 className="text-base font-semibold">Apply “{scenario.name}”?</h2>
        <p className="mt-2 text-sm text-muted-foreground">
          Your real forecast will start using this scenario&apos;s {scenario.event_count}{" "}
          {scenario.event_count === 1 ? "change" : "changes"}, instead of only showing them
          when the scenario is selected.
        </p>
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
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          <Button disabled={busy} onClick={onConfirm}>
            Apply to my forecast
          </Button>
        </div>
      </div>
    </div>
  );
}

/// Deleting is the one irreversible scenario action (ADR 0051 §1), so it is confirmed
/// and says exactly what goes — including the change count, so "archive instead" is an
/// informed choice rather than a guess.
function DeleteScenarioDialog({
  scenario,
  busy,
  onCancel,
  onConfirm,
}: {
  scenario: ScenarioDto;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-foreground/40 p-6"
      onClick={onCancel}
      role="presentation"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Delete scenario"
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-md rounded-lg border bg-card p-5 shadow-lg"
      >
        <h2 className="text-base font-semibold">Delete “{scenario.name}”?</h2>
        <p className="mt-2 text-sm text-muted-foreground">
          This permanently removes the scenario and its {scenario.event_count}{" "}
          {scenario.event_count === 1 ? "change" : "changes"}. Your real forecast is not
          affected. To keep the plan but stop seeing it, archive it instead.
        </p>
        <p className="mt-2 text-xs text-muted-foreground">
          Created {formatIsoDate(scenario.created_at.slice(0, 10))}
        </p>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          <Button variant="destructive" disabled={busy} onClick={onConfirm}>
            Delete permanently
          </Button>
        </div>
      </div>
    </div>
  );
}
