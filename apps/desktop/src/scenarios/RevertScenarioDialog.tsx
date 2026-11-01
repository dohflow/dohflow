import { useRef } from "react";

import type { AssumptionEventDto, RevertLineDto, RevertPreviewDto, ScenarioDto } from "@/bindings";
import { useBills } from "@/bills/useBills";
import { useCategories } from "@/categories/useCategories";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogError,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { describeAssumption, type Target } from "@/future-cash/describeChange";
import { useIncome } from "@/income/useIncome";
import { useBaseCurrency } from "@/settings/useBaseCurrency";

/// One change as a stored event, so the sentence that names it is the same one the scenario's
/// own list uses (`describeAssumption`).
function asEvent(line: RevertLineDto, paramsJson: string): AssumptionEventDto {
  return {
    id: "",
    kind: line.kind,
    target_entity_id: line.target_entity_id,
    scenario_id: null,
    params_json: paramsJson,
    created_at: "",
    promoted_from_scenario_id: null,
  };
}

/// What the dialog shows. `null` is closed.
export type RevertPlan = {
  scenario: ScenarioDto;
  preview: RevertPreviewDto;
  /// The undo was refused because the changes moved while the person looked; `preview` is the
  /// fresh list.
  moved: boolean;
  /// A failed undo, shown inside the dialog until the next attempt or until it closes.
  error: string | null;
};

/// The confirmation before undoing an applied scenario whose items were touched afterwards
/// (personal-cfo-5ie.12, owner choice B), on the shared `Dialog` shell (personal-cfo-4d8.6.1).
/// Undoing removes the whole apply, including later edits to its items, so each of those is
/// listed first. It also covers the case where a scenario applied later sits on top, which has to
/// be undone first; then there is nothing to confirm.
///
/// Escape and Cancel close it without sending anything. A failed undo is shown in the dialog's own
/// alert and the dialog stays open. Focus returns to the Undo button that opened it, or to the
/// fallback when that button is gone (the row stops being applied once undone).
export function RevertScenarioDialog({
  plan,
  busy,
  onCancel,
  onConfirm,
  restoreFocusTo,
  fallbackFocusTo,
}: {
  plan: RevertPlan | null;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  restoreFocusTo: React.RefObject<HTMLElement | null>;
  fallbackFocusTo: React.RefObject<HTMLElement | null>;
}) {
  const { bills } = useBills();
  const { sources } = useIncome();
  const { categories } = useCategories();
  const { baseCurrency } = useBaseCurrency();
  const safeActionRef = useRef<HTMLButtonElement>(null);

  const targets: Target[] = [
    ...(bills ?? []).map((b) => ({ id: b.id, name: b.name, type: "bill" as const })),
    ...(sources ?? []).map((s) => ({ id: s.id, name: s.name, type: "income" as const })),
  ];
  const categoryNames = new Map((categories ?? []).map((c) => [c.id, c.name] as const));
  const describe = (line: RevertLineDto, paramsJson: string) =>
    describeAssumption(asEvent(line, paramsJson), targets, baseCurrency, categoryNames);

  return (
    <Dialog
      open={plan !== null}
      onOpenChange={(open) => {
        // A request in flight is not abandoned by Escape or a click outside.
        if (!open && !busy) onCancel();
      }}
    >
      <DialogContent
        initialFocusRef={safeActionRef}
        restoreFocusTo={restoreFocusTo}
        fallbackFocusTo={fallbackFocusTo}
      >
        {plan !== null && plan.preview.blocker_name !== null ? (
          <>
            {/* Only the wrapping title stays pinned; the explanation scrolls with the body so the
                action is never pushed out of a short window. */}
            <DialogHeader>
              <DialogTitle>Undo “{plan.preview.blocker_name}” first</DialogTitle>
            </DialogHeader>
            <DialogBody>
              <DialogDescription>
                “{plan.preview.blocker_name}” was applied after “{plan.scenario.name}” and builds
                on it, so it has to be undone first. Then you can undo “{plan.scenario.name}”.
              </DialogDescription>
            </DialogBody>
            <DialogFooter>
              <Button ref={safeActionRef} onClick={onCancel}>
                OK
              </Button>
            </DialogFooter>
          </>
        ) : plan !== null ? (
          <>
            <DialogHeader>
              <DialogTitle>Undo “{plan.scenario.name}”?</DialogTitle>
            </DialogHeader>
            {/* The status, the intro, the lines and the note all scroll together: a long scenario
                name or a long intro wraps and scrolls, and Cancel and Undo stay in view. */}
            <DialogBody className="flex flex-col gap-3">
              {plan.moved && (
                <p role="status" className="text-foreground">
                  This changed while you were looking, and nothing was undone. Check the list
                  below, then confirm again.
                </p>
              )}
              <DialogDescription>
                Undoing removes the changes this scenario added to your forecast, including later
                edits to them. If applying replaced earlier assumptions, they return unless a
                newer assumption is kept instead. Review what happens below:
              </DialogDescription>
              <ul className="flex flex-col gap-2">
                {plan.preview.lines.map((line, index) => (
                  <li key={`${line.kind}-${index}`}>
                    {line.change === "edited" && line.after_params_json !== null && (
                      <>
                        You changed “{describe(line, line.before_params_json)}” to “
                        {describe(line, line.after_params_json)}”.
                      </>
                    )}
                    {line.change === "cleared" && (
                      <>You removed “{describe(line, line.before_params_json)}”.</>
                    )}
                    {line.change === "kept_newer" && line.after_params_json !== null && (
                      <>
                        You added “{describe(line, line.after_params_json)}” after applying. It
                        stays, and the earlier “{describe(line, line.before_params_json)}” is not
                        brought back.
                      </>
                    )}
                  </li>
                ))}
              </ul>
              <p className="text-muted-foreground">
                Your bills, income and transactions are not edited.
              </p>
            </DialogBody>
            <DialogError error={plan.error} />
            <DialogFooter>
              {/* Inert while the undo is in flight: closing now would hide a write that still lands. */}
              <Button variant="ghost" ref={safeActionRef} disabled={busy} onClick={onCancel}>
                Cancel
              </Button>
              <Button disabled={busy} onClick={onConfirm}>
                Undo scenario
              </Button>
            </DialogFooter>
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
