import { useState } from "react";

import type { AccountViewDto, IpcError } from "@/bindings";
import { describeIpcError } from "@/vault/useVault";

import { archiveMovesTheForecast } from "./archiveNotice";

/// Shown when the archive call threw: nobody knows whether the account was archived.
export const ARCHIVE_OUTCOME_UNKNOWN =
  "Couldn’t confirm whether the account was archived. Check your accounts before trying again.";

/// The one archive flow every surface that archives an account shares (personal-cfo-4d8.57), so the
/// forecast-impact question and the way a failure is kept cannot drift between them: an account whose
/// balance anchors the forecast asks first (`archiveMovesTheForecast`), any other archives directly,
/// busy is always released, and a failure stays in `error` until the next attempt.
export function useArchiveFlow(
  /// `null` while the account is not known yet: nothing can be archived until it is.
  account: AccountViewDto | null,
  archive: (accountId: string) => Promise<IpcError | null>,
) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    if (account === null) return;
    setBusy(true);
    setError(null);
    try {
      const failure = await archive(account.id);
      if (failure) setError(describeIpcError(failure));
    } catch {
      setError(ARCHIVE_OUTCOME_UNKNOWN);
    } finally {
      setBusy(false);
    }
  }

  return {
    confirming,
    busy,
    error,
    /// The person asked to archive: ask about the forecast first when it matters, else do it.
    request() {
      if (busy || account === null) return;
      setError(null);
      if (archiveMovesTheForecast(account)) setConfirming(true);
      else void run();
    },
    confirm() {
      setConfirming(false);
      void run();
    },
    cancel() {
      setConfirming(false);
    },
  };
}

export type ArchiveFlow = ReturnType<typeof useArchiveFlow>;
