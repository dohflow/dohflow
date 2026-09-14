// Create a brand-new account straight from the connection mapping step
// (personal-cfo-07bn): the NewLoanDialog idiom, prefilled from the provider's
// account name. No opening balance is asked for — the next sync brings the
// provider's balance (yl53), so the user maps and moves on.

import { useEffect, useState } from "react";
import { Loader2, X } from "lucide-react";

import type { CashflowRoleDto } from "@/bindings";
import { useAccounts } from "@/accounts/useAccounts";
import { ROLE_DTO_TO_TOKEN, subtypesForRoleToken } from "@/accounts/subtypes";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { describeIpcError } from "@/vault/useVault";

const ROLE_OPTIONS: { value: CashflowRoleDto; label: string }[] = [
  { value: "LiquidCash", label: "Cash / bank" },
  { value: "CreditFacility", label: "Credit card" },
  { value: "LoanLiability", label: "Loan" },
  { value: "InvestmentAsset", label: "Investment" },
  { value: "RealAsset", label: "Property or vehicle" },
];

export function NewMappedAccountDialog({
  externalName,
  currency,
  onCreated,
  onClose,
}: {
  /// The provider's name for the account — the natural default name.
  externalName: string;
  currency: string;
  onCreated: (id: string) => void;
  onClose: () => void;
}) {
  const { createAccount } = useAccounts();
  const [name, setName] = useState(externalName);
  const [role, setRole] = useState<CashflowRoleDto>("LiquidCash");
  const [subtype, setSubtype] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      // Never dismiss mid-create: the create still resolves and maps, and a
      // failure would report to an unmounted dialog (review of 07bn).
      if (event.key === "Escape" && !busy) onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, busy]);

  const subtypes = subtypesForRoleToken(ROLE_DTO_TO_TOKEN[role] ?? "");

  async function create() {
    if (name.trim() === "") {
      setError("Enter a name for the account.");
      return;
    }
    setBusy(true);
    setError(null);
    const { id, error: failure } = await createAccount({
      name: name.trim(),
      cashflow_role: role,
      subtype: subtype || null,
      currency,
      flags: null,
      // The provider's balance arrives with the next sync (yl53).
      opening_balance: null,
      idempotency_key: mintIdempotencyKey(),
    });
    setBusy(false);
    if (failure || id === null) {
      setError(failure ? describeIpcError(failure) : "Could not create the account.");
      return;
    }
    onCreated(id);
    onClose();
  }

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center p-4">
      <div
        className="absolute inset-0 bg-foreground/40"
        aria-hidden
        onClick={busy ? undefined : onClose}
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label="New account for this connection"
        className="relative flex w-full max-w-sm flex-col gap-4 rounded-lg border bg-background p-6 shadow-xl"
      >
        <div className="flex items-center justify-between">
          <h2 className="font-semibold">New account</h2>
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            aria-label="Close"
            className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="size-4" aria-hidden />
          </button>
        </div>
        <p className="text-sm text-muted-foreground">
          Tracks the provider&rsquo;s &ldquo;{externalName}&rdquo;, created in{" "}
          {currency} (your base currency). Its balance and history arrive with
          the next sync.
        </p>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="new-mapped-name">Account name</Label>
          <Input
            id="new-mapped-name"
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="new-mapped-role">Type</Label>
          <NativeSelect
            id="new-mapped-role"
            value={role}
            onChange={(e) => {
              setRole(e.target.value as CashflowRoleDto);
              setSubtype("");
            }}
          >
            {ROLE_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </NativeSelect>
        </div>
        {subtypes.length > 0 ? (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="new-mapped-subtype">Subtype</Label>
            <NativeSelect
              id="new-mapped-subtype"
              value={subtype}
              onChange={(e) => setSubtype(e.target.value)}
            >
              <option value="">— None —</option>
              {subtypes.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </NativeSelect>
          </div>
        ) : null}

        {error ? (
          <p role="alert" className="text-sm text-loss">
            {error}
          </p>
        ) : null}

        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button size="sm" onClick={() => void create()} disabled={busy}>
            {busy ? <Loader2 className="animate-spin" aria-hidden /> : null}
            Create and map
          </Button>
        </div>
      </div>
    </div>
  );
}
