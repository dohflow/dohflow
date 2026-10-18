// The provider picker (personal-cfo-dto2j, ADR 0076 §4): every ENABLED registry
// entry, in the registry's order (the launch provider first, then alphabetical
// — never by referral), with what it fetches, what it costs and to whom, its
// terms and when they were last reviewed, and where it works. Choosing one
// opens its disclosure panel; nothing here asks for a credential.

import type { ConnectorAdapterDto } from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { formatIsoDate } from "@/lib/format";

import { capabilityChips, costLine, countryNames } from "./providerCopy";

export function ProviderPicker({
  providers,
  headingId,
  onChoose,
}: {
  providers: ConnectorAdapterDto[];
  headingId: string;
  onChoose: (adapterId: string) => void;
}) {
  return (
    <section aria-labelledby={headingId} className="flex flex-col gap-3">
      <h3 id={headingId} tabIndex={-1} className="text-sm font-medium outline-none">
        Choose a provider
      </h3>
      <ul className="flex flex-col gap-2">
        {providers.map((provider) => {
          const descriptionId = `provider-${provider.adapter_id}-details`;
          return (
            <li
              key={provider.adapter_id}
              className="flex flex-col gap-2 rounded-md border p-3 text-sm"
            >
              <div className="flex items-center justify-between gap-3">
                <p className="font-medium">{provider.display_name}</p>
                <Button
                  size="sm"
                  variant="outline"
                  aria-describedby={descriptionId}
                  onClick={() => onChoose(provider.adapter_id)}
                >
                  Choose {provider.display_name}
                </Button>
              </div>
              <div id={descriptionId} className="flex flex-col gap-1.5">
                <ul aria-label="What it fetches" className="flex flex-wrap gap-1">
                  {capabilityChips(provider).map((chip) => (
                    <li key={chip}>
                      <Badge variant="outline">{chip}</Badge>
                    </li>
                  ))}
                </ul>
                <p>{costLine(provider)}</p>
                <p className="text-xs text-muted-foreground">
                  Terms reviewed{" "}
                  {formatIsoDate(provider.economics.terms_reviewed_at)}
                  {provider.economics.terms_url ? (
                    <>
                      {" — "}
                      <code className="select-all rounded bg-muted px-1 font-mono">
                        {provider.economics.terms_url}
                      </code>
                    </>
                  ) : null}
                </p>
                <p className="text-xs text-muted-foreground">
                  Available in: {countryNames(provider)}
                </p>
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
