import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { PageHeader } from "@/components/PageHeader";
import { GeneralCard } from "./GeneralCard";
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

export function SettingsView({
  onRerunSetup,
}: {
  /// Reopens the first-run guide (personal-cfo-kdw6): the connected-vs-manual
  /// choice, and everything after it, stays revisitable.
  onRerunSetup?: () => void;
} = {}) {
  const summaries = useSettingsSummaries();
  // The updater section exists only when the build explicitly enables self-update. While the
  // capability is unknown or false there is no Software update section, wrapper or placeholder,
  // and no updater command runs (the card, and its hook, are never mounted).
  const selfUpdate = useBuildInfo()?.self_update_available === true;

  return (
    // The page is a container: its sections lay out against the width of the main pane, not the window.
    <div className="@container w-full">
      <div className="flex flex-col gap-4">
        <PageHeader title="Settings" />
        <SettingsGrid>
          {/* Section order is the reading and Tab order; nothing is visually reordered. Every section
              starts open, and each card keeps its own title and body unchanged. */}
          <SettingsSection title="Household" span="full">
            <HouseholdCard />
          </SettingsSection>

          <SettingsSection title="General" span="full">
            <GeneralCard />
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
          <SettingsSection title="Vaults" summary={summaries.vaults} span={onRerunSetup ? "half" : "full"}>
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
