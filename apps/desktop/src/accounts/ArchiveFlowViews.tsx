import type { AccountViewDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { formatMoney } from "@/lib/format";

import { storedToShownMinor } from "./balanceSign";
import type { ArchiveFlow } from "./archiveFlow";

/// What archiving this account does to the forecast, and the choice to go ahead. Renders only while
/// the flow is confirming.
export function ArchiveImpactConfirm({
  account,
  flow,
}: {
  account: AccountViewDto;
  flow: ArchiveFlow;
}) {
  if (!flow.confirming) return null;
  const shown = formatMoney({
    minor_units: storedToShownMinor(account.cashflow_role, account.balance.minor_units),
    currency: account.balance.currency,
  });
  return (
    <div
      role="alertdialog"
      aria-label={`Archive ${account.name}`}
      className="mt-2 rounded-md border bg-muted/40 p-3 text-sm"
    >
      <p>
        {`Archiving ${account.name} stops counting its ${shown} toward your forecast, so your projected cash will drop by that much. Restoring the account brings it back.`}
      </p>
      <div className="mt-2 flex gap-1">
        <Button
          size="sm"
          variant="ghost"
          onClick={flow.confirm}
          aria-label={`Confirm archive ${account.name}`}
        >
          Archive
        </Button>
        <Button size="sm" variant="ghost" type="button" onClick={flow.cancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

/// A failed archive, kept on screen until the next attempt.
export function ArchiveFailure({ flow }: { flow: ArchiveFlow }) {
  if (flow.error === null) return null;
  return (
    <p role="alert" className="mt-2 text-sm text-loss">
      {flow.error}
    </p>
  );
}
