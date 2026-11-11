import { useState } from "react";
import { Loader2 } from "lucide-react";

import { Label } from "@/components/ui/label";
import { NativeSelectValue } from "@/components/ui/native-select";
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
import { COMPACT_WRAP_BUTTONS } from "@/lib/sheet";
import { useBaseCurrency } from "./useBaseCurrency";
import { AppearanceCard } from "./AppearanceCard";
import { HouseholdCard } from "./HouseholdCard";
import { ComfortBandCard } from "./ComfortBandCard";
import { AutoCategorizeOnImportCard } from "./AutoCategorizeOnImportCard";
import { ConnectionsCard } from "./ConnectionsCard";
import { VaultHealthCard } from "./VaultHealthCard";
import { DiagnosticsCard } from "./DiagnosticsCard";
import { ChangePasswordCard } from "./ChangePasswordCard";
import { RotateKeyCard } from "./RotateKeyCard";
import { TouchIdCard } from "./TouchIdCard";
import { SoftwareUpdateCard } from "./SoftwareUpdateCard";
import { AboutCard } from "./AboutCard";
import { VaultsCard } from "./VaultsCard";
import { DeleteVaultCard } from "./DeleteVaultCard";
import { BackupScheduleCard } from "@/backup/BackupScheduleCard";
import { SettingsGrid, SettingsSection } from "./SettingsSection";
import { useSettingsSummaries } from "./settingsSummaries";
import { useBuildInfo } from "./BuildBadge";

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

  const summaries = useSettingsSummaries();
  // The updater section exists only when the build explicitly enables self-update. While the
  // capability is unknown or false there is no Software update section, wrapper or placeholder,
  // and no updater command runs (the card, and its hook, are never mounted).
  const selfUpdate = useBuildInfo()?.self_update_available === true;

  return (
    // The page is a container: its sections lay out against the width of the main pane, not the window.
    // On a phone every button in Settings may wrap its label and grow taller (the shared Button is nowrap and fixed-height, so
    // "Run the setup guide again" ran past the screen at the largest text size). One rule here, not a class at every call site;
    // the 44 pt floor in globals.css still holds.
    <div className={`@container w-full ${COMPACT_WRAP_BUTTONS}`}>
      <div className="flex flex-col gap-4">
        <PageHeader title="Settings" />
        <SettingsGrid>
          {/* Section order is the reading and Tab order; nothing is visually reordered. Every section
              starts open, and each card keeps its own title and body unchanged. */}
          <SettingsSection title="Household" span="full">
            <HouseholdCard />
          </SettingsSection>

          <SettingsSection title="General" span="full">
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-base">General</CardTitle>
                <CardDescription>Defaults that apply across the app.</CardDescription>
              </CardHeader>
              <CardContent className="flex flex-col gap-4">
                <div className="@container flex flex-col gap-1.5">
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
                  <NativeSelectValue data-testid="base-currency-value">
                    {CURRENCIES.find((currency) => currency.value === value)?.label}
                  </NativeSelectValue>
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
          </SettingsSection>

          <SettingsSection title="Appearance" span="full">
            <AppearanceCard />
          </SettingsSection>

          <SettingsSection title="Backups" summary={summaries.backups} span="oneOfThree">
            <BackupScheduleCard />
          </SettingsSection>
          <SettingsSection title="Comfort band" summary={summaries.comfort} span="twoOfThree">
            <ComfortBandCard />
          </SettingsSection>
          <SettingsSection
            title="Auto-categorize imported transactions"
            summary={summaries.autoCategorize}
            span="full"
          >
            <AutoCategorizeOnImportCard />
          </SettingsSection>

          <SettingsSection title="Connections" summary={summaries.connections} span="full">
            <ConnectionsCard />
          </SettingsSection>

          {/* With every section open the health group is Health on its own row, then (when this
              build can update itself) Software update and Diagnostics side by side. Without the
              updater there is no Software update section at all, and Diagnostics takes its own row. */}
          <SettingsSection title="Vault health" span="full">
            <VaultHealthCard />
          </SettingsSection>
          {selfUpdate && (
            <SettingsSection title="Software update" span="half">
              <SoftwareUpdateCard />
            </SettingsSection>
          )}
          <SettingsSection title="Diagnostics" span={selfUpdate ? "half" : "full"}>
            <DiagnosticsCard />
          </SettingsSection>

          {/* Vaults; the setup guide sits beside it when the host provides it (kdw6), and Vaults has
              the whole row when it does not (no placeholder). */}
          <SettingsSection title="Vaults" span={onRerunSetup ? "half" : "full"}>
            <VaultsCard />
          </SettingsSection>
          {onRerunSetup ? (
            <SettingsSection title="Setup guide" span="half">
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
            </SettingsSection>
          ) : null}

          {/* Until the destructive group has its own guards, each is its own full row, in order. */}
          <SettingsSection title="Touch ID" span="full">
            <TouchIdCard />
          </SettingsSection>
          <SettingsSection title="Change master password" span="full">
            <ChangePasswordCard />
          </SettingsSection>
          <SettingsSection title="Rotate encryption key" span="full">
            <RotateKeyCard />
          </SettingsSection>
          <SettingsSection title="Delete this vault" span="full">
            <DeleteVaultCard />
          </SettingsSection>

          {/* Name, build identity, site links, and the quiet Support row (n76x.18). */}
          <SettingsSection title="About" summary={summaries.about} span="full">
            <AboutCard />
          </SettingsSection>
        </SettingsGrid>
      </div>
    </div>
  );
}
