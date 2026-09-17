// What the SimpleFIN Bridge is, stated plainly before anyone pastes a token
// (personal-cfo-kdw6, ADR 0060 addendum 2026-09-02). The four disclosures the
// owner requires are all here: independent (unaffiliated), a separate service
// handling bank credentials, paid, and optional. Descriptive copy only
// (ADR 0018) — it explains, it never recommends. The Bridge URL is plain,
// selectable text: the app ships no opener/shell capability (ADR 0010).

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

export const BRIDGE_URL = "https://bridge.simplefin.org";

export function BridgeEducation() {
  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base">About the SimpleFIN Bridge</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-4 text-sm">
        <p>
          The SimpleFIN Bridge is an independent service that connects to your
          banks and hands this app read-only account and transaction data. Your
          bank credentials are given to the Bridge, never to this app.
        </p>
        <ul className="flex flex-col gap-2">
          <li className="flex gap-2">
            <span aria-hidden>·</span>
            <span>
              <b>Not affiliated with us.</b> If you use it, the Bridge is a
              separate service handling your bank connections under its own
              terms and privacy policy.
            </span>
          </li>
          <li className="flex gap-2">
            <span aria-hidden>·</span>
            <span>
              <b>It costs money.</b> The Bridge charges a small yearly fee,
              paid to them — nothing here is billed by this app.
            </span>
          </li>
          <li className="flex gap-2">
            <span aria-hidden>·</span>
            <span>
              <b>It is optional.</b> Everything in this app works with manual
              entry and file imports; a connection only saves the typing.
            </span>
          </li>
          <li className="flex gap-2">
            <span aria-hidden>·</span>
            <span>
              <b>Data refreshes about daily.</b> The Bridge pulls from your
              banks roughly once a day; this app refreshes it on open and on
              demand.
            </span>
          </li>
        </ul>
        <div className="flex flex-col gap-1.5 rounded-md border bg-muted/30 p-3">
          <p className="font-medium">To set it up</p>
          <ol className="flex list-decimal flex-col gap-1 pl-5">
            <li>
              Create an account at{" "}
              <code className="select-all rounded bg-muted px-1 font-mono text-xs">
                {BRIDGE_URL}
              </code>{" "}
              (copy it into your browser) and connect each bank there.
            </li>
            <li>
              Under <b>Apps</b>, choose <b>New app connection</b> to get a setup
              token.
            </li>
            <li>
              Paste the token below. Tokens are single-use; make a new one if a
              link fails.
            </li>
          </ol>
        </div>
      </CardContent>
    </Card>
  );
}
