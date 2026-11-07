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
    /// The phone's confirmation (a full-screen sheet): the question stays OPEN while the archive runs and when it
    /// fails, so the failure is read where the choice was made and Cancel is still there to leave; it closes only when
    /// the archive worked. (The desktop's inline notice closes at once and shows the failure under the row: `confirm`.)
    async confirmInSheet() {
      if (busy || account === null) return;
      setBusy(true);
      setError(null);
      try {
        const failure = await archive(account.id);
        if (failure) setError(describeIpcError(failure));
        else setConfirming(false);
      } catch {
        setError(ARCHIVE_OUTCOME_UNKNOWN);
      } finally {
        setBusy(false);
      }
    },
    cancel() {
      setConfirming(false);
    },
    /// Leaving the sheet with a failure shown: the failure goes with it (the person has seen it and chose to leave).
    dismissSheet() {
      if (busy) return;
      setConfirming(false);
    },
  };
}

export type ArchiveFlow = ReturnType<typeof useArchiveFlow>;
