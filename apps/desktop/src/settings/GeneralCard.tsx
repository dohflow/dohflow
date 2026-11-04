import { useState } from "react";
import { Loader2 } from "lucide-react";

import { Label } from "@/components/ui/label";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { describeIpcError } from "@/vault/useVault";

import { PersistentNotice } from "./PersistentSlot";
import { useBaseCurrency } from "./useBaseCurrency";
import { useKeepFocusAcrossMove } from "./useKeepFocusAcrossMove";

const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

const CURRENCIES: { value: string; label: string }[] = [
  { value: "USD", label: "US Dollar (USD)" },
  { value: "EUR", label: "Euro (EUR)" },
];

/// General settings: the base currency (ADR 0020). A failed read or save keeps its message and the
/// one control that recovers from it, the currency select, in the section's persistent slot (open
/// or closed); the select is rendered once, there while an error shows, else in the body. "Saved."
/// is this card's own status: it says nothing about, and clears nothing from, any other section.
export function GeneralCard() {
  const { baseCurrency, error: loadError, setBaseCurrency } = useBaseCurrency();
  // Optimistic selection so the dropdown doesn't flicker back while the write +
  // refetch are in flight; cleared (falls back to the query value) on failure.
  const [selected, setSelected] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const select = useKeepFocusAcrossMove<HTMLSelectElement>({ settled: !saving });

  const value = selected ?? baseCurrency;
  const message = error ?? loadError;

  async function onChange(code: string) {
    select.remember();
    setSelected(code);
    setSaving(true);
    setSaved(false);
    setError(null);
    const failure = await setBaseCurrency(code);
    setSaving(false);
    if (failure) {
      setSelected(null);
      setError(describeIpcError(failure));
    } else {
      setSaved(true);
    }
  }

  const currencySelect = (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor="base-currency">Base currency</Label>
      <select
        ref={select.ref}
        id="base-currency"
        className={SELECT_CLASS}
        value={value}
        disabled={saving}
        onChange={(event) => void onChange(event.target.value)}
      >
        {CURRENCIES.map((currency) => (
          <option key={currency.value} value={currency.value}>
            {currency.label}
          </option>
        ))}
      </select>
    </div>
  );

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base">General</CardTitle>
        <CardDescription>Defaults that apply across the app.</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="flex flex-col gap-1.5">
          {message === null && currencySelect}
          <p className="text-xs text-muted-foreground">
            New accounts, income, and bills default to this currency. You can
            still override the currency on each one.
          </p>
          {saving && (
            <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <Loader2 className="size-3.5 animate-spin" aria-hidden />
              Saving…
            </p>
          )}
          {saved && !saving && <p className="text-xs text-gain">Saved.</p>}
        </div>
        <PersistentNotice>
          {message !== null && (
            <div className="flex flex-col gap-2">
              <p role="alert" className="text-xs text-loss">
                {message}
              </p>
              {currencySelect}
            </div>
          )}
        </PersistentNotice>
      </CardContent>
    </Card>
  );
}
