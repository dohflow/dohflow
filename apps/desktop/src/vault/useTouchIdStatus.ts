import { useCallback, useEffect, useState } from "react";

import { commands, type TouchIdStatusDto } from "@/bindings";

/// Touch ID's status for the active vault, read from the host (personal-cfo-xuu). It carries only a
/// state, a flag, a stable reason code and a count. A failed read leaves `status` null, which every
/// caller treats as "Touch ID is not available": the password path never depends on this. `refresh`
/// returns the status it read, or `undefined` when the read failed, so a caller that must not guess
/// (the Settings card after a call that threw) can tell a confirmed status from a failed read. It only
/// reads: no prompt, no write.
export function useTouchIdStatus() {
  const [status, setStatus] = useState<TouchIdStatusDto | null>(null);
  const [loaded, setLoaded] = useState(false);

  const refresh = useCallback(async (): Promise<TouchIdStatusDto | undefined> => {
    try {
      const read = await commands.touchIdStatus();
      setStatus(read);
      return read;
    } catch {
      setStatus(null);
      return undefined;
    } finally {
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { status, loaded, refresh, setStatus };
}
