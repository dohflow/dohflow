import { useRef, useState } from "react";
import { Loader2, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogError,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { describeIpcError } from "@/vault/useVault";
import { dollarsToMinorUnits } from "@/lib/format";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { useAccounts } from "./useAccounts";
import { enteredToStoredMinor } from "./balanceSign";

/// Create a new loan account inline from the account editor's "Financed by" link picker
/// (personal-cfo-4d8.23.5): a compact modal for the essentials (name + amount owed) so a
/// property/vehicle can be linked to a mortgage/auto-loan that does not exist yet, without
/// leaving the editor. The created loan is a `loan_liability` (the only valid link target,
/// ADR 0044 §5) and is returned via `onCreated` so the caller auto-selects it. Its APR /
/// due day / other terms can be filled in later by editing the loan.
export function NewLoanDialog({
  currency,
  onCreated,
  onClose,
}: {
  currency: string;
  onCreated: (id: string) => void;
  onClose: () => void;
}) {
  const { createAccount } = useAccounts();
  const [name, setName] = useState("");
  const [amountOwed, setAmountOwed] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const nameRef = useRef<HTMLInputElement>(null);

  async function create() {
    if (name.trim() === "") {
      setError("Enter a name for the loan.");
      return;
    }
    const owed = amountOwed.trim() === "" ? null : dollarsToMinorUnits(amountOwed);
    if (amountOwed.trim() !== "" && owed === null) {
      setError("Enter a valid amount owed.");
      return;
    }
    setBusy(true);
    setError(null);
    const { id, error: failure } = await createAccount({
      name: name.trim(),
      cashflow_role: "LoanLiability",
      subtype: null,
      currency,
      flags: null,
      // Liabilities store a negative signed balance (4d8.23.3).
      opening_balance:
        owed === null
          ? null
          : { minor_units: enteredToStoredMinor("loan_liability", owed), currency },
      idempotency_key: mintIdempotencyKey(),
    });
    setBusy(false);
    if (failure || id === null) {
      setError(failure ? describeIpcError(failure) : "Could not create the loan.");
      return;
    }
    onCreated(id);
    onClose();
  }

  return (
    // Shared Dialog shell (personal-cfo-4d8.6.2). It opens over the account editor, so Escape closes
    // only this dialog (the editor's own Escape listener never sees it) and focus returns to the
    // "New loan" button that opened it.
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        initialFocusRef={nameRef}
        onEscapeKeyDown={(event) => event.stopPropagation()}
        className="max-w-sm"
      >
        <DialogHeader className="flex-row items-center justify-between">
          <DialogTitle>New loan</DialogTitle>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="size-4" aria-hidden />
          </button>
        </DialogHeader>

        <DialogBody className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="new-loan-name">Loan name</Label>
            <Input
              id="new-loan-name"
              ref={nameRef}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Home mortgage"
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="new-loan-owed">Amount owed</Label>
            <Input
              id="new-loan-owed"
              inputMode="decimal"
              value={amountOwed}
              onChange={(e) => setAmountOwed(e.target.value)}
              placeholder="0.00"
            />
            <p className="text-xs text-muted-foreground">
              You can add the APR, due date, and payment terms later by editing this loan.
            </p>
          </div>
        </DialogBody>

        <DialogError error={error} />

        <DialogFooter>
          <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="button" onClick={() => void create()} disabled={busy}>
            {busy && <Loader2 className="animate-spin" aria-hidden />}
            Create loan
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
