import { useQuery } from "@tanstack/react-query";

import type { BackupCadenceDto, BackupHistoryEntryDto, ComfortBandDto, MoneyDto } from "@/bindings";
import { useBackupHistory, useBackupScheduleSettings } from "@/backup/useBackup";
import { useComfortBand } from "@/dashboard/useComfortBand";
import { formatMoney } from "@/lib/format";
import { queryKeys } from "@/lib/query";
import { useVault } from "@/vault/useVault";

import { useBuildInfo } from "./BuildBadge";
import { useConnections } from "./useConnections";

/// The one-line summaries a collapsed Settings section shows (personal-cfo-4d8.39.1). Each is built
/// from data the page already reads, in the approved wording, and is `undefined` (no summary) unless
/// that data has loaded and says exactly what the line claims: nothing is shown while a read is
/// loading or has failed, and a line that would say "healthy" or "verified" needs the data that
/// proves it.

const CADENCE_LABEL: Record<BackupCadenceDto, string | undefined> = {
  off: undefined,
  daily: "Daily",
  weekly: "Weekly",
  monthly: "Monthly",
};

/// "[Frequency] · last backup verified": the schedule is on, and the most recent backup on record
/// was verified.
export function backupsSummary(
  cadence: BackupCadenceDto | undefined,
  history: BackupHistoryEntryDto[] | undefined,
): string | undefined {
  const label = cadence === undefined ? undefined : CADENCE_LABEL[cadence];
  if (label === undefined) return undefined;
  return history?.[0]?.verified === true ? `${label} · last backup verified` : undefined;
}

/// "[lower]–[upper]": both edges of the comfort band, formatted as money. No upper edge, no summary.
export function comfortSummary(band: ComfortBandDto | null): string | undefined {
  if (band === null || band.upper === null) return undefined;
  const money = (m: MoneyDto) => formatMoney(m);
  return `${money(band.lower)}–${money(band.upper)}`;
}

/// "On", only when the setting has loaded and is enabled.
export function autoCategorizeSummary(enabled: boolean | undefined): string | undefined {
  return enabled === true ? "On" : undefined;
}

/// "[count] connections", for two or more. A single connection has no approved wording.
export function connectionsSummary(count: number | undefined): string | undefined {
  return count !== undefined && count >= 2 ? `${count} connections` : undefined;
}

/// "[vault name] · active".
export function vaultsSummary(activeName: string | undefined): string | undefined {
  return activeName === undefined || activeName === "" ? undefined : `${activeName} · active`;
}

/// "DohFlow · version [version]".
export function aboutSummary(version: string | undefined): string | undefined {
  return version === undefined || version === "" ? undefined : `DohFlow · version ${version}`;
}

export function useSettingsSummaries() {
  const schedule = useBackupScheduleSettings();
  const history = useBackupHistory();
  const { band } = useComfortBand();
  // Read from the cache the card already fills; `enabled: false` makes this an observer, not a fetch.
  const autoCategorize = useQuery<boolean>({
    queryKey: queryKeys.autoCategorizeOnImport,
    enabled: false,
  });
  const { connections } = useConnections();
  const { vaults } = useVault();
  const build = useBuildInfo();

  return {
    backups: backupsSummary(schedule.data?.cadence, history.data),
    comfort: comfortSummary(band),
    autoCategorize: autoCategorizeSummary(autoCategorize.data),
    connections: connectionsSummary(connections?.length),
    vaults: vaultsSummary(vaults.find((v) => v.is_active)?.name),
    about: aboutSummary(build?.version),
  };
}
