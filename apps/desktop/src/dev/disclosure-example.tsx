/* eslint-disable react-refresh/only-export-components -- an entry module: it renders itself and nothing imports it, so fast refresh has no component boundary to preserve. */
import "@fontsource-variable/ibm-plex-sans/index.css";
import "../styles/globals.css";

import React, { useRef, useState } from "react";
import ReactDOM from "react-dom/client";

import { Button } from "@/components/ui/button";
import { Disclosure, usePinDisclosures } from "@/components/ui/disclosure";
import { Input } from "@/components/ui/input";

/// Dev-only reusable example for the shared disclosure (personal-cfo-4d8.38). Fake data only.
/// Not part of the production bundle: the build input is `index.html` alone.
/// `?theme=dark` renders the dark theme; `?open=0` starts every section collapsed.

const params = new URLSearchParams(window.location.search);
document.documentElement.classList.toggle("dark", params.get("theme") === "dark");
const startOpen = params.get("open") !== "0";

function DeleteConfirm() {
  const [confirming, setConfirming] = useState(false);
  const [typed, setTyped] = useState("");
  const startRef = useRef<HTMLButtonElement>(null);
  usePinDisclosures(confirming);
  return (
    <div className="flex flex-col gap-3">
      <p className="text-sm text-muted-foreground">
        Deleting the vault removes every account, bill and forecast in it. This cannot be undone.
      </p>
      {confirming ? (
        <div role="group" aria-label="Confirm delete" className="flex flex-col gap-2">
          <Input
            aria-label="Type the vault name to confirm"
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
          />
          <div className="flex gap-2">
            <Button variant="destructive" disabled={typed !== "Example vault"}>
              Delete this vault
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                setConfirming(false);
                setTyped("");
                startRef.current?.focus();
              }}
            >
              Cancel
            </Button>
          </div>
        </div>
      ) : (
        <Button
          ref={startRef}
          variant="destructive"
          className="self-start"
          onClick={() => setConfirming(true)}
        >
          Delete vault…
        </Button>
      )}
    </div>
  );
}

function Example() {
  return (
    <main className="mx-auto flex max-w-xl flex-col gap-4 p-6">
      <Disclosure
        title="A deliberately long section title that has to wrap onto a second line without clipping or truncating anything"
        defaultOpen={startOpen}
        summary="Daily · last verified Tuesday"
        persistent={
          <div className="flex flex-col gap-2 rounded-md border border-loss/30 bg-loss/10 p-3 text-sm">
            <p role="alert">
              Could not refresh Example Bank. Your balances are from Monday. Check the connection
              and try again.
            </p>
            <Button size="sm" variant="outline" className="self-start">
              Refresh now
            </Button>
          </div>
        }
      >
        <div className="flex flex-col gap-2">
          <label htmlFor="ex-name" className="text-sm font-medium">
            Household name
          </label>
          <Input id="ex-name" defaultValue="The Example household" />
        </div>
      </Disclosure>

      <Disclosure title="Backups" summary="Daily · last verified Tuesday" defaultOpen={false}>
        <p className="text-sm text-muted-foreground">Back up now, schedule and history.</p>
      </Disclosure>

      <Disclosure title="Password, encryption and deletion" defaultOpen={startOpen}>
        <Disclosure title="Delete this vault" headingLevel={3} defaultOpen={startOpen}>
          <DeleteConfirm />
        </Disclosure>
      </Disclosure>
    </main>
  );
}

const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("Root element #root not found");
ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <Example />
  </React.StrictMode>,
);
