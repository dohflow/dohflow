/* eslint-disable react-refresh/only-export-components -- an entry module: it renders itself and nothing imports it, so fast refresh has no component boundary to preserve. */
import "@fontsource-variable/ibm-plex-sans/index.css";
import "../styles/globals.css";

import React, { useRef, useState } from "react";
import ReactDOM from "react-dom/client";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogError,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ThemeProvider } from "@/theme/ThemeProvider";
import { writeStoredThemePreference } from "@/theme/themePreference";

/// Dev-only example of the shared Dialog (personal-cfo-4d8.6.1). Fake data only. Not part of the
/// production bundle: the build input is `index.html` alone. It loads the app's own IBM Plex from
/// `@fontsource-variable/ibm-plex-sans`, the declared font.
///
/// `?case=` is one of:
/// - `plain`: a short confirm.
/// - `long-name`: a 100-character unbroken name in the title and in the body.
/// - `many-lines`: 30 lines of body text (the body scrolls, the actions stay reachable).
/// - `error`: a persistent error in the alert slot.
/// - `all`: the long name, 30 lines and an error together.
/// `&theme=dark` selects Dark. The window size is the window's: shrink it to see the bounded height.

const params = new URLSearchParams(window.location.search);
const example = params.get("case") ?? "plain";
writeStoredThemePreference(params.get("theme") === "dark" ? "dark" : "light");

const LONG_NAME = "Household-budget-plan-".repeat(5).slice(0, 100);
const NAME = example === "long-name" || example === "all" ? LONG_NAME : "Maternity leave";
const LINES =
  example === "many-lines" || example === "all"
    ? Array.from({ length: 30 }, (_, i) => `You changed “Rent” item ${i + 1} to $${2400 + i}.00.`)
    : ["You changed “Rent” to $2,400.00.", "You removed “Moving costs”."];
const ERROR =
  example === "error" || example === "all"
    ? "That didn’t work, and nothing was undone. Check the list and try again."
    : null;

function Example() {
  const [open, setOpen] = useState(true);
  const cancelRef = useRef<HTMLButtonElement>(null);
  return (
    <div className="p-6">
      <p className="mb-3 text-sm text-muted-foreground">Shared dialog example, fake data.</p>
      <Button onClick={() => setOpen(true)}>Open dialog</Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent initialFocusRef={cancelRef}>
          <DialogHeader>
            <DialogTitle>Undo “{NAME}”?</DialogTitle>
            <DialogDescription>
              Undoing removes what applying added to your forecast. You changed some of it after
              applying:
            </DialogDescription>
          </DialogHeader>
          <DialogBody>
            <ul className="flex flex-col gap-2">
              {LINES.map((line) => (
                <li key={line}>{line}</li>
              ))}
              {example === "long-name" || example === "all" ? (
                <li>You removed “{LONG_NAME}”.</li>
              ) : null}
            </ul>
          </DialogBody>
          <DialogError error={ERROR} />
          <DialogFooter>
            <Button ref={cancelRef} variant="ghost" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button onClick={() => setOpen(false)}>Undo scenario</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ThemeProvider>
      <Example />
    </ThemeProvider>
  </React.StrictMode>,
);
