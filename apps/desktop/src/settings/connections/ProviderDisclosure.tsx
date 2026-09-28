// What a bank-connection provider is, stated plainly before anyone pastes a
// credential (ADR 0060 addendum 2026-09-02; ADR 0076 §4–§5; personal-cfo-dto2j).
// Everything shown comes from the provider's registry entry — the four
// disclosure points (ADR 0015 §5) and its link guide — so the component itself
// carries no provider-specific copy. Descriptive only (ADR 0018): it explains,
// it never recommends.
//
// This generalizes the onboarding panel it replaces (personal-cfo-kdw6) and
// keeps its structure element for element: SimpleFIN's rendered text is pinned
// byte for byte to the shipped panel (ProviderDisclosure.test.tsx).
//
// URLs are plain, selectable text, never anchors: the app's only opener grant
// is scoped to https://dohflow.app/* (ADR 0010), and a provider or referral
// URL must never become a click-through (ADR 0076 §5).

import { type ReactNode, useId } from "react";

import type { ConnectorAdapterDto } from "@/bindings";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

import { splitLead } from "./providerCopy";

function SelectableUrl({ url }: { url: string }) {
  return (
    // break-all: a long URL wraps at narrow widths instead of overflowing.
    <code className="select-all break-all rounded bg-muted px-1 font-mono text-xs">
      {url}
    </code>
  );
}

/// A setup step, with every occurrence of the provider's URL set as
/// selectable text.
function stepWithUrl(step: string, url: string): ReactNode[] {
  const parts = step.split(url);
  return parts.flatMap((part, index) =>
    index === 0
      ? [part]
      : [<SelectableUrl key={`${url}-${index}`} url={url} />, part],
  );
}

function Point({ text }: { text: string }) {
  const { lead, rest } = splitLead(text);
  return (
    <li className="flex gap-2">
      <span aria-hidden>·</span>
      <span>
        {lead ? (
          <>
            <b>{lead}</b> {rest}
          </>
        ) : (
          rest
        )}
      </span>
    </li>
  );
}

export function ProviderDisclosure({
  adapter,
  titleId,
}: {
  adapter: ConnectorAdapterDto;
  /// Id for the heading, so a flow can move focus to it and label its region.
  titleId?: string;
}) {
  const { disclosure, link_guide: guide, referral } = adapter;
  const referralId = useId();
  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle id={titleId} tabIndex={-1} className="text-base outline-none">
          {guide.title}
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-4 text-sm">
        <p>{disclosure.handles_credentials}</p>
        <ul className="flex flex-col gap-2">
          <Point text={disclosure.independent_party} />
          <Point text={disclosure.cost_summary} />
          <Point text={disclosure.optional} />
          <Point text={guide.refresh_note} />
        </ul>
        {referral ? (
          // The FTC 16 CFR Part 255 sentence sits directly under the URL it
          // discloses, in the same note and at the panel's full text color
          // and size — never muted (ADR 0076 §5, personal-cfo-pxi.5).
          <div
            role="note"
            aria-label={`Referral disclosure for ${adapter.display_name}`}
            aria-describedby={referralId}
            className="flex flex-col gap-1.5 rounded-md border p-3"
          >
            <p>
              Referral link: <SelectableUrl url={referral.url} />
            </p>
            <p id={referralId}>{referral.disclosure}</p>
          </div>
        ) : null}
        <div className="flex flex-col gap-1.5 rounded-md border bg-muted/30 p-3">
          <p className="font-medium">To set it up</p>
          <ol className="flex list-decimal flex-col gap-1 pl-5">
            {guide.setup_steps.map((step) => (
              <li key={step}>{stepWithUrl(step, guide.provider_url)}</li>
            ))}
          </ol>
        </div>
      </CardContent>
    </Card>
  );
}
