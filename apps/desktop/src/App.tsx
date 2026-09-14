import { useState } from "react";
import { Loader2 } from "lucide-react";
import { QueryClientProvider } from "@tanstack/react-query";

import { createQueryClient } from "@/lib/queryClient";
import { VaultProvider, useVault } from "@/vault/useVault";
import { CreateVaultScreen } from "@/vault/screens/CreateVaultScreen";
import { UnlockScreen } from "@/vault/screens/UnlockScreen";
import { VaultPickerScreen } from "@/vault/screens/VaultPickerScreen";
import { UnlockedHome } from "@/vault/screens/UnlockedHome";
import { BuildBadge } from "@/settings/BuildBadge";
import { RecoveryScreen } from "@/vault/screens/RecoveryScreen";
import { Screen } from "@/vault/screens/Screen";
import { ThemeProvider } from "@/theme/ThemeProvider";
import { ThemeToggle } from "@/theme/ThemeToggle";

// The vault is the app's front door (personal-cfo-8v2 / -3ry): the screen the
// user sees is a pure function of the vault lifecycle state, which the kernel
// classifies on launch and the lifecycle commands update. The QueryClient (ADR
// 0020) wraps the vault provider so vault lock can clear the read-model cache.
//
// ThemeProvider wraps EVERYTHING, outside VaultProvider — deliberately (personal-cfo-17u1
// review, Finding 1): appearance must follow the system, and stay changeable, on the
// locked/picker screens too, not just once a vault is unlocked. There must be exactly
// one ThemeProvider instance in the whole app.
export default function App() {
  const [queryClient] = useState(createQueryClient);
  return (
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        <VaultProvider>
          <VaultRouter />
        </VaultProvider>
      </QueryClientProvider>
    </ThemeProvider>
  );
}

function Busy({ label }: { label: string }) {
  return (
    <Screen>
      <div className="flex items-center justify-center gap-2 text-muted-foreground">
        <Loader2 className="size-5 animate-spin" aria-hidden />
        {label}
      </div>
    </Screen>
  );
}

function VaultRouter() {
  const { status, loadError, vaults } = useVault();
  // Everything except Unlocked renders without the sidebar, so the build identity is
  // pinned to a corner there — the launch/unlock moment is exactly when "is this my
  // rebuild?" gets asked (personal-cfo-4d8.27.3.2).
  const badge = status?.state === "Unlocked" ? null : <BuildBadge floating />;

  const screen = () => {
    if (loadError) {
      return (
        <Screen>
          <div role="alert" className="text-center text-sm text-loss">
            {loadError}
          </div>
        </Screen>
      );
    }

    if (status === null) {
      return <Busy label="Checking vault…" />;
    }

    switch (status.state) {
      case "NoVault":
        // Registered vaults exist but none is openable here (e.g. the active one was
        // deleted out-of-band) — the picker can switch or create without unlocking.
        return vaults.length > 0 ? <VaultPickerScreen /> : <CreateVaultScreen />;
      case "Locked":
        // Multi-vault: pick WHICH vault to unlock (Claude Design "Vault Loader" 1a);
        // the single-vault/no-registry case keeps the plain unlock form.
        return vaults.length > 0 ? <VaultPickerScreen /> : <UnlockScreen />;
      case "Unlocked":
        // Unlocked shows it in the sidebar footer instead (no floating duplicate).
        return <UnlockedHome />;
      case "CorruptNeedsRecovery":
        return <RecoveryScreen />;
      default:
        // Transient states (CreatingVault / Unlocking / Locking / …) are momentary
        // — an action is mid-flight. Show a neutral busy state.
        return <Busy label="Working…" />;
    }
  };

  return (
    <>
      {screen()}
      {badge}
      {/* Unconditional — unlike `badge`, shown on EVERY vault state (locked, picker,
          unlocked, recovery, transient) so the appearance control is reachable no
          matter what screen is up (personal-cfo-17u1 review, Finding 1). Opposite
          corner from BuildBadge so the two never collide. */}
      <ThemeToggle floating />
    </>
  );
}
