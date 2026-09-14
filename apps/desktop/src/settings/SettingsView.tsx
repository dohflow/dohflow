import { useState } from "react";
import { Loader2 } from "lucide-react";

import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { PageHeader } from "@/components/PageHeader";
import { describeIpcError } from "@/vault/useVault";
import { useBaseCurrency } from "./useBaseCurrency";
import { AppearanceCard } from "./AppearanceCard";
import { HouseholdCard } from "./HouseholdCard";
import { ComfortBandCard } from "./ComfortBandCard";
import { AutoCategorizeOnImportCard } from "./AutoCategorizeOnImportCard";
import { ConnectionsCard } from "./ConnectionsCard";
import { VaultHealthCard } from "./VaultHealthCard";
import { ChangePasswordCard } from "./ChangePasswordCard";
import { SoftwareUpdateCard } from "./SoftwareUpdateCard";
import { AboutCard } from "./AboutCard";
import { VaultsCard } from "./VaultsCard";
import { DeleteVaultCard } from "./DeleteVaultCard";

const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

const CURRENCIES: { value: string; label: string }[] = [
  { value: "USD", label: "US Dollar (USD)" },
  { value: "EUR", label: "Euro (EUR)" },
];

export function SettingsView({
  onRerunSetup,
}: {
  /// Reopens the first-run guide (personal-cfo-kdw6): the connected-vs-manual
  /// choice, and everything after it, stays revisitable.
  onRerunSetup?: () => void;
} = {}) {
  const { baseCurrency, error: loadError, setBaseCurrency } = useBaseCurrency();
  // Optimistic selection so the dropdown doesn't flicker back while the write +
  // refetch are in flight; cleared (falls back to the query value) on failure.
  const [selected, setSelected] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const value = selected ?? baseCurrency;

  async function onChange(code: string) {
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

  return (
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-4">
      <PageHeader title="Settings" />
      <HouseholdCard />
      <AppearanceCard />
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">General</CardTitle>
          <CardDescription>Defaults that apply across the app.</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="base-currency">Base currency</Label>
            <select
              id="base-currency"
              className={SELECT_CLASS}
              value={value}
              disabled={saving}
              onChange={(event) => onChange(event.target.value)}
            >
              {CURRENCIES.map((currency) => (
                <option key={currency.value} value={currency.value}>
                  {currency.label}
                </option>
              ))}
            </select>
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
            {(error ?? loadError) && (
              <p role="alert" className="text-xs text-loss">
                {error ?? loadError}
              </p>
            )}
          </div>
        </CardContent>
      </Card>

      {onRerunSetup ? (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-sm">Setup guide</CardTitle>
            <CardDescription>
              Walk through connecting banks or entering by hand, accounts,
              income, and bills again. Nothing you have already added changes.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Button size="sm" variant="outline" onClick={onRerunSetup}>
              Run the setup guide again
            </Button>
          </CardContent>
        </Card>
      ) : null}

      <ComfortBandCard />

      <AutoCategorizeOnImportCard />
      <ConnectionsCard />

      <VaultHealthCard />

      <ChangePasswordCard />

      <SoftwareUpdateCard />

      {/* Name, build identity, site links, and the quiet Support row (n76x.18). */}
      <AboutCard />

      <VaultsCard />

      <DeleteVaultCard />
    </div>
  );
}
