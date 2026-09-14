import { useMemo, useState, type FormEvent } from "react";
import {
  ArrowLeft,
  ChevronRight,
  Eye,
  EyeOff,
  Loader2,
  LockKeyhole,
  Plus,
  Search,
} from "lucide-react";

import type { VaultSummaryDto } from "@/bindings";
import { BrandLockup } from "@/brand/Brand";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { describeIpcError, useVault } from "@/vault/useVault";
import { PasswordStrengthMeter } from "@/vault/PasswordStrengthMeter";
import { RestoreFromBackup } from "@/backup/RestoreFromBackup";
import { NoResetWarning } from "./NoResetWarning";

const MIN_LENGTH = 8;

/// A deterministic decorative sparkline for a vault row (Claude Design "Vault Loader" 1a).
/// Seeded from the vault id so each vault keeps a stable shape across launches — a generic
/// movement marker, NOT the vault's real balances (those stay encrypted until unlock).
export function sparklinePoints(seed: string): { points: string; end: [number, number] } {
  let h = 2166136261;
  for (const ch of seed) {
    h = Math.imul(h ^ ch.charCodeAt(0), 16777619) >>> 0;
  }
  const pts: [number, number][] = [];
  let y = 16 + (h % 9) - 4;
  for (let i = 0; i < 8; i += 1) {
    h = Math.imul(h ^ (i + 1), 16777619) >>> 0;
    y = Math.min(28, Math.max(4, y + ((h % 15) - 7)));
    pts.push([Math.round((i * 96) / 7), Math.round(y)]);
  }
  const end = pts[pts.length - 1] ?? [96, 16];
  return { points: pts.map(([x, py]) => `${x},${py}`).join(" "), end };
}

function VaultSparkline({ id }: { id: string }) {
  const spark = sparklinePoints(id);
  return (
    <svg
      width="72"
      height="26"
      viewBox="0 0 96 32"
      fill="none"
      aria-hidden
      className="overflow-visible text-primary/70"
    >
      <polygon
        points={`0,32 ${spark.points} 96,32`}
        fill="currentColor"
        opacity="0.1"
      />
      <polyline
        points={spark.points}
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <circle cx={spark.end[0]} cy={spark.end[1]} r="2.4" fill="currentColor" />
    </svg>
  );
}

/// The month + year a vault was created, from its RFC-3339 stamp ("Created June 2026").
function createdLabel(createdAt: string): string {
  const date = new Date(createdAt);
  if (Number.isNaN(date.getTime())) return "On this device";
  return `Created ${date.toLocaleDateString(undefined, { month: "long", year: "numeric" })}`;
}

/// The launch-screen vault picker (personal-cfo-j0cg.6.1, Claude Design "Vault Loader"
/// option 1a — the ledger list): choose WHICH vault to unlock before typing a password,
/// create a new vault, or restore a backup — all without unlocking anything first. The
/// picked vault is remembered (the registry's active entry) for the next launch.
export function VaultPickerScreen() {
  const { vaults, switchVault } = useVault();
  const [query, setQuery] = useState("");
  const [unlocking, setUnlocking] = useState<VaultSummaryDto | null>(null);
  const [creating, setCreating] = useState(false);
  const [switchError, setSwitchError] = useState<string | null>(null);

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (needle === "") return vaults;
    return vaults.filter((vault) => vault.name.toLowerCase().includes(needle));
  }, [vaults, query]);

  async function pick(vault: VaultSummaryDto) {
    setSwitchError(null);
    if (!vault.is_active) {
      // Re-point the controller first (the registry remembers it for next launch);
      // the vault comes up locked and the modal collects its password.
      const failure = await switchVault(vault.id);
      if (failure) {
        setSwitchError(describeIpcError(failure));
        return;
      }
    }
    setUnlocking(vault);
  }

  return (
    <main className="flex min-h-screen flex-col items-center bg-background px-8 pt-12 pb-6">
      <div className="flex h-full w-full max-w-[476px] flex-1 flex-col">
        {/* The stacked lockup above the vault list (personal-cfo-4d8.28.3): the mark
            centered over the wordmark, the tagline aligned to the wordmark's left edge. */}
        <div className="mb-6 flex flex-col items-start gap-2">
          <BrandLockup variant="stacked" height={88} />
          <span className="text-xs leading-none text-muted-foreground">
            Local · encrypted on this device
          </span>
        </div>

        <h1 className="mb-1 text-[27px] font-semibold leading-tight tracking-[-0.02em]">
          Welcome back
        </h1>
        <p className="mb-5 text-[15px] text-muted-foreground">Choose a vault to unlock.</p>

        {vaults.length > 3 && (
          <div className="relative mb-4">
            <Search
              className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
              aria-hidden
            />
            <Input
              type="search"
              aria-label="Search vaults"
              placeholder="Search vaults…"
              className="pl-9"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
          </div>
        )}

        {switchError && (
          <p role="alert" className="mb-3 text-sm text-loss">
            {switchError}
          </p>
        )}

        <div className="-mx-2 flex flex-1 flex-col gap-2 overflow-y-auto px-2 pb-2">
          {visible.map((vault) => (
            <button
              key={vault.id}
              type="button"
              onClick={() => void pick(vault)}
              className="flex w-full shrink-0 items-center gap-3.5 rounded-xl border bg-card px-4 py-3 text-left transition-all hover:-translate-y-px hover:border-primary hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <div className="flex size-10 shrink-0 items-center justify-center rounded-[11px] bg-secondary text-base font-semibold text-primary">
                {vault.name.trim().charAt(0).toUpperCase() || "V"}
              </div>
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="truncate text-[14.5px] font-semibold leading-tight">
                  {vault.name}
                </span>
                <span className="text-xs leading-none text-muted-foreground">
                  {vault.is_active ? "Last opened" : createdLabel(vault.created_at)}
                </span>
              </div>
              <VaultSparkline id={vault.id} />
              <ChevronRight className="size-[18px] shrink-0 text-muted-foreground" aria-hidden />
            </button>
          ))}
          {visible.length === 0 && (
            <div className="px-3 py-8 text-center text-sm leading-relaxed text-muted-foreground">
              No vaults match that name.
              <br />
              Try a different search or create a new vault.
            </div>
          )}
        </div>

        <div className="flex items-center justify-between gap-3 border-t pt-3 pb-1">
          <span className="text-xs text-muted-foreground">
            {vaults.length} vault{vaults.length === 1 ? "" : "s"} on this device
          </span>
          <Button size="sm" onClick={() => setCreating(true)}>
            <Plus aria-hidden />
            New vault
          </Button>
        </div>
        <RestoreFromBackup caption="Have an encrypted backup? Restore it as a vault instead." />
      </div>

      {unlocking && (
        <UnlockVaultModal vault={unlocking} onBack={() => setUnlocking(null)} />
      )}
      {creating && <CreateVaultModal onBack={() => setCreating(false)} />}
    </main>
  );
}

/// The per-vault unlock dialog (design 1a's modal): the vault's name, its master
/// password with show/hide, and a no-reset "Forgot?" note that points at backup restore
/// (there is deliberately no password reset — ADR 0002).
function UnlockVaultModal({
  vault,
  onBack,
}: {
  vault: VaultSummaryDto;
  onBack: () => void;
}) {
  const { unlockVault } = useVault();
  const [password, setPassword] = useState("");
  const [show, setShow] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [forgot, setForgot] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  async function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (password.length === 0 || submitting) return;
    setSubmitting(true);
    setError(null);
    const failure = await unlockVault(password);
    // Drop the typed password from state either way (defense-in-depth); on success the
    // provider status flips to Unlocked and the router swaps screens.
    setPassword("");
    if (failure) {
      setError(describeIpcError(failure));
      setSubmitting(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-foreground/50 backdrop-blur-[3px]"
      role="dialog"
      aria-modal="true"
      aria-label={`Unlock ${vault.name}`}
      onClick={onBack}
    >
      <div
        className="relative w-[414px] max-w-[calc(100vw-44px)] rounded-2xl border bg-popover p-8 pb-7 shadow-2xl"
        onClick={(event) => event.stopPropagation()}
      >
        <button
          type="button"
          title="Back"
          onClick={onBack}
          className="absolute left-4 top-4 flex size-8 items-center justify-center rounded-lg text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <ArrowLeft className="size-[19px]" aria-hidden />
          <span className="sr-only">Back to vaults</span>
        </button>

        <div className="flex flex-col items-center gap-3.5 text-center">
          <div className="flex size-14 items-center justify-center rounded-2xl bg-primary shadow-[0_10px_22px_-8px] shadow-primary">
            <LockKeyhole className="size-7 text-primary-foreground" aria-hidden />
          </div>
          <div>
            <h2 className="mb-1 text-xl font-semibold leading-tight">{vault.name}</h2>
            <p className="text-[13.5px] text-muted-foreground">
              Enter your master password to unlock.
            </p>
          </div>
        </div>

        <form onSubmit={onSubmit} className="mt-5 flex flex-col gap-2">
          <Label htmlFor="picker-password">Master password</Label>
          <div className="relative">
            <Input
              id="picker-password"
              type={show ? "text" : "password"}
              autoComplete="current-password"
              autoFocus
              placeholder="Enter password"
              className="pr-11"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              aria-invalid={error !== null}
            />
            <button
              type="button"
              title={show ? "Hide password" : "Show password"}
              onClick={() => setShow((v) => !v)}
              className="absolute right-1.5 top-1/2 flex size-8 -translate-y-1/2 items-center justify-center text-muted-foreground hover:text-foreground"
            >
              {show ? (
                <EyeOff className="size-[18px]" aria-hidden />
              ) : (
                <Eye className="size-[18px]" aria-hidden />
              )}
              <span className="sr-only">{show ? "Hide password" : "Show password"}</span>
            </button>
          </div>

          <div className="flex justify-end">
            <button
              type="button"
              onClick={() => setForgot((v) => !v)}
              className="py-0.5 text-[12.5px] font-medium text-primary hover:underline"
            >
              Forgot master password?
            </button>
          </div>

          {forgot && (
            <p className="rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-xs leading-relaxed text-foreground">
              There is no password reset — the vault is encrypted with this password alone.
              If you have an encrypted backup, go back and restore it instead.
            </p>
          )}

          {error && (
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          )}

          <Button
            type="submit"
            className="mt-1 w-full"
            disabled={password.length === 0 || submitting}
          >
            {submitting && <Loader2 className="animate-spin" aria-hidden />}
            {submitting ? "Unlocking…" : "Unlock"}
          </Button>
        </form>
      </div>
    </div>
  );
}

/// Create a new vault from the picker (name + password + the canonical no-reset
/// acknowledgement). On success the new vault is active and unlocked — the router
/// drops straight into it.
function CreateVaultModal({ onBack }: { onBack: () => void }) {
  const { createNamedVault } = useVault();
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [acknowledged, setAcknowledged] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const tooShort = password.length > 0 && password.length < MIN_LENGTH;
  const mismatch = confirm.length > 0 && confirm !== password;
  const canSubmit =
    name.trim().length > 0 &&
    password.length >= MIN_LENGTH &&
    password === confirm &&
    acknowledged &&
    !submitting;

  async function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    const failure = await createNamedVault(name.trim(), password);
    // On success the provider status flips to Unlocked in the new vault.
    if (failure) {
      setError(describeIpcError(failure));
      setSubmitting(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-foreground/50 backdrop-blur-[3px]"
      role="dialog"
      aria-modal="true"
      aria-label="Create a new vault"
      onClick={onBack}
    >
      <div
        className="relative max-h-[calc(100vh-44px)] w-[440px] max-w-[calc(100vw-44px)] overflow-y-auto rounded-2xl border bg-popover p-8 pb-7 shadow-2xl"
        onClick={(event) => event.stopPropagation()}
      >
        <button
          type="button"
          title="Back"
          onClick={onBack}
          className="absolute left-4 top-4 flex size-8 items-center justify-center rounded-lg text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <ArrowLeft className="size-[19px]" aria-hidden />
          <span className="sr-only">Back to vaults</span>
        </button>

        <div className="flex flex-col items-center gap-2 text-center">
          <div className="flex size-14 items-center justify-center rounded-2xl bg-primary shadow-[0_10px_22px_-8px] shadow-primary">
            <Plus className="size-7 text-primary-foreground" aria-hidden />
          </div>
          <h2 className="text-xl font-semibold leading-tight">New vault</h2>
          <p className="text-[13.5px] text-muted-foreground">
            A separate, fully encrypted space with its own password.
          </p>
        </div>

        <form onSubmit={onSubmit} className="mt-5 flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="new-vault-name">Name</Label>
            <Input
              id="new-vault-name"
              autoFocus
              placeholder="e.g. Family, Test"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="new-vault-password">Master password</Label>
            <Input
              id="new-vault-password"
              type="password"
              autoComplete="new-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              aria-invalid={tooShort}
            />
            <PasswordStrengthMeter password={password} />
            {tooShort && (
              <p className="text-xs text-loss">Use at least {MIN_LENGTH} characters.</p>
            )}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="new-vault-confirm">Confirm password</Label>
            <Input
              id="new-vault-confirm"
              type="password"
              autoComplete="new-password"
              value={confirm}
              onChange={(event) => setConfirm(event.target.value)}
              aria-invalid={mismatch}
            />
            {mismatch && <p className="text-xs text-loss">Passwords don’t match.</p>}
          </div>

          <NoResetWarning
            acknowledged={acknowledged}
            onAcknowledgedChange={setAcknowledged}
          />

          {error && (
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          )}

          <Button type="submit" className="w-full" disabled={!canSubmit}>
            {submitting && <Loader2 className="animate-spin" aria-hidden />}
            {submitting ? "Creating…" : "Create vault"}
          </Button>
        </form>
      </div>
    </div>
  );
}
