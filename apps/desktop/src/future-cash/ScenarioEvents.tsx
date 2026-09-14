import { useState, type FormEvent } from "react";
import { Plus, Trash2 } from "lucide-react";

import type { CreateForecastAssumptionInput } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { dollarsToMinorUnits } from "@/lib/format";
import { describeAssumption, previewChange } from "./describeChange";
import { cn } from "@/lib/utils";
import { describeIpcError } from "@/vault/useVault";
import { useBills } from "@/bills/useBills";
import { useIncome } from "@/income/useIncome";
import { useCategories } from "@/categories/useCategories";
import { useScenarioEvents } from "./useScenarioEvents";

/// A base income/bill a modification or exclusion can target.
type Target = { id: string; name: string; type: "bill" | "income" };

/// What an event change does — the form's top-level choice.
type ChangeKind = "add" | "amount" | "date" | "remove" | "spend";

const CHANGE_LABELS: Record<ChangeKind, string> = {
  add: "Add money",
  amount: "Change amount",
  date: "Change date",
  remove: "Remove",
  spend: "Spend more or less",
};

/// Today as `YYYY-MM-DD` in local time (the `<input type="date">` default).
function todayIso(): string {
  const now = new Date();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${now.getFullYear()}-${month}-${day}`;
}

/// Per-scenario event management (personal-cfo-6zep): add additions (one-off cash),
/// amount/date modifications, or removals against the base income/bills, and list /
/// delete them. Every change invalidates the forecast, so the chart and compare
/// delta refresh live. Rendered only when a scenario is selected.
export function ScenarioEvents({
  scenarioId,
  currency,
}: {
  scenarioId: string;
  currency: string;
}) {
  const { events, error, addEvent, deleteEvent } = useScenarioEvents(scenarioId);
  const { bills } = useBills();
  const { sources } = useIncome();
  const { categories: allCategories } = useCategories();
  const categoryNames = new Map(
    (allCategories ?? []).map((c) => [c.id, c.name] as const),
  );
  const [open, setOpen] = useState(false);

  const targets: Target[] = [
    ...(bills ?? [])
      .filter((b) => b.active)
      .map((b) => ({ id: b.id, name: b.name, type: "bill" as const })),
    ...(sources ?? []).map((s) => ({
      id: s.id,
      name: s.name,
      type: "income" as const,
    })),
  ];

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-3">
        <CardTitle className="text-sm">What-if changes</CardTitle>
        {!open && (
          <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
            <Plus aria-hidden />
            Add change
          </Button>
        )}
      </CardHeader>
      <CardContent className="flex flex-col gap-3 p-0">
        {error && (
          <p role="alert" className="px-6 text-sm text-loss">
            {error}
          </p>
        )}

        {open && (
          <EventForm
            scenarioId={scenarioId}
            currency={currency}
            targets={targets}
            onCancel={() => setOpen(false)}
            onSubmit={async (input) => {
              const failure = await addEvent(input);
              if (!failure) setOpen(false);
              return failure ? describeIpcError(failure) : null;
            }}
          />
        )}

        {events && events.length > 0 ? (
          <ul>
            {events.map((event) => (
              <li
                key={event.id}
                className="flex items-center justify-between gap-4 border-t px-6 py-2.5 text-sm"
              >
                <span className="min-w-0 truncate">
                  {describeAssumption(event, targets, currency, categoryNames)}
                </span>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label="Delete change"
                  onClick={() => void deleteEvent(event.id)}
                >
                  <Trash2 aria-hidden />
                </Button>
              </li>
            ))}
          </ul>
        ) : (
          !open && (
            <p className="px-6 pb-6 text-sm text-muted-foreground">
              Add a what-if change — extra cash, a different bill amount or date, or
              dropping an item — to see how it shifts this scenario.
            </p>
          )
        )}
      </CardContent>
    </Card>
  );
}

function EventForm({
  scenarioId,
  currency,
  targets,
  onCancel,
  onSubmit,
}: {
  scenarioId: string;
  currency: string;
  targets: Target[];
  onCancel: () => void;
  onSubmit: (input: CreateForecastAssumptionInput) => Promise<string | null>;
}) {
  const [change, setChange] = useState<ChangeKind>("add");
  // Empty until the user picks; falls back to the first target so the default is
  // valid even if the bills/income lists finish loading after the form opens.
  const [targetId, setTargetId] = useState<string>("");
  const selectedTargetId = targetId || targets[0]?.id || "";
  const [label, setLabel] = useState("");
  const [amount, setAmount] = useState("");
  const [direction, setDirection] = useState<"in" | "out">("in");
  const [eventDate, setEventDate] = useState(todayIso());
  const [anchorDate, setAnchorDate] = useState(todayIso());
  const [effectiveDate, setEffectiveDate] = useState("");
  const [endDate, setEndDate] = useState("");
  const [selectedCategoryId, setSelectedCategoryId] = useState("");
  // Its own control rather than reusing the in/out money toggle: "spend less" is the
  // common case and reads nothing like "money in".
  const [spendDirection, setSpendDirection] = useState<"less" | "more">("less");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { categories } = useCategories();
  // Spend overrides target a CATEGORY, which is never in `targets` (bills + income), so
  // the preview needs the names to say more than "a category".
  const categoryNames = new Map(
    (categories ?? []).map((c) => [c.id, c.name] as const),
  );
  // Only categories the spend model actually projects can be adjusted (ADR 0026 §7
  // addendum) — offering a fixed or transfer category would accept a change that
  // silently does nothing.
  const spendCategories = (categories ?? []).filter(
    (c) =>
      !c.archived &&
      (c.forecast_behavior === "variable_regular" ||
        c.forecast_behavior === "variable_lumpy"),
  );

  // "spend" targets a CATEGORY, so it does not need a bill/income target.
  const needsTarget = change !== "add" && change !== "spend";
  const noTargets = needsTarget && targets.length === 0;

  function build(): CreateForecastAssumptionInput | string {
    const base: CreateForecastAssumptionInput = {
      kind: "one_time_event",
      scenario_id: scenarioId,
      target_entity_id: null,
      amount: null,
      date: null,
      label: null,
      new_amount_minor: 0,
      new_anchor_date: null,
      effective_date: null,
      end_date: null,
    };
    const target = targets.find((t) => t.id === selectedTargetId) ?? null;
    const effective = effectiveDate.trim() ? effectiveDate : null;
    const end = endDate.trim() ? endDate : null;

    if (change === "add") {
      const magnitude = dollarsToMinorUnits(amount);
      if (magnitude === null || magnitude <= 0)
        return "Enter an amount greater than zero.";
      if (!label.trim()) return "Add a short label.";
      const minor = direction === "out" ? -magnitude : magnitude;
      return {
        ...base,
        kind: "one_time_event",
        amount: { minor_units: minor, currency },
        date: eventDate,
        label: label.trim(),
      };
    }
    if (change === "spend") {
      // A planned change to a CATEGORY's discretionary spend (4d8.27.6.2). Signed:
      // "less" is stored negative. It adjusts the modelled spend draw, not an event.
      if (!selectedCategoryId) return "Pick a category.";
      const magnitude = dollarsToMinorUnits(amount);
      if (magnitude === null || magnitude <= 0)
        return "Enter an amount greater than zero.";
      if (effective && end && end < effective)
        return "The end date must be on or after the start date.";
      return {
        ...base,
        kind: "variable_spend_override",
        target_entity_id: selectedCategoryId,
        new_amount_minor: spendDirection === "less" ? -magnitude : magnitude,
        effective_date: effective,
        end_date: end,
      };
    }
    if (!target) return "Pick an item to change.";
    if (change === "amount") {
      const magnitude = dollarsToMinorUnits(amount);
      // 0 is a valid override here — an income or bill can drop to 0 for the
      // period (e.g. unpaid leave). Only one-off additions (above) require a
      // non-zero amount (personal-cfo-yiau).
      if (magnitude === null || magnitude < 0)
        return "Enter an amount of zero or more.";
      // A bounded window must end on or after it starts (personal-cfo-w6o9).
      if (effective && end && end < effective)
        return "The end date must be on or after the start date.";
      return {
        ...base,
        kind: target.type === "bill" ? "bill_amount" : "income_amount",
        target_entity_id: target.id,
        new_amount_minor: magnitude,
        effective_date: effective,
        end_date: end,
      };
    }
    if (change === "date") {
      return {
        ...base,
        kind: target.type === "bill" ? "bill_date" : "income_date",
        target_entity_id: target.id,
        new_anchor_date: anchorDate,
      };
    }
    // remove
    return {
      ...base,
      kind: "exclusion",
      target_entity_id: target.id,
      effective_date: effective,
    };
  }

  async function submit(formEvent: FormEvent) {
    formEvent.preventDefault();
    const built = build();
    if (typeof built === "string") {
      setError(built);
      return;
    }
    setBusy(true);
    setError(null);
    const failure = await onSubmit(built);
    setBusy(false);
    if (failure) setError(failure);
  }

  return (
    <form
      onSubmit={submit}
      className="flex flex-col gap-3 border-y bg-muted/30 px-6 py-4"
    >
      <div className="flex flex-col gap-1.5">
        <span className="text-sm font-medium">Change</span>
        <div
          className="inline-flex rounded-md border p-0.5"
          role="group"
          aria-label="Change type"
        >
          {(Object.keys(CHANGE_LABELS) as ChangeKind[]).map((option) => (
            <button
              key={option}
              type="button"
              onClick={() => setChange(option)}
              aria-pressed={change === option}
              className={cn(
                "rounded px-2.5 py-1 text-xs font-medium transition-colors",
                change === option
                  ? "bg-secondary text-secondary-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {CHANGE_LABELS[option]}
            </button>
          ))}
        </div>
      </div>

      {noTargets ? (
        <p className="text-sm text-muted-foreground">
          Add a bill or income source first to change or remove it.
        </p>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          {needsTarget && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="event-target">Item</Label>
              <select
                id="event-target"
                value={selectedTargetId}
                onChange={(e) => setTargetId(e.target.value)}
                className="h-9 rounded-md border bg-background px-3 text-sm"
              >
                {targets.map((target) => (
                  <option key={target.id} value={target.id}>
                    {target.name} ({target.type})
                  </option>
                ))}
              </select>
            </div>
          )}

          {change === "spend" && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="event-category">Category</Label>
              <select
                id="event-category"
                value={selectedCategoryId}
                onChange={(e) => setSelectedCategoryId(e.target.value)}
                className="h-9 rounded-md border bg-background px-3 text-sm"
              >
                <option value="">Pick a category…</option>
                {spendCategories.map((category) => (
                  <option key={category.id} value={category.id}>
                    {category.name}
                  </option>
                ))}
              </select>
              <p className="text-xs text-muted-foreground">
                Applies to spending your forecast has learned — that needs about six
                months of categorised spending from a cash account. Spending on a credit
                card reaches the forecast through the card payment, so a change here
                won&rsquo;t move it yet. Two changes to the same category add up.
              </p>
              {spendCategories.length === 0 && (
                <p className="text-xs text-muted-foreground">
                  No variable-spend categories yet — only categories your forecast
                  models as variable spend can be adjusted.
                </p>
              )}
              <div
                className="inline-flex rounded-md border p-0.5"
                role="group"
                aria-label="Spend direction"
              >
                {(["less", "more"] as const).map((option) => (
                  <button
                    key={option}
                    type="button"
                    onClick={() => setSpendDirection(option)}
                    aria-pressed={spendDirection === option}
                    className={cn(
                      "flex-1 rounded px-2.5 py-1 text-xs font-medium capitalize transition-colors",
                      spendDirection === option
                        ? "bg-secondary text-secondary-foreground"
                        : "text-muted-foreground hover:text-foreground",
                    )}
                  >
                    {option}
                  </button>
                ))}
              </div>
            </div>
          )}

          {change === "add" && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="event-label">Label</Label>
              <Input
                id="event-label"
                value={label}
                onChange={(e) => setLabel(e.target.value)}
                placeholder="e.g. Bonus"
              />
            </div>
          )}

          {(change === "add" || change === "amount" || change === "spend") && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="event-amount">
                {change === "add"
                  ? `Amount (${currency})`
                  : change === "spend"
                    ? `Change per month (${currency})`
                    : `New amount (${currency})`}
              </Label>
              <Input
                id="event-amount"
                inputMode="decimal"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                placeholder="0.00"
              />
            </div>
          )}

          {change === "add" && (
            <>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="event-date">Date</Label>
                <Input
                  id="event-date"
                  type="date"
                  value={eventDate}
                  onChange={(e) => setEventDate(e.target.value)}
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <span className="text-sm font-medium">Direction</span>
                <div
                  className="inline-flex rounded-md border p-0.5"
                  role="group"
                  aria-label="Direction"
                >
                  {(["in", "out"] as const).map((option) => (
                    <button
                      key={option}
                      type="button"
                      onClick={() => setDirection(option)}
                      aria-pressed={direction === option}
                      className={cn(
                        "flex-1 rounded px-2.5 py-1 text-xs font-medium transition-colors",
                        direction === option
                          ? "bg-secondary text-secondary-foreground"
                          : "text-muted-foreground hover:text-foreground",
                      )}
                    >
                      {option === "in" ? "Money in" : "Money out"}
                    </button>
                  ))}
                </div>
              </div>
            </>
          )}

          {change === "date" && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="event-anchor">New start date</Label>
              <Input
                id="event-anchor"
                type="date"
                value={anchorDate}
                onChange={(e) => setAnchorDate(e.target.value)}
              />
            </div>
          )}

          {(change === "amount" || change === "remove" || change === "spend") && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="event-effective">Starting from (optional)</Label>
              <Input
                id="event-effective"
                type="date"
                value={effectiveDate}
                onChange={(e) => setEffectiveDate(e.target.value)}
              />
            </div>
          )}

          {change === "amount" && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="event-end">Until (optional)</Label>
              <Input
                id="event-end"
                type="date"
                value={endDate}
                onChange={(e) => setEndDate(e.target.value)}
              />
            </div>
          )}
        </div>
      )}

      {error && (
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
      )}

      {/* One plain sentence restating the RESULT before it is saved (personal-cfo-c5en).
          A form of individually correct fields can still add up to something the user did
          not intend; this is the last point at which that is cheap to notice.

          Built from the same `build()` the submit uses, so it cannot describe a different
          change than the one that would be saved — and rendered through the same describer
          as the saved row, so it is the same sentence afterwards. */}
      {(() => {
        const pending = build();
        if (typeof pending === "string") return null;
        return (
          <p className="text-sm">
            <span className="text-muted-foreground">This will add: </span>
            <span className="font-medium">
              {previewChange(pending, targets, currency, categoryNames)}
            </span>
          </p>
        );
      })()}

      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" disabled={busy || noTargets}>
          Add change
        </Button>
      </div>
    </form>
  );
}
