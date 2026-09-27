// Connecting a bank provider, one surface for Settings and onboarding
// (ADR 0060 addendum 2026-09-02; ADR 0076 §4; personal-cfo-dto2j):
//
//   picker (only when more than one provider is enabled)
//     → the provider's disclosure panel
//     → "Continue" → the provider's credential form.
//
// No credential field exists in the DOM until the user has seen the disclosure
// and chosen to continue. The credential is a one-time secret: it lives in
// local state only until submit and is zeroed on every path (FRONTEND.md §1),
// then goes straight to Rust with the chosen adapter id.

import { Cable, Loader2 } from "lucide-react";
import { useEffect, useId, useState } from "react";

import type { ConnectorAdapterDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";

import { ProviderDisclosure } from "./ProviderDisclosure";
import { ProviderPicker } from "./ProviderPicker";
import { useConnectorAdapters } from "./useConnectorAdapters";

function CredentialForm({
  provider,
  pending,
  onSubmit,
}: {
  provider: ConnectorAdapterDto;
  pending: boolean;
  onSubmit: (credential: string) => Promise<void>;
}) {
  const guide = provider.link_guide;
  const inputId = useId();
  const helpId = useId();
  const [credential, setCredential] = useState("");
  const submit = async () => {
    try {
      await onSubmit(credential.trim());
    } finally {
      // Zero the pasted secret from component state on every path.
      setCredential("");
    }
  };
  return (
    <div className="rounded-md border p-3">
      <Label htmlFor={inputId} className="text-sm">
        {guide.credential_label}
      </Label>
      <p id={helpId} className="mt-1 text-xs text-muted-foreground">
        {guide.paste_instructions}
      </p>
      <form
        className="mt-2 flex items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (credential.trim().length > 0 && !pending) void submit();
        }}
      >
        <Input
          id={inputId}
          type="password"
          autoComplete="off"
          autoFocus
          aria-describedby={helpId}
          value={credential}
          onChange={(event) => setCredential(event.target.value)}
          placeholder={guide.credential_placeholder}
        />
        <Button type="submit" size="sm" disabled={credential.trim().length === 0 || pending}>
          {pending ? <Loader2 className="animate-spin" /> : null}
          Connect
        </Button>
      </form>
    </div>
  );
}

export function ConnectProviderFlow({
  onLink,
  linkPending,
  onCancel,
}: {
  /// Link `adapterId` with the pasted credential; the caller reports the
  /// outcome and closes the flow on success.
  onLink: (adapterId: string, credential: string) => Promise<void>;
  linkPending: boolean;
  onCancel?: () => void;
}) {
  const { enabled, error } = useConnectorAdapters();
  const [chosenId, setChosenId] = useState<string | null>(null);
  const [continued, setContinued] = useState(false);
  const pickerHeadingId = useId();
  const disclosureTitleId = useId();

  const single = enabled && enabled.length === 1 ? enabled[0] : null;
  const chosen =
    single ?? enabled?.find((provider) => provider.adapter_id === chosenId) ?? null;

  // Keyboard and screen-reader users land on each step's heading.
  useEffect(() => {
    if (chosenId) document.getElementById(disclosureTitleId)?.focus();
  }, [chosenId, disclosureTitleId]);

  const cancel = onCancel ? (
    <Button size="sm" variant="ghost" onClick={onCancel}>
      Cancel
    </Button>
  ) : null;

  if (error) {
    return (
      <div className="flex flex-col gap-2">
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
        {cancel}
      </div>
    );
  }
  if (enabled === null) {
    return (
      <div role="status" aria-label="Loading connection providers" className="flex flex-col gap-2">
        <Skeleton className="h-24 w-full" />
      </div>
    );
  }
  if (enabled.length === 0) {
    return (
      <div className="flex flex-col gap-2">
        <EmptyState
          icon={Cable}
          title="No connection providers available"
          description="This version of the app has no bank-connection provider switched on. Manual entry and file imports work as usual."
        />
        {cancel}
      </div>
    );
  }
  if (!chosen) {
    return (
      <div className="flex flex-col gap-2">
        <ProviderPicker
          providers={enabled}
          headingId={pickerHeadingId}
          onChoose={(adapterId) => {
            setContinued(false);
            setChosenId(adapterId);
          }}
        />
        <div>{cancel}</div>
      </div>
    );
  }

  const back =
    enabled.length > 1 ? (
      <Button
        size="sm"
        variant="ghost"
        onClick={() => {
          setChosenId(null);
          setContinued(false);
          // Return to the list's heading once it has rendered.
          requestAnimationFrame(() =>
            document.getElementById(pickerHeadingId)?.focus(),
          );
        }}
      >
        Back to providers
      </Button>
    ) : null;

  return (
    <section aria-labelledby={disclosureTitleId} className="flex flex-col gap-3">
      <ProviderDisclosure adapter={chosen} titleId={disclosureTitleId} />
      {continued ? (
        <CredentialForm
          provider={chosen}
          pending={linkPending}
          onSubmit={(credential) => onLink(chosen.adapter_id, credential)}
        />
      ) : (
        <div>
          <Button size="sm" onClick={() => setContinued(true)}>
            Continue
          </Button>
        </div>
      )}
      <div className="flex items-center gap-2">
        {back}
        {cancel}
      </div>
    </section>
  );
}
