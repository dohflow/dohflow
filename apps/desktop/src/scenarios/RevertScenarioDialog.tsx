import type { AssumptionEventDto, RevertLineDto, RevertPreviewDto, ScenarioDto } from "@/bindings";
import { useBills } from "@/bills/useBills";
import { useCategories } from "@/categories/useCategories";
import { Button } from "@/components/ui/button";
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

/// The confirmation before undoing an applied scenario whose items were touched afterwards
/// (personal-cfo-5ie.12, owner choice B). Undoing removes the whole apply, including later edits
/// to its items, so each of those is listed first. It also covers the case where a scenario
/// applied later sits on top, which has to be undone first; then there is nothing to confirm.
export function RevertScenarioDialog({
  scenario,
  preview,
  busy,
  moved,
  onCancel,
  onConfirm,
}: {
  scenario: ScenarioDto;
  preview: RevertPreviewDto;
  busy: boolean;
  /// The undo was refused because the changes moved while the person looked; `preview` is the
  /// fresh list.
  moved: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { bills } = useBills();
  const { sources } = useIncome();
  const { categories } = useCategories();
  const { baseCurrency } = useBaseCurrency();
  const targets: Target[] = [
    ...(bills ?? []).map((b) => ({ id: b.id, name: b.name, type: "bill" as const })),
    ...(sources ?? []).map((s) => ({ id: s.id, name: s.name, type: "income" as const })),
  ];
  const categoryNames = new Map((categories ?? []).map((c) => [c.id, c.name] as const));
  const describe = (line: RevertLineDto, paramsJson: string) =>
    describeAssumption(asEvent(line, paramsJson), targets, baseCurrency, categoryNames);

  const blocked = preview.blocker_name !== null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-foreground/40 p-6"
      onClick={onCancel}
      role="presentation"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={blocked ? "Undo another scenario first" : "Undo scenario"}
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-md rounded-lg border bg-card p-5 shadow-lg"
      >
        {blocked ? (
          <>
            <h2 className="text-base font-semibold">
              Undo “{preview.blocker_name}” first
            </h2>
            <p className="mt-2 text-sm text-muted-foreground">
              “{preview.blocker_name}” was applied after “{scenario.name}” and builds on it, so
              it has to be undone first. Then you can undo “{scenario.name}”.
            </p>
            <div className="mt-4 flex justify-end">
              <Button onClick={onCancel} autoFocus>
                OK
              </Button>
            </div>
          </>
        ) : (
          <>
            <h2 className="text-base font-semibold">Undo “{scenario.name}”?</h2>
            {moved && (
              <p role="status" className="mt-2 text-sm text-foreground">
                This changed while you were looking, and nothing was undone. Check the list
                below, then confirm again.
              </p>
            )}
            <p className="mt-2 text-sm text-muted-foreground">
              Undoing removes what applying added to your forecast and brings back what it
              replaced. You changed some of it after applying, and those changes are undone
              too:
            </p>
            <ul className="mt-3 flex flex-col gap-2 text-sm">
              {preview.lines.map((line, index) => (
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
            <p className="mt-3 text-sm text-muted-foreground">
              Your bills, income and transactions are not edited.
            </p>
            <div className="mt-4 flex justify-end gap-2">
              <Button variant="ghost" onClick={onCancel} autoFocus>
                Cancel
              </Button>
              <Button disabled={busy} onClick={onConfirm}>
                Undo scenario
              </Button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
