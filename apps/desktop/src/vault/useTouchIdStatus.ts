import { useCallback, useEffect, useState } from "react";

import { commands, type TouchIdStatusDto } from "@/bindings";

/// Touch ID's status for the active vault, read from the host (personal-cfo-xuu). It carries only a
/// state, a flag, a stable reason code and a count. A failed read leaves `status` null, which every
/// caller treats as "Touch ID is not available": the password path never depends on this.
export function useTouchIdStatus() {
  const [status, setStatus] = useState<TouchIdStatusDto | null>(null);
  const [loaded, setLoaded] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await commands.touchIdStatus());
    } catch {
      setStatus(null);
    } finally {
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { status, loaded, refresh, setStatus };
}
